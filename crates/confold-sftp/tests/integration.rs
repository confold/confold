//! Hermetic integration test for [`confold_sftp::SftpSource`].
//!
//! Stands up an in-process SSH + SFTP server (russh server side + russh-sftp server side) serving a
//! `tempfile::tempdir()` directory on `127.0.0.1:<ephemeral port>`, then drives the real
//! `SftpSource` client against it over a genuine TCP/SSH connection. No fixtures, no live server:
//! the host key is an ed25519 key generated in-test and the password is fixed.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    File as SftpFile, FileAttributes, Handle, Name, Status, StatusCode, Version,
};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use confold_vfs::{EntryKind, RelPath, Source, SourceMut};

use confold_sftp::{SftpAuth, SftpConfig, SftpSource};

const PASSWORD: &str = "test-password";

// ---------------------------------------------------------------------------
// SSH server side: accept the password, hand the SFTP subsystem to our handler.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SshServer {
    root: PathBuf,
}

impl russh::server::Server for SshServer {
    type Handler = SshSession;

    fn new_client(&mut self, _: Option<SocketAddr>) -> Self::Handler {
        SshSession {
            root: self.root.clone(),
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

struct SshSession {
    root: PathBuf,
    channels: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
}

impl russh::server::Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        if password == PASSWORD {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.channels.lock().await.insert(channel.id(), channel);
        Ok(true)
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" {
            let channel = self.channels.lock().await.remove(&channel_id).unwrap();
            session.channel_success(channel_id)?;
            let handler = SftpHandler::new(self.root.clone());
            russh_sftp::server::run(channel.into_stream(), handler).await;
        } else {
            session.channel_failure(channel_id)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SFTP server side: a thin shim over the local filesystem rooted at `root`.
// ---------------------------------------------------------------------------

struct OpenFile {
    file: std::fs::File,
}

struct SftpHandler {
    root: PathBuf,
    version: Option<u32>,
    /// Open file handles, keyed by an opaque id string.
    files: HashMap<String, OpenFile>,
    /// Directory listings already fully drained (so the next `readdir` returns EOF).
    dirs_done: HashMap<String, bool>,
    next_id: u64,
}

impl SftpHandler {
    fn new(root: PathBuf) -> Self {
        SftpHandler {
            root,
            version: None,
            files: HashMap::new(),
            dirs_done: HashMap::new(),
            next_id: 0,
        }
    }

    /// Map an absolute POSIX path from the wire to a path under the served root, refusing escapes.
    fn resolve(&self, remote: &str) -> Result<PathBuf, StatusCode> {
        let mut path = self.root.clone();
        for comp in Path::new(remote).components() {
            match comp {
                Component::RootDir | Component::CurDir => {}
                Component::Normal(c) => path.push(c),
                // Disallow `..` and prefixes — keep everything inside the sandbox.
                _ => return Err(StatusCode::PermissionDenied),
            }
        }
        Ok(path)
    }

    fn fresh_handle(&mut self) -> String {
        self.next_id += 1;
        format!("h{}", self.next_id)
    }
}

/// Build SFTP `FileAttributes` from std metadata.
fn attrs_from(md: &std::fs::Metadata) -> FileAttributes {
    let mut attrs = FileAttributes::from(md);
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as u32);
    attrs.mtime = mtime;
    attrs
}

fn io_status(err: &std::io::Error) -> StatusCode {
    match err.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn ok_status(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_string(),
        language_tag: "en-US".to_string(),
    }
}

impl russh_sftp::server::Handler for SftpHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        self.version = Some(version);
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        // Normalize to an absolute POSIX path within the sandbox; the client only uses this to anchor.
        let resolved = self.resolve(&path)?;
        let rel = resolved
            .strip_prefix(&self.root)
            .map_err(|_| StatusCode::Failure)?;
        let mut out = String::from("/");
        let joined: Vec<String> = rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        out.push_str(&joined.join("/"));
        Ok(Name {
            id,
            files: vec![SftpFile::dummy(out)],
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let resolved = self.resolve(&path)?;
        if !resolved.is_dir() {
            return Err(StatusCode::NoSuchFile);
        }
        // Use the wire path as the handle; track drain state per handle.
        self.dirs_done.insert(path.clone(), false);
        Ok(Handle { id, handle: path })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        if *self.dirs_done.get(&handle).unwrap_or(&true) {
            return Err(StatusCode::Eof);
        }
        let resolved = self.resolve(&handle)?;
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&resolved).map_err(|e| io_status(&e))? {
            let entry = entry.map_err(|e| io_status(&e))?;
            let md = entry.metadata().map_err(|e| io_status(&e))?;
            files.push(SftpFile::new(
                entry.file_name().to_string_lossy().into_owned(),
                attrs_from(&md),
            ));
        }
        self.dirs_done.insert(handle, true);
        Ok(Name { id, files })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: russh_sftp::protocol::OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        use russh_sftp::protocol::OpenFlags;
        let resolved = self.resolve(&filename)?;
        let mut opts = std::fs::OpenOptions::new();
        opts.read(pflags.contains(OpenFlags::READ));
        opts.write(pflags.contains(OpenFlags::WRITE));
        opts.create(pflags.contains(OpenFlags::CREATE));
        opts.truncate(pflags.contains(OpenFlags::TRUNCATE));
        opts.append(pflags.contains(OpenFlags::APPEND));
        // A bare read open still needs the read flag.
        if !pflags.contains(OpenFlags::WRITE) {
            opts.read(true);
        }
        let file = opts.open(&resolved).map_err(|e| io_status(&e))?;
        let handle = self.fresh_handle();
        self.files.insert(handle.clone(), OpenFile { file });
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<russh_sftp::protocol::Data, Self::Error> {
        use std::io::{Read, Seek, SeekFrom};
        let open = self.files.get_mut(&handle).ok_or(StatusCode::Failure)?;
        open.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| io_status(&e))?;
        let mut buf = vec![0u8; len as usize];
        let n = open.file.read(&mut buf).map_err(|e| io_status(&e))?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(n);
        Ok(russh_sftp::protocol::Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        use std::io::{Seek, SeekFrom, Write};
        let open = self.files.get_mut(&handle).ok_or(StatusCode::Failure)?;
        open.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| io_status(&e))?;
        open.file.write_all(&data).map_err(|e| io_status(&e))?;
        Ok(ok_status(id))
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.files.remove(&handle);
        self.dirs_done.remove(&handle);
        Ok(ok_status(id))
    }

    async fn stat(
        &mut self,
        id: u32,
        path: String,
    ) -> Result<russh_sftp::protocol::Attrs, Self::Error> {
        let resolved = self.resolve(&path)?;
        let md = std::fs::metadata(&resolved).map_err(|e| io_status(&e))?;
        Ok(russh_sftp::protocol::Attrs {
            id,
            attrs: attrs_from(&md),
        })
    }

    async fn lstat(
        &mut self,
        id: u32,
        path: String,
    ) -> Result<russh_sftp::protocol::Attrs, Self::Error> {
        let resolved = self.resolve(&path)?;
        let md = std::fs::symlink_metadata(&resolved).map_err(|e| io_status(&e))?;
        Ok(russh_sftp::protocol::Attrs {
            id,
            attrs: attrs_from(&md),
        })
    }

    async fn fstat(
        &mut self,
        id: u32,
        handle: String,
    ) -> Result<russh_sftp::protocol::Attrs, Self::Error> {
        let open = self.files.get(&handle).ok_or(StatusCode::Failure)?;
        let md = open.file.metadata().map_err(|e| io_status(&e))?;
        Ok(russh_sftp::protocol::Attrs {
            id,
            attrs: attrs_from(&md),
        })
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let resolved = self.resolve(&path)?;
        std::fs::create_dir(&resolved).map_err(|e| io_status(&e))?;
        Ok(ok_status(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let resolved = self.resolve(&path)?;
        std::fs::remove_dir(&resolved).map_err(|e| io_status(&e))?;
        Ok(ok_status(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        let resolved = self.resolve(&filename)?;
        std::fs::remove_file(&resolved).map_err(|e| io_status(&e))?;
        Ok(ok_status(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        let from = self.resolve(&oldpath)?;
        let to = self.resolve(&newpath)?;
        // Mimic plain SSH_FXP_RENAME (e.g. OpenSSH without the posix-rename extension): refuse to
        // clobber an existing target, forcing the client's remove-then-rename fallback.
        if to.exists() {
            return Err(StatusCode::Failure);
        }
        std::fs::rename(&from, &to).map_err(|e| io_status(&e))?;
        Ok(ok_status(id))
    }
}

// ---------------------------------------------------------------------------
// Harness: spin the server on an ephemeral port, return its address.
// ---------------------------------------------------------------------------

/// Start the in-process SSH/SFTP server serving `root`, returning the bound port. The server task
/// is detached and lives for the duration of the test process.
async fn start_server(root: PathBuf) -> u16 {
    let key =
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::ssh_key::Algorithm::Ed25519)
            .unwrap();
    let config = Arc::new(russh::server::Config {
        auth_rejection_time: Duration::from_millis(1),
        keys: vec![key],
        ..Default::default()
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let mut server = SshServer { root };
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let handler = server.new_client(None);
            let config = config.clone();
            tokio::spawn(async move {
                let _ = russh::server::run_stream(config, stream, handler).await;
            });
        }
    });

    port
}

/// Connect a `SftpSource` against an in-process server seeded by `seed`, then run `body`.
///
/// Runs the whole thing on a multi-thread tokio runtime via `spawn_blocking` so the synchronous
/// `SftpSource` (which itself owns a runtime and calls `block_on`) is never invoked from within an
/// async task on the same runtime.
fn with_connected_source<F>(seed: impl FnOnce(&Path), body: F)
where
    F: FnOnce(&SftpSource, &Path) + Send + 'static,
{
    let tmp = tempfile::tempdir().unwrap();
    seed(tmp.path());
    let root = tmp.path().to_path_buf();

    let server_rt = tokio::runtime::Runtime::new().unwrap();
    let port = server_rt.block_on(start_server(root.clone()));

    let body_root = root.clone();
    std::thread::spawn(move || {
        let config = SftpConfig {
            host: "127.0.0.1".to_string(),
            port,
            username: "tester".to_string(),
            auth: SftpAuth::Password(PASSWORD.to_string()),
            root: "/".to_string(),
        };
        let source = SftpSource::connect(config).expect("connect");
        body(&source, &body_root);
    })
    .join()
    .expect("test body panicked");

    // Keep the server runtime (and thus the listener) alive until the body finished.
    drop(server_rt);
}

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(dir.join(name), bytes).unwrap();
}

/// Drain a [`ContentReader`] fully via `read_at` in 64 KiB windows (exercises the streaming reader's
/// seek + multi-READ fill loop without ever materializing the whole file up front).
fn read_all_windowed(reader: &dyn confold_vfs::ContentReader) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut offset = 0u64;
    loop {
        let n = reader.read_at(offset, &mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        offset += n as u64;
    }
    out
}

#[test]
fn read_dir_lists_names_kinds_sizes() {
    with_connected_source(
        |root| {
            write(root, "a.txt", b"hello");
            std::fs::create_dir(root.join("sub")).unwrap();
        },
        |src, _root| {
            let mut entries = src.read_dir(&RelPath::root()).unwrap();
            entries.sort_by(|a, b| a.name.cmp(&b.name));

            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].name, "a.txt");
            assert_eq!(entries[0].kind, EntryKind::File);
            assert_eq!(entries[0].size, 5);
            assert_eq!(entries[0].rel_path, RelPath::root().child("a.txt"));
            assert_eq!(entries[1].name, "sub");
            assert_eq!(entries[1].kind, EntryKind::Dir);
        },
    );
}

#[test]
fn open_streams_content_via_read_at() {
    with_connected_source(
        |root| write(root, "f.bin", b"0123456789"),
        |src, _root| {
            let reader = src.open(&RelPath::root().child("f.bin")).unwrap();
            assert_eq!(reader.len(), 10);
            // Streaming reader: no whole-content slice — callers must use the ranged `read_at` path
            // (this is what unlocks the engine's first-difference short-circuit).
            assert!(reader.as_slice().is_none());

            // Ranged read at an offset.
            let mut buf = [0u8; 4];
            assert_eq!(reader.read_at(3, &mut buf).unwrap(), 4);
            assert_eq!(&buf, b"3456");

            // Reading the whole content back window-by-window reconstructs it exactly.
            assert_eq!(read_all_windowed(reader.as_ref()), b"0123456789");

            // Past EOF yields zero bytes.
            assert_eq!(reader.read_at(100, &mut buf).unwrap(), 0);
        },
    );
}

#[test]
fn open_streams_large_file_across_multiple_packets() {
    // A file larger than one SFTP READ packet (~32 KiB): proves the reader's seek + fill-across-
    // multiple-READs loop, ranged reads at arbitrary offsets, and bounded (windowed) memory use.
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let expected = data.clone();
    with_connected_source(
        move |root| write(root, "big.bin", &data),
        move |src, _root| {
            let reader = src.open(&RelPath::root().child("big.bin")).unwrap();
            assert_eq!(reader.len(), expected.len() as u64);
            assert!(reader.as_slice().is_none());

            // Full reconstruction in 64 KiB windows.
            assert_eq!(read_all_windowed(reader.as_ref()), expected);

            // A window straddling the interior: 1000 bytes at offset 100_000.
            let mut mid = [0u8; 1000];
            let n = reader.read_at(100_000, &mut mid).unwrap();
            assert_eq!(n, 1000);
            assert_eq!(&mid[..], &expected[100_000..101_000]);
        },
    );
}

#[test]
fn copy_from_writes_a_new_file() {
    with_connected_source(
        |root| write(root, "f.txt", b"payload"),
        |src, root| {
            let reader = src.open(&RelPath::root().child("f.txt")).unwrap();
            let target = RelPath::root().child("nested").child("copy.txt");
            src.copy_from(&target, reader.as_ref()).unwrap();

            // Read back through the source (streaming reader → via read_at)...
            let copied = src.open(&target).unwrap();
            assert_eq!(read_all_windowed(copied.as_ref()), b"payload");
            // ...and verify the bytes really landed on the served directory on disk.
            let on_disk = std::fs::read(root.join("nested").join("copy.txt")).unwrap();
            assert_eq!(on_disk, b"payload");
        },
    );
}

#[test]
fn copy_from_streams_large_file() {
    // Copy a file larger than one SFTP packet, source→dest both over SFTP. Exercises the streaming
    // copy's read-block / write-block loop (multiple READ + WRITE packets) with bounded memory, and
    // confirms the bytes are reproduced exactly. `data.read_at` (source) runs outside the dest
    // `block_on`, so the two runtimes' `block_on`s never nest.
    let data: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(31) % 253) as u8).collect();
    let expected = data.clone();
    with_connected_source(
        move |root| write(root, "big-src.bin", &data),
        move |src, root| {
            let reader = src.open(&RelPath::root().child("big-src.bin")).unwrap();
            let target = RelPath::root().child("out").child("big-dst.bin");
            src.copy_from(&target, reader.as_ref()).unwrap();

            // Read back through the source, windowed.
            let copied = src.open(&target).unwrap();
            assert_eq!(read_all_windowed(copied.as_ref()), expected);
            // And on the served directory on disk.
            let on_disk = std::fs::read(root.join("out").join("big-dst.bin")).unwrap();
            assert_eq!(on_disk, expected);
        },
    );
}

#[test]
fn supports_atomic_replace_true_when_server_has_rename() {
    with_connected_source(
        |_root| {},
        |src, root| {
            // The in-process server implements `rename`, so the probe must report atomic replace.
            assert!(src.supports_atomic_replace());
            // Cached: a second call must not re-probe (and not leave files either).
            assert!(src.supports_atomic_replace());

            let probe_leftovers: Vec<_> = std::fs::read_dir(root)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains("confold-probe"))
                .collect();
            assert!(probe_leftovers.is_empty(), "probe leftovers: {probe_leftovers:?}");
        },
    );
}

#[test]
fn copy_from_overwrites_existing_file_via_remove_retry() {
    with_connected_source(
        |root| write(root, "target.txt", b"OLD-CONTENT"),
        |src, root| {
            // Server has rename → atomic temp+rename path is active.
            assert!(src.supports_atomic_replace());

            let target = RelPath::root().child("target.txt");
            // Overwrite an existing file. The server refuses rename-over-existing (like plain SSH_FXP_
            // RENAME), so this drives the remove-then-retry branch of SftpSource::rename.
            src.copy_from(&target, &Bytes(b"NEW".to_vec())).unwrap();

            assert_eq!(std::fs::read(root.join("target.txt")).unwrap(), b"NEW");

            // No temp/probe files left behind anywhere in the root.
            let leftovers: Vec<_> = std::fs::read_dir(root)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains("confold-tmp") || n.contains("confold-probe"))
                .collect();
            assert!(leftovers.is_empty(), "temp/probe leftovers: {leftovers:?}");
        },
    );
}

#[test]
fn create_dir_all_then_remove_file_and_dir() {
    with_connected_source(
        |_root| {},
        |src, root| {
            // create_dir_all builds every missing ancestor.
            let deep = RelPath::root().child("a").child("b").child("c");
            src.create_dir_all(&deep).unwrap();
            assert!(root.join("a").join("b").join("c").is_dir());

            // Put a file inside and remove it.
            let file = deep.child("f.txt");
            src.copy_from(&file, &Bytes(b"x".to_vec())).unwrap();
            assert!(root.join("a/b/c/f.txt").is_file());
            src.remove(&file).unwrap();
            assert!(!root.join("a/b/c/f.txt").exists());

            // Recursive directory removal (non-empty tree).
            src.copy_from(&deep.child("keep.txt"), &Bytes(b"y".to_vec()))
                .unwrap();
            src.remove(&RelPath::root().child("a")).unwrap();
            assert!(!root.join("a").exists());
        },
    );
}

/// Minimal in-memory `ContentReader` for write tests.
struct Bytes(Vec<u8>);

impl confold_vfs::ContentReader for Bytes {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, confold_vfs::SourceError> {
        let start = offset.min(self.0.len() as u64) as usize;
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.0)
    }
}
