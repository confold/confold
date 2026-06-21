//! SFTP [`Source`] backend for Confold, built on the pure-Rust `russh` stack (no C/OpenSSL —
//! portable across Linux/macOS/Windows).
//!
//! The VFS traits are **synchronous**, but `russh`/`russh-sftp` are **async**. Per the project's
//! design decision, the async runtime is *encapsulated inside this backend*: [`SftpSource`] owns a
//! dedicated multi-threaded tokio runtime and calls `runtime.block_on(...)` inside each sync trait
//! method. The compare engine stays fully synchronous and never sees a `Future`.
//!
//! A multi-thread runtime (rather than `current_thread`) is used so that calling a sync trait method
//! from a rayon worker thread — `block_on` parks *that* thread — does not starve the I/O driver
//! that must keep pumping the SSH connection; the runtime's own worker threads carry it.
//!
//! # Path model
//!
//! [`RelPath`] components are joined under a configured [`SftpConfig::root`] on the server to form
//! the absolute POSIX path sent over the wire. `RelPath::root()` maps to `root` itself.
//!
//! # Known limitations / TODOs
//!
//! - **Host-key verification**: the connection currently accepts *any* server key (trust-on-first-
//!   use is not even enforced). This is a deliberate stopgap; a `known_hosts` policy is future work.
//! - **Read-ahead**: [`SftpSource::open`] returns a streaming [`ContentReader`] that fetches each
//!   window over the wire on demand (no whole-file download), so memory stays bounded regardless of
//!   file size. Adaptive read-ahead pipelining (a larger dynamic window) is future work.

use std::sync::{Arc, Mutex, OnceLock};

use russh::client::{self, Handle};
use russh::keys::{decode_secret_key, ssh_key, HashAlg, PrivateKeyWithHashAlg};
use russh_sftp::client::fs::File;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileType, OpenFlags, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::runtime::Runtime;

use confold_vfs::{
    Capabilities, ContentReader, EntryKind, EntryMeta, RelPath, Source, SourceError, SourceMut,
};

/// How to authenticate to the SFTP server.
#[derive(Clone)]
pub enum SftpAuth {
    /// Password authentication.
    Password(String),
    /// Public-key authentication from a PEM-encoded OpenSSH private key.
    PrivateKey {
        /// The PEM-encoded private key (OpenSSH or PKCS#8).
        pem: String,
        /// Optional passphrase protecting the key.
        passphrase: Option<String>,
    },
}

impl std::fmt::Debug for SftpAuth {
    /// Never leak the secret material in logs / debug output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SftpAuth::Password(_) => f.write_str("Password(<redacted>)"),
            SftpAuth::PrivateKey { .. } => f.write_str("PrivateKey { <redacted> }"),
        }
    }
}

/// Connection parameters for an [`SftpSource`].
#[derive(Clone, Debug)]
pub struct SftpConfig {
    /// Server host name or IP.
    pub host: String,
    /// Server port (SFTP-over-SSH default is 22).
    pub port: u16,
    /// User to authenticate as.
    pub username: String,
    /// Credentials. See [`SftpAuth`].
    pub auth: SftpAuth,
    /// Base directory on the server that `RelPath::root()` maps to (e.g. `"/"` or `"/home/u/data"`).
    pub root: String,
    // Host-key policy is accept-any for now; see the crate-level "Known limitations" note.
}

impl SftpConfig {
    /// Convenience constructor with the default SSH port (22) and root `"/"`.
    pub fn new(host: impl Into<String>, username: impl Into<String>, auth: SftpAuth) -> Self {
        SftpConfig {
            host: host.into(),
            port: 22,
            username: username.into(),
            auth,
            root: "/".to_owned(),
        }
    }
}

/// A [`Source`] backed by a directory tree on a remote SFTP server.
///
/// Construct with [`SftpSource::connect`], which eagerly establishes the SSH connection,
/// authenticates, and opens the SFTP subsystem. The session and its owning runtime live for as long
/// as the `SftpSource`.
pub struct SftpSource {
    /// Owns the async machinery. Declared *before* the connection fields so it is dropped last
    /// (Rust drops struct fields in declaration order), keeping the runtime alive while the session
    /// shuts down.
    runtime: Runtime,
    /// The SFTP session. `russh-sftp`'s `SftpSession` is not `Sync`, so we guard it with a `Mutex`
    /// to make `SftpSource: Sync` (required by `Source`). Operations are serialized — acceptable for
    /// a compare/sync tool that reads the tree breadth-first.
    sftp: Arc<Mutex<SftpSession>>,
    /// Kept alive for the lifetime of the session: dropping the SSH handle tears down the transport.
    _ssh: Handle<ClientHandler>,
    /// Base directory on the server; `RelPath` components are appended to this.
    root: String,
    /// Cached result of the one-time "does this server's rename atomically replace?" probe, used by
    /// [`supports_atomic_replace`](SourceMut::supports_atomic_replace). `None` until first probed.
    atomic_replace: OnceLock<bool>,
}

impl SftpSource {
    /// Connect to the SFTP server described by `config`, authenticate, and open the SFTP subsystem.
    ///
    /// Blocking: builds the runtime and drives the async connect to completion before returning.
    pub fn connect(config: SftpConfig) -> Result<SftpSource, SourceError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| SourceError::io("<sftp connect>", e))?;

        let (ssh, sftp) = runtime.block_on(async { Self::establish(&config).await })?;

        Ok(SftpSource {
            runtime,
            sftp: Arc::new(Mutex::new(sftp)),
            _ssh: ssh,
            root: normalize_root(&config.root),
            atomic_replace: OnceLock::new(),
        })
    }

    /// Async connect/auth/subsystem handshake, run on the owned runtime.
    async fn establish(
        config: &SftpConfig,
    ) -> Result<(Handle<ClientHandler>, SftpSession), SourceError> {
        let ssh_config = Arc::new(client::Config::default());
        let mut handle = client::connect(
            ssh_config,
            (config.host.as_str(), config.port),
            ClientHandler,
        )
        .await
        .map_err(map_ssh)?;

        let authenticated = match &config.auth {
            SftpAuth::Password(password) => handle
                .authenticate_password(&config.username, password)
                .await
                .map_err(map_ssh)?,
            SftpAuth::PrivateKey { pem, passphrase } => {
                let key = decode_secret_key(pem, passphrase.as_deref())
                    .map_err(|e| SourceError::Other(format!("invalid private key: {e}")))?;
                handle
                    .authenticate_publickey(
                        &config.username,
                        PrivateKeyWithHashAlg::new(Arc::new(key), Some(HashAlg::Sha256)),
                    )
                    .await
                    .map_err(map_ssh)?
            }
        };
        if !authenticated.success() {
            return Err(SourceError::Other("SFTP authentication failed".to_owned()));
        }

        let channel = handle.channel_open_session().await.map_err(map_ssh)?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(map_ssh)?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SourceError::Other(format!("opening SFTP subsystem: {e}")))?;

        Ok((handle, sftp))
    }

    /// Resolve a [`RelPath`] to the absolute POSIX path on the server.
    fn remote_path(&self, rel: &RelPath) -> String {
        join_remote(&self.root, rel)
    }

    /// Run an async closure with the locked SFTP session on the owned runtime.
    ///
    /// Centralizes `block_on` + `Mutex` handling so every trait method stays a one-liner. The
    /// closure receives the live session and returns a boxed future borrowing it (the `for<'a>`
    /// bound is what lets the returned future borrow the `&SftpSession`); the lock is held across
    /// the whole future, which serializes SFTP operations (see [`SftpSource::sftp`]). `block_on`
    /// drives the future on the current thread, so holding the (non-`Send`) guard across `.await`
    /// is sound.
    fn with_sftp<T, F>(&self, f: F) -> Result<T, SourceError>
    where
        F: for<'a> FnOnce(
            &'a SftpSession,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, SourceError>> + 'a>,
        >,
    {
        let guard = self
            .sftp
            .lock()
            .map_err(|_| SourceError::Other("SFTP session lock poisoned".to_owned()))?;
        self.runtime.block_on(f(&guard))
    }

    /// One-time probe (cached in [`SftpSource::atomic_replace`]): does this server support `rename` at
    /// all? Creates a temp file and renames it to a fresh name; success ⇒ the temp+rename copy strategy
    /// is usable, otherwise [`copy_from`](SourceMut::copy_from) falls back to direct writes. Some
    /// backends allow create but not rename — this is the "can it move?" check. Temps are cleaned up.
    fn probe_rename_support(&self) -> bool {
        let from = RelPath::root().child(&probe_name("a"));
        let to = RelPath::root().child(&probe_name("b"));
        let ok = self.write_file(&from, &ProbeReader).is_ok() && {
            // Pure rename to a non-existent target — no remove+retry dance, so this tests rename itself.
            let from_path = self.remote_path(&from);
            let to_path = self.remote_path(&to);
            let rel_to = to.clone();
            self.with_sftp(move |sftp| {
                Box::pin(async move {
                    sftp.rename(from_path, to_path)
                        .await
                        .map_err(|e| map_sftp(&rel_to, e))
                })
            })
            .is_ok()
        };
        let _ = self.remove(&from); // best-effort cleanup (one of these no longer exists post-rename)
        let _ = self.remove(&to);
        ok
    }
}

impl Source for SftpSource {
    fn capabilities(&self) -> Capabilities {
        // List + read + write; no cheap fingerprint (we read bytes to compare content).
        Capabilities::FS_RW
    }

    fn read_dir(&self, rel: &RelPath) -> Result<Vec<EntryMeta>, SourceError> {
        let dir = self.remote_path(rel);
        // Own `rel` so the future captures no caller borrow (required by `with_sftp`'s HRTB bound).
        let rel = rel.clone();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let read = sftp.read_dir(&dir).await.map_err(|e| map_sftp(&rel, e))?;
                let mut entries = Vec::new();
                for entry in read {
                    let name = entry.file_name();
                    // SFTP `read_dir` reports the entry's own (lstat-style) type, so symlinks surface
                    // as `Symlink` rather than their target — matching `LocalSource`.
                    let kind = match entry.file_type() {
                        FileType::Dir => EntryKind::Dir,
                        FileType::Symlink => EntryKind::Symlink,
                        // Files and anything exotic (socket/fifo/…) are treated as files.
                        _ => EntryKind::File,
                    };
                    let attrs = entry.metadata();
                    entries.push(EntryMeta {
                        rel_path: rel.child(&name),
                        kind,
                        size: if kind.is_dir() {
                            0
                        } else {
                            attrs.size.unwrap_or(0)
                        },
                        // SFTP mtime is Unix epoch *seconds*; the VFS contract is milliseconds.
                        mtime: attrs.mtime.map(|s| s as i64 * 1000),
                        // SFTP (v3) exposes no creation time.
                        created: None,
                        name,
                    });
                }
                Ok(entries)
            })
        })
    }

    fn open(&self, rel: &RelPath) -> Result<Box<dyn ContentReader>, SourceError> {
        let path = self.remote_path(rel);
        let rel = rel.clone();
        // Open the remote handle and fstat its length (one round-trip), holding the session lock only
        // for the open — NOT for the subsequent ranged reads. The returned `File` carries its own
        // `Arc` to the session's request machinery, so it streams independently of this borrow.
        let (file, len) = self.with_sftp(move |sftp| {
            Box::pin(async move {
                let file = sftp.open(&path).await.map_err(|e| map_sftp(&rel, e))?;
                let len = file
                    .metadata()
                    .await
                    .map_err(|e| map_sftp(&rel, e))?
                    .size
                    .unwrap_or(0);
                Ok((file, len))
            })
        })?;
        Ok(Box::new(SftpFileReader {
            file: Mutex::new(file),
            handle: self.runtime.handle().clone(),
            len,
        }))
    }
}

/// A streaming [`ContentReader`] over a remote SFTP file: each [`read_at`](ContentReader::read_at)
/// fetches just that window over the wire (no whole-file download), bounding memory and letting the
/// engine's [`full_equal`](confold_core) short-circuit stop at the first differing block. `as_slice`
/// is `None` on purpose so callers take the ranged path.
///
/// Streaming reader: valid only while the owning [`SftpSource`] (hence its runtime) is alive — see the
/// `ContentReader` lifetime contract. `read_at` must run on a synchronous thread (it `block_on`s the
/// runtime); the compare engine calls it from rayon/sync threads, never from inside the runtime.
struct SftpFileReader {
    /// `File` needs `&mut` for seek/read and is not `Sync`; the `Mutex` gives interior mutability and
    /// serializes the (always single-threaded-per-reader) access.
    file: Mutex<File>,
    /// Handle to the [`SftpSource`]'s runtime, to drive the async seek/read to completion synchronously.
    handle: tokio::runtime::Handle,
    len: u64,
}

impl ContentReader for SftpFileReader {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut file = self
            .file
            .lock()
            .map_err(|_| SourceError::Other("SFTP file lock poisoned".to_owned()))?;
        self.handle.block_on(async move {
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| SourceError::io("<sftp seek>", e))?;
            // One SFTP READ returns at most the server's max packet (~32 KiB), so loop to fill the
            // caller's block — matching the `read_at` contract (up to `buf.len()`, short only at EOF).
            let mut filled = 0;
            while filled < buf.len() {
                let n = file
                    .read(&mut buf[filled..])
                    .await
                    .map_err(|e| SourceError::io("<sftp read>", e))?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            Ok(filled)
        })
    }
}

impl SourceMut for SftpSource {
    fn write_file(&self, rel: &RelPath, data: &dyn ContentReader) -> Result<(), SourceError> {
        let path = self.remote_path(rel);

        // Create parent directories on the server first (mkdir-per-component, ignoring "exists").
        if let Some(parent) = rel.parent() {
            self.create_dir_all(&parent)?;
        }

        // Open the remote write handle once (holding the session lock only for the open). The returned
        // `File` carries its own Arc to the session, so we stream blocks to it without holding the lock.
        let rel_open = rel.clone();
        let open_path = path.clone();
        let mut file = self.with_sftp(move |sftp| {
            Box::pin(async move {
                sftp.open_with_flags(
                    &open_path,
                    OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
                )
                .await
                .map_err(|e| map_sftp(&rel_open, e))
            })
        })?;

        // Stream block-by-block: read from the source (synchronous, and crucially OUTSIDE the dest
        // `block_on` so a streaming source reader's own `block_on` never nests) then write to the
        // remote file. Memory stays bounded to one block regardless of file size. Sequential offsets,
        // so the write handle's position advances naturally — no dest-side seek needed.
        let mut offset = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = data.read_at(offset, &mut buf)?;
            if n == 0 {
                break;
            }
            self.runtime
                .block_on(async { file.write_all(&buf[..n]).await })
                .map_err(|e| SourceError::io(rel.to_string(), e))?;
            offset += n as u64;
        }
        self.runtime
            .block_on(async { file.shutdown().await })
            .map_err(|e| SourceError::io(rel.to_string(), e))?;
        Ok(())
    }

    fn rename(&self, from: &RelPath, to: &RelPath) -> Result<(), SourceError> {
        let from_path = self.remote_path(from);
        let to_path = self.remote_path(to);
        // Try a plain SFTP rename first (the common case: `to` is a fresh name, e.g. a new file's temp).
        let rel_to = to.clone();
        let (f, t) = (from_path.clone(), to_path.clone());
        let first = self.with_sftp(move |sftp| {
            Box::pin(async move { sftp.rename(f, t).await.map_err(|e| map_sftp(&rel_to, e)) })
        });
        if first.is_ok() {
            return Ok(());
        }
        // `SSH_FXP_RENAME` refuses an existing target on most servers, so the likely cause is that `to`
        // already exists. Remove it and retry once. The source temp is already fully written, so the
        // only non-atomic window is between this remove and the retry (fast metadata ops) — never during
        // the long content stream. If the retry also fails, `copy_from` propagates the error and the
        // caller keeps the origin (so no source data is lost).
        let _ = self.remove(to);
        let rel_to = to.clone();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                sftp.rename(from_path, to_path)
                    .await
                    .map_err(|e| map_sftp(&rel_to, e))
            })
        })
    }

    fn supports_atomic_replace(&self) -> bool {
        *self.atomic_replace.get_or_init(|| self.probe_rename_support())
    }

    fn create_dir_all(&self, rel: &RelPath) -> Result<(), SourceError> {
        // SFTP has no recursive mkdir, so create each ancestor in turn; an "already exists" failure
        // on an intermediate component is benign and ignored.
        let root = self.root.clone();
        let components: Vec<String> = rel.components().to_vec();
        let rel = rel.clone();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let mut current = RelPath::root();
                for component in &components {
                    current = current.child(component);
                    let path = join_remote(&root, &current);
                    match sftp.create_dir(&path).await {
                        Ok(()) => {}
                        Err(russh_sftp::client::error::Error::Status(s))
                            if s.status_code == StatusCode::Failure =>
                        {
                            // Servers report "directory already exists" as a generic Failure — but so
                            // are permission/quota errors. Disambiguate by checking the path: tolerate
                            // only if it is now a directory; otherwise surface the failure instead of
                            // masking it until a later, confusing error on a child write.
                            let exists_as_dir = matches!(
                                sftp.metadata(&path).await,
                                Ok(meta) if meta.file_type().is_dir()
                            );
                            if !exists_as_dir {
                                return Err(map_sftp(&rel, russh_sftp::client::error::Error::Status(s)));
                            }
                        }
                        Err(e) => return Err(map_sftp(&rel, e)),
                    }
                }
                Ok(())
            })
        })
    }

    fn remove(&self, rel: &RelPath) -> Result<(), SourceError> {
        let path = self.remote_path(rel);
        let root = self.root.clone();
        let rel = rel.clone();
        self.with_sftp(move |sftp| {
            Box::pin(async move { remove_recursive(sftp, &root, &rel, &path).await })
        })
    }
}

/// Recursively remove the file or directory at `path` on the server.
async fn remove_recursive(
    sftp: &SftpSession,
    root: &str,
    rel: &RelPath,
    path: &str,
) -> Result<(), SourceError> {
    let meta = sftp
        .symlink_metadata(path)
        .await
        .map_err(|e| map_sftp(rel, e))?;
    if meta.file_type().is_dir() {
        let children = sftp.read_dir(path).await.map_err(|e| map_sftp(rel, e))?;
        for child in children {
            let child_rel = rel.child(&child.file_name());
            let child_path = join_remote(root, &child_rel);
            Box::pin(remove_recursive(sftp, root, &child_rel, &child_path)).await?;
        }
        sftp.remove_dir(path).await.map_err(|e| map_sftp(rel, e))
    } else {
        sftp.remove_file(path).await.map_err(|e| map_sftp(rel, e))
    }
}

/// SSH client handler. We accept any server host key for now (see crate-level limitations).
struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TODO: known_hosts verification. Accept-any is a documented stopgap.
        Ok(true)
    }
}

/// Normalize a configured root into a clean absolute prefix (no trailing slash except for `"/"`).
fn normalize_root(root: &str) -> String {
    let trimmed = root.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Join a normalized `root` with a [`RelPath`] into an absolute POSIX path.
fn join_remote(root: &str, rel: &RelPath) -> String {
    if rel.is_root() {
        return root.to_owned();
    }
    let joined = rel.components().join("/");
    if root == "/" {
        format!("/{joined}")
    } else {
        format!("{root}/{joined}")
    }
}

/// A unique temp file name for the rename-support probe (`pid` + counter keep it collision-free).
fn probe_name(suffix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        ".confold-probe.{}.{}.{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        suffix,
    )
}

/// Zero-length content, used by the rename-support probe to create a throwaway empty file.
struct ProbeReader;

impl ContentReader for ProbeReader {
    fn len(&self) -> u64 {
        0
    }
    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<usize, SourceError> {
        Ok(0)
    }
}

/// Map an `russh` transport/SSH error onto a [`SourceError`] for the connection phase.
fn map_ssh(err: russh::Error) -> SourceError {
    SourceError::Other(format!("SSH error: {err}"))
}

/// Map an `russh-sftp` client error onto a [`SourceError`], annotated with the relative path.
fn map_sftp(rel: &RelPath, err: russh_sftp::client::error::Error) -> SourceError {
    use russh_sftp::client::error::Error as E;
    match err {
        // Translate "no such file" into a matching `io::ErrorKind` so callers can pattern-match it
        // like the local backend.
        E::Status(s) if s.status_code == StatusCode::NoSuchFile => SourceError::io(
            rel.to_string(),
            std::io::Error::new(std::io::ErrorKind::NotFound, s.error_message),
        ),
        E::Status(s) if s.status_code == StatusCode::PermissionDenied => SourceError::io(
            rel.to_string(),
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, s.error_message),
        ),
        E::IO(msg) => SourceError::io(rel.to_string(), std::io::Error::other(msg)),
        other => SourceError::Other(format!("SFTP error at {rel}: {other}")),
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn normalize_root_strips_trailing_slash() {
        assert_eq!(normalize_root("/"), "/");
        assert_eq!(normalize_root("/home/u/data/"), "/home/u/data");
        assert_eq!(normalize_root("/home/u/data"), "/home/u/data");
        assert_eq!(normalize_root(""), "/");
        assert_eq!(normalize_root("///"), "/");
    }

    #[test]
    fn join_remote_maps_relpath_under_root() {
        let rel = RelPath::root().child("a").child("b.txt");
        assert_eq!(join_remote("/", &rel), "/a/b.txt");
        assert_eq!(join_remote("/srv/data", &rel), "/srv/data/a/b.txt");
        // root maps to the base dir itself.
        assert_eq!(join_remote("/srv/data", &RelPath::root()), "/srv/data");
        assert_eq!(join_remote("/", &RelPath::root()), "/");
    }

    #[test]
    fn auth_debug_is_redacted() {
        let pw = format!("{:?}", SftpAuth::Password("hunter2".to_owned()));
        assert!(!pw.contains("hunter2"));
        let pk = format!(
            "{:?}",
            SftpAuth::PrivateKey {
                pem: "SECRET".to_owned(),
                passphrase: Some("PASS".to_owned())
            }
        );
        assert!(!pk.contains("SECRET") && !pk.contains("PASS"));
    }
}
