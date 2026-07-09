//! Local-filesystem [`Source`] backend.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use memmap2::Mmap;

use crate::error::SourceError;
use crate::meta::{EntryKind, EntryMeta};
use crate::path::RelPath;
use crate::{Capabilities, ContentReader, Source, SourceMut};

/// A [`Source`] backed by a directory on the local filesystem.
#[derive(Clone, Debug)]
pub struct LocalSource {
    root: PathBuf,
}

impl LocalSource {
    /// Create a source rooted at `root` (the directory whose tree will be compared).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        LocalSource { root: root.into() }
    }

    /// The filesystem root this source is anchored at.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }
}

impl Source for LocalSource {
    fn capabilities(&self) -> Capabilities {
        // List + read + write; no cheap fingerprint (we read bytes to compare content).
        Capabilities::FS_RW
    }

    fn read_dir(&self, rel: &RelPath) -> Result<Vec<EntryMeta>, SourceError> {
        let dir = rel.to_path(&self.root);
        let mut entries = Vec::new();
        let read = std::fs::read_dir(&dir).map_err(|e| SourceError::io(rel.to_string(), e))?;
        for entry in read {
            let entry = entry.map_err(|e| SourceError::io(rel.to_string(), e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // `file_type` and `metadata` on a DirEntry do NOT follow symlinks — we report the link itself.
            let file_type = entry
                .file_type()
                .map_err(|e| SourceError::io(rel.child(&name).to_string(), e))?;
            let kind = if file_type.is_symlink() {
                EntryKind::Symlink
            } else if file_type.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            let md = entry
                .metadata()
                .map_err(|e| SourceError::io(rel.child(&name).to_string(), e))?;
            entries.push(EntryMeta {
                rel_path: rel.child(&name),
                kind,
                size: if kind.is_dir() { 0 } else { md.len() },
                mtime: md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64),
                created: md
                    .created()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64),
                name,
            });
        }
        Ok(entries)
    }

    fn open(&self, rel: &RelPath) -> Result<Box<dyn ContentReader>, SourceError> {
        let path = rel.to_path(&self.root);
        let file = File::open(&path).map_err(|e| SourceError::io(rel.to_string(), e))?;
        let len = file
            .metadata()
            .map_err(|e| SourceError::io(rel.to_string(), e))?
            .len();
        if len == 0 {
            // memmap cannot map a zero-length file; represent it as an empty reader.
            return Ok(Box::new(LocalContentReader::Empty));
        }
        // SAFETY: standard memory-mapping caveat — undefined behavior if the file is mutated externally
        // while mapped. Acceptable for a read-only compare tool; documented as a known limitation.
        let mmap = unsafe { Mmap::map(&file) }.map_err(|e| SourceError::io(rel.to_string(), e))?;
        Ok(Box::new(LocalContentReader::Mapped(mmap)))
    }
}

impl SourceMut for LocalSource {
    fn write_file(&self, rel: &RelPath, data: &dyn ContentReader) -> Result<(), SourceError> {
        let path = rel.to_path(&self.root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SourceError::io(rel.to_string(), e))?;
        }
        let mut file = File::create(&path).map_err(|e| SourceError::io(rel.to_string(), e))?;
        if let Some(slice) = data.as_slice() {
            file.write_all(slice)
                .map_err(|e| SourceError::io(rel.to_string(), e))?;
        } else {
            let len = data.len();
            let mut offset = 0u64;
            let mut buf = vec![0u8; 64 * 1024];
            while offset < len {
                let n = data.read_at(offset, &mut buf)?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n])
                    .map_err(|e| SourceError::io(rel.to_string(), e))?;
                offset += n as u64;
            }
        }
        Ok(())
    }

    fn rename(&self, from: &RelPath, to: &RelPath) -> Result<(), SourceError> {
        // `std::fs::rename` atomically replaces the destination on the same filesystem (POSIX rename(2),
        // Windows MoveFileEx with REPLACE_EXISTING). The temp sibling is always in the dest's directory.
        std::fs::rename(from.to_path(&self.root), to.to_path(&self.root))
            .map_err(|e| SourceError::io(to.to_string(), e))
    }

    fn supports_atomic_replace(&self) -> bool {
        true
    }

    fn create_dir_all(&self, rel: &RelPath) -> Result<(), SourceError> {
        std::fs::create_dir_all(rel.to_path(&self.root))
            .map_err(|e| SourceError::io(rel.to_string(), e))
    }

    fn remove(&self, rel: &RelPath) -> Result<(), SourceError> {
        let path = rel.to_path(&self.root);
        let md =
            std::fs::symlink_metadata(&path).map_err(|e| SourceError::io(rel.to_string(), e))?;
        let result = if md.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        result.map_err(|e| SourceError::io(rel.to_string(), e))
    }
}

/// Content reader for local files: memory-mapped, or an empty reader for zero-length files.
enum LocalContentReader {
    Empty,
    Mapped(Mmap),
}

impl ContentReader for LocalContentReader {
    fn len(&self) -> u64 {
        match self {
            LocalContentReader::Empty => 0,
            LocalContentReader::Mapped(mmap) => mmap.len() as u64,
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError> {
        let data = self
            .as_slice()
            .expect("local reader always exposes a slice");
        let start = offset.min(data.len() as u64) as usize;
        let n = buf.len().min(data.len() - start);
        buf[..n].copy_from_slice(&data[start..start + n]);
        Ok(n)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(match self {
            LocalContentReader::Empty => &[],
            LocalContentReader::Mapped(mmap) => &mmap[..],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) {
        let mut f = File::create(dir.join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }

    #[test]
    fn local_source_is_read_write() {
        let src = LocalSource::new(".");
        assert_eq!(src.capabilities(), Capabilities::FS_RW);
        assert!(src.capabilities().allows_tree_compare());
        assert!(src.capabilities().allows_content_compare());
    }

    #[test]
    fn read_dir_reports_names_kinds_sizes() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "a.txt", b"hello");
        std::fs::create_dir(tmp.path().join("sub")).unwrap();

        let src = LocalSource::new(tmp.path());
        let mut entries = src.read_dir(&RelPath::root()).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[0].kind, EntryKind::File);
        assert_eq!(entries[0].size, 5);
        assert_eq!(entries[0].rel_path, RelPath::root().child("a.txt"));
        assert_eq!(entries[1].name, "sub");
        assert_eq!(entries[1].kind, EntryKind::Dir);
    }

    #[test]
    fn open_exposes_contents_via_slice_and_read_at() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "f.bin", b"0123456789");
        let src = LocalSource::new(tmp.path());

        let reader = src.open(&RelPath::root().child("f.bin")).unwrap();
        assert_eq!(reader.len(), 10);
        assert_eq!(reader.as_slice().unwrap(), b"0123456789");

        let mut buf = [0u8; 4];
        let n = reader.read_at(3, &mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf, b"3456");

        // Reading past EOF yields 0 bytes.
        assert_eq!(reader.read_at(100, &mut buf).unwrap(), 0);
    }

    #[test]
    fn empty_file_opens_as_empty_reader() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "empty", b"");
        let src = LocalSource::new(tmp.path());

        let reader = src.open(&RelPath::root().child("empty")).unwrap();
        assert_eq!(reader.len(), 0);
        assert_eq!(reader.as_slice().unwrap(), b"");
    }

    #[test]
    fn read_dir_on_missing_path_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let src = LocalSource::new(tmp.path());
        let err = src.read_dir(&RelPath::root().child("nope")).unwrap_err();
        assert!(matches!(err, SourceError::Io { .. }));
    }

    #[test]
    fn source_mut_copy_create_remove() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();
        write(src_dir.path(), "f.txt", b"payload");
        let src = LocalSource::new(src_dir.path());
        let dst = LocalSource::new(dst_dir.path());

        // copy_from: read from src, write into a nested path on dst (parents created).
        let reader = src.open(&RelPath::root().child("f.txt")).unwrap();
        let target = RelPath::root().child("sub").child("f.txt");
        dst.copy_from(&target, reader.as_ref()).unwrap();
        let copied = dst.open(&target).unwrap();
        assert_eq!(copied.as_slice().unwrap(), b"payload");

        // create_dir_all
        let deep = RelPath::root().child("a").child("b");
        dst.create_dir_all(&deep).unwrap();
        assert!(deep.to_path(dst_dir.path()).is_dir());

        // remove a file
        dst.remove(&target).unwrap();
        assert!(dst.open(&target).is_err());

        // remove a directory recursively
        dst.remove(&RelPath::root().child("a")).unwrap();
        assert!(!RelPath::root().child("a").to_path(dst_dir.path()).exists());
    }

    /// A streaming reader (no `as_slice`) that yields `'X'` bytes up to `fail_after`, then errors —
    /// to simulate a network/disk failure part-way through a copy.
    struct FailingReader {
        total: u64,
        fail_after: u64,
    }
    impl ContentReader for FailingReader {
        fn len(&self) -> u64 {
            self.total
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError> {
            if offset >= self.fail_after {
                return Err(SourceError::Other("simulated mid-write failure".to_owned()));
            }
            let n = (self.fail_after - offset).min(buf.len() as u64) as usize;
            buf[..n].fill(b'X');
            Ok(n)
        }
        // No zero-copy slice → forces the streaming write path (the one that can fail mid-stream).
        fn as_slice(&self) -> Option<&[u8]> {
            None
        }
    }

    #[test]
    fn copy_from_failure_leaves_destination_intact_and_no_temp() {
        let dst_dir = tempfile::tempdir().unwrap();
        let dst = LocalSource::new(dst_dir.path());
        let target = RelPath::root().child("data.bin");

        // Seed an existing destination file we must not corrupt.
        write(dst_dir.path(), "data.bin", b"ORIGINAL");

        // A copy that fails after 50 bytes must NOT touch the destination (temp+rename: the partial
        // write went to a temp sibling, which is cleaned up on failure).
        let bad = FailingReader {
            total: 200,
            fail_after: 50,
        };
        let err = dst.copy_from(&target, &bad).unwrap_err();
        assert!(matches!(err, SourceError::Other(_)));

        // Destination still holds the original bytes — never truncated.
        let kept = dst.open(&target).unwrap();
        assert_eq!(kept.as_slice().unwrap(), b"ORIGINAL");

        // No leftover temp files in the directory.
        let leftover: Vec<_> = std::fs::read_dir(dst_dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("confold-tmp"))
            .collect();
        assert!(leftover.is_empty(), "temp file left behind: {leftover:?}");
    }

    #[test]
    fn copy_from_success_leaves_no_temp() {
        let dst_dir = tempfile::tempdir().unwrap();
        let dst = LocalSource::new(dst_dir.path());
        let src_dir = tempfile::tempdir().unwrap();
        write(src_dir.path(), "f.txt", b"hello");
        let src = LocalSource::new(src_dir.path());

        let reader = src.open(&RelPath::root().child("f.txt")).unwrap();
        let target = RelPath::root().child("f.txt");
        dst.copy_from(&target, reader.as_ref()).unwrap();

        assert_eq!(dst.open(&target).unwrap().as_slice().unwrap(), b"hello");
        let entries: Vec<_> = std::fs::read_dir(dst_dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["f.txt".to_owned()]);
    }
}
