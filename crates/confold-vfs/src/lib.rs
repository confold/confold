//! Virtual file-system (VFS) abstraction for the Confold compare engine.
//!
//! The engine never touches `std::fs` directly — it compares trees through the [`Source`] trait, so
//! local, SMB, S3, … backends are interchangeable. Ships [`LocalSource`] here; an SFTP backend lives in
//! `confold-sftp`.
//!
//! The trait is **synchronous**: local I/O is blocking and the engine parallelizes with rayon. Network
//! backends (e.g. SFTP) encapsulate their own async runtime and drive it to completion synchronously
//! behind this trait, so the engine stays blocking + rayon-parallel and never sees `async`.

mod capabilities;
mod error;
mod local;
mod meta;
mod path;

pub use capabilities::Capabilities;
pub use error::SourceError;
pub use local::LocalSource;
pub use meta::{EntryKind, EntryMeta, Fingerprint};
pub use path::RelPath;

/// A comparable source of a directory tree.
///
/// Implemented by [`LocalSource`] today; SMB/S3/… later. `Send + Sync` so the engine can read from it
/// across rayon worker threads.
pub trait Source: Send + Sync {
    /// What this source can do — gates which operations the engine and UI offer. See [`Capabilities`].
    fn capabilities(&self) -> Capabilities;

    /// List the immediate children of the directory at `rel` (metadata only, no content read).
    fn read_dir(&self, rel: &RelPath) -> Result<Vec<EntryMeta>, SourceError>;

    /// Open the file at `rel` for content comparison.
    fn open(&self, rel: &RelPath) -> Result<Box<dyn ContentReader>, SourceError>;

    /// A cheap content fingerprint, if the backend exposes one (e.g. an S3 ETag), to let the engine
    /// skip reading bytes. Returns `None` by default (and for [`LocalSource`]).
    fn quick_fingerprint(&self, _rel: &RelPath) -> Result<Option<Fingerprint>, SourceError> {
        Ok(None)
    }
}

/// Positioned, thread-friendly reader over a file's contents.
///
/// [`read_at`](ContentReader::read_at) maps cleanly onto both memory-mapped local files and ranged
/// network reads. [`as_slice`](ContentReader::as_slice) offers a zero-copy fast path when the backend
/// has the whole content in memory (local mmap).
///
/// **Lifetime contract.** A reader may be *self-contained* (owns its bytes — e.g. an in-memory buffer)
/// or *streaming* (fetches windows lazily from its [`Source`] — e.g. ranged SFTP reads). A streaming
/// reader is only valid **while its originating [`Source`] is alive**; callers must keep the source
/// alive for the reader's lifetime (the compare engine and sync engine do, holding the source across
/// the whole operation). Streaming readers also assume `read_at` is called from a synchronous thread
/// (never from inside an async runtime).
pub trait ContentReader: Send {
    /// Total content length in bytes.
    fn len(&self) -> u64;

    /// `true` if the content is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read into `buf` starting at `offset`, **filling it completely** unless EOF is reached first.
    /// Returns the number of bytes read: `buf.len()` for an interior read, fewer (possibly 0) only when
    /// `offset + buf.len()` reaches past the end. A short read therefore *always* means EOF — never a
    /// mid-file chunk boundary. Content comparison relies on this (it treats `na != nb` as inequality),
    /// so streaming backends must loop their underlying reads to fill the buffer.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError>;

    /// Zero-copy access to the whole content when cheaply available (memory-mapped files). `None`
    /// otherwise, in which case callers must use [`read_at`](ContentReader::read_at).
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }
}

/// A [`Source`] that also supports mutating operations, for synchronization (Phase 2+).
///
/// Backends opt in by implementing this in addition to [`Source`], so a read-only backend never gains
/// write capability by accident. Sync actions go through this trait, so SMB/S3 inherit them later.
///
/// **Crash-safety of writes.** Callers use [`copy_from`](SourceMut::copy_from), which is a *provided*
/// method that — on backends reporting [`supports_atomic_replace`](SourceMut::supports_atomic_replace) —
/// writes to a unique temp sibling and renames it into place, so a mid-write failure (network drop, disk
/// full) never leaves a truncated file at the destination. Backends only implement the primitives
/// ([`write_file`](SourceMut::write_file), [`rename`](SourceMut::rename), …); the atomicity logic lives
/// here, once.
pub trait SourceMut: Source {
    /// Write `data` to the file at `rel`, creating parent directories. Overwrites an existing file.
    /// **Raw, non-atomic primitive** — a mid-write failure may leave a partial file. Callers should use
    /// [`copy_from`](SourceMut::copy_from) instead, which wraps this with temp+rename where possible.
    fn write_file(&self, rel: &RelPath, data: &dyn ContentReader) -> Result<(), SourceError>;

    /// Rename/move `from` to `to` (same backend), replacing `to` if the backend's rename can. Used by
    /// [`copy_from`](SourceMut::copy_from) to swap a fully-written temp into place atomically.
    fn rename(&self, from: &RelPath, to: &RelPath) -> Result<(), SourceError>;

    /// Whether [`rename`](SourceMut::rename) can atomically replace the destination on this backend — so
    /// [`copy_from`](SourceMut::copy_from) can use the temp+rename strategy. A local filesystem returns
    /// `true`; a network backend may probe its server once and cache the result.
    fn supports_atomic_replace(&self) -> bool;

    /// Create the directory at `rel` (and any missing parents).
    fn create_dir_all(&self, rel: &RelPath) -> Result<(), SourceError>;

    /// Remove the file, or recursively remove the directory, at `rel`.
    fn remove(&self, rel: &RelPath) -> Result<(), SourceError>;

    /// Write `data` to the file at `rel`, creating parent directories and overwriting any existing file.
    ///
    /// Crash-safe where the backend [`supports_atomic_replace`](SourceMut::supports_atomic_replace):
    /// the bytes are streamed to a unique temp sibling first, then renamed onto `rel`, so a failure
    /// part-way through never leaves a truncated file in place (the temp is cleaned up best-effort).
    /// Backends without atomic rename fall back to a direct write, removing the partial file on error
    /// (which cannot be made fully atomic — an overwrite still loses the prior content on failure).
    fn copy_from(&self, rel: &RelPath, data: &dyn ContentReader) -> Result<(), SourceError> {
        if self.supports_atomic_replace() {
            let tmp = temp_sibling(rel);
            if let Err(e) = self.write_file(&tmp, data) {
                let _ = self.remove(&tmp); // best-effort: don't leave the partial temp behind
                return Err(e);
            }
            if let Err(e) = self.rename(&tmp, rel) {
                let _ = self.remove(&tmp);
                return Err(e);
            }
            Ok(())
        } else {
            // No atomic replace available: write directly. On failure, remove the partial so we don't
            // leave a silently-truncated file. (Genuinely atomic replacement is impossible without rename.)
            match self.write_file(rel, data) {
                Ok(()) => Ok(()),
                Err(e) => {
                    let _ = self.remove(rel);
                    Err(e)
                }
            }
        }
    }
}

/// A process-unique temp path in the same directory as `rel` (so a rename stays on the same filesystem /
/// remote directory). The `pid` + monotonic counter make it collision-free even across concurrent writes
/// to the same destination, within and across processes.
fn temp_sibling(rel: &RelPath) -> RelPath {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!(
        ".confold-tmp.{}.{}.{}",
        std::process::id(),
        n,
        rel.file_name().unwrap_or("file"),
    );
    match rel.parent() {
        Some(parent) => parent.child(&name),
        None => RelPath::root().child(&name),
    }
}

#[cfg(test)]
mod copy_from_tests {
    use super::*;
    use std::sync::Mutex;

    /// A `SourceMut` that records which primitives `copy_from` drives, with configurable atomic-replace
    /// support and an optional `write_file` failure — to exercise the provided `copy_from` orchestration
    /// deterministically (no real filesystem). The `Source` half is stubbed; `copy_from` never calls it.
    struct Mock {
        atomic: bool,
        fail_write: bool,
        written: Mutex<Vec<String>>,
        renamed: Mutex<Vec<(String, String)>>,
        removed: Mutex<Vec<String>>,
    }
    impl Mock {
        fn new(atomic: bool, fail_write: bool) -> Self {
            Mock {
                atomic,
                fail_write,
                written: Mutex::new(Vec::new()),
                renamed: Mutex::new(Vec::new()),
                removed: Mutex::new(Vec::new()),
            }
        }
    }
    impl Source for Mock {
        fn capabilities(&self) -> Capabilities {
            Capabilities::FS_RW
        }
        fn read_dir(&self, _: &RelPath) -> Result<Vec<EntryMeta>, SourceError> {
            unimplemented!("copy_from never lists")
        }
        fn open(&self, _: &RelPath) -> Result<Box<dyn ContentReader>, SourceError> {
            unimplemented!("copy_from never reads the destination")
        }
    }
    impl SourceMut for Mock {
        fn write_file(&self, rel: &RelPath, _data: &dyn ContentReader) -> Result<(), SourceError> {
            self.written.lock().unwrap().push(rel.to_string());
            if self.fail_write {
                Err(SourceError::Other("simulated write failure".to_owned()))
            } else {
                Ok(())
            }
        }
        fn rename(&self, from: &RelPath, to: &RelPath) -> Result<(), SourceError> {
            self.renamed
                .lock()
                .unwrap()
                .push((from.to_string(), to.to_string()));
            Ok(())
        }
        fn supports_atomic_replace(&self) -> bool {
            self.atomic
        }
        fn create_dir_all(&self, _: &RelPath) -> Result<(), SourceError> {
            Ok(())
        }
        fn remove(&self, rel: &RelPath) -> Result<(), SourceError> {
            self.removed.lock().unwrap().push(rel.to_string());
            Ok(())
        }
    }

    struct Empty;
    impl ContentReader for Empty {
        fn len(&self) -> u64 {
            0
        }
        fn read_at(&self, _: u64, _: &mut [u8]) -> Result<usize, SourceError> {
            Ok(0)
        }
    }

    fn rel() -> RelPath {
        RelPath::root().child("dir").child("file.txt")
    }

    #[test]
    fn atomic_success_writes_temp_then_renames_onto_target() {
        let m = Mock::new(true, false);
        m.copy_from(&rel(), &Empty).unwrap();
        // Wrote to a temp sibling (not the final path), then renamed temp → final. No cleanup.
        let written = m.written.lock().unwrap();
        assert_eq!(written.len(), 1);
        assert!(
            written[0].contains("confold-tmp"),
            "wrote to {}",
            written[0]
        );
        let renamed = m.renamed.lock().unwrap();
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0].0, written[0]); // from == the temp we wrote
        assert_eq!(renamed[0].1, "dir/file.txt"); // to == the final target
        assert!(m.removed.lock().unwrap().is_empty());
    }

    #[test]
    fn atomic_failure_cleans_up_temp_and_never_touches_target() {
        let m = Mock::new(true, true);
        let err = m.copy_from(&rel(), &Empty).unwrap_err();
        assert!(matches!(err, SourceError::Other(_)));
        // The partial temp is removed; no rename happened; the final target was never written.
        let written = m.written.lock().unwrap();
        assert!(written[0].contains("confold-tmp"));
        assert!(m.renamed.lock().unwrap().is_empty());
        assert_eq!(*m.removed.lock().unwrap(), vec![written[0].clone()]);
    }

    #[test]
    fn fallback_success_writes_directly_no_temp_no_remove() {
        let m = Mock::new(false, false);
        m.copy_from(&rel(), &Empty).unwrap();
        assert_eq!(*m.written.lock().unwrap(), vec!["dir/file.txt".to_owned()]);
        assert!(m.renamed.lock().unwrap().is_empty());
        assert!(m.removed.lock().unwrap().is_empty());
    }

    #[test]
    fn fallback_failure_removes_the_partial_file() {
        let m = Mock::new(false, true);
        let err = m.copy_from(&rel(), &Empty).unwrap_err();
        assert!(matches!(err, SourceError::Other(_)));
        // Wrote directly to the target, and on failure removed the partial.
        assert_eq!(*m.written.lock().unwrap(), vec!["dir/file.txt".to_owned()]);
        assert_eq!(*m.removed.lock().unwrap(), vec!["dir/file.txt".to_owned()]);
    }
}
