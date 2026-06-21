//! Hermetic, no-Docker integration test for the S3 stack.
//!
//! Stands up an in-process `s3s` + `s3s-fs` server over a `tempfile::tempdir()` on
//! `127.0.0.1:<ephemeral port>`, then drives a real [`object_store`] S3 client (built via
//! [`confold_s3::S3Config`]) against it. Proves the pure-Rust client↔server stack works end-to-end with
//! no external infra — and is the reusable harness the `S3Source` trait tests will build on.

use std::net::SocketAddr;
use std::path::PathBuf;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use object_store::list::{PaginatedListOptions, PaginatedListStore};
use object_store::path::Path as ObjPath;
use object_store::{GetOptions, GetRange, ObjectStore, ObjectStoreExt, PutPayload};
use s3s::auth::SimpleAuth;
use s3s::service::S3ServiceBuilder;
use s3s_fs::FileSystem;
use tokio::net::TcpListener;

use confold_s3::{S3Config, S3Source};
use confold_vfs::{ContentReader, EntryKind, RelPath, Source, SourceError, SourceMut};

const ACCESS_KEY: &str = "test-access-key";
const SECRET_KEY: &str = "test-secret-key";
const BUCKET: &str = "confold-test";

/// Start `s3s-fs` over `root` on an ephemeral localhost port; returns the bound address. The accept
/// loop runs on a background task for the lifetime of the test process.
async fn spawn_server(root: PathBuf) -> SocketAddr {
    let fs = FileSystem::new(&root).expect("s3s-fs filesystem");
    let service = {
        let mut b = S3ServiceBuilder::new(fs);
        b.set_auth(SimpleAuth::from_single(ACCESS_KEY, SECRET_KEY));
        b.build()
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let http = ConnBuilder::new(TokioExecutor::new());
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else { break };
            let svc = service.clone(); // cheap (Arc inside)
            let http = http.clone();
            tokio::spawn(async move {
                let _ = http.serve_connection(TokioIo::new(socket), svc).await;
            });
        }
    });
    addr
}

fn s3_config(addr: SocketAddr) -> S3Config {
    S3Config {
        endpoint: Some(format!("http://{addr}")),
        region: "us-east-1".to_string(),
        bucket: BUCKET.to_string(),
        access_key_id: ACCESS_KEY.to_string(),
        secret_access_key: SECRET_KEY.to_string(),
    }
}

fn client(addr: SocketAddr) -> impl ObjectStore {
    s3_config(addr).build_store().expect("build object_store client")
}

#[tokio::test(flavor = "multi_thread")]
async fn s3s_fs_roundtrips_put_get_range_and_list() {
    let dir = tempfile::tempdir().unwrap();
    // s3s-fs maps a bucket to a subdirectory of the served root; it does NOT auto-create it.
    std::fs::create_dir(dir.path().join(BUCKET)).unwrap();

    let addr = spawn_server(dir.path().to_path_buf()).await;
    let store = client(addr);

    let key = ObjPath::from("dir/hello.txt");
    store
        .put(&key, PutPayload::from_static(b"hello world"))
        .await
        .unwrap();

    // Full read.
    let got = store.get(&key).await.unwrap().bytes().await.unwrap();
    assert_eq!(&got[..], b"hello world");

    // Ranged read — the streaming `read_at` primitive `S3Source` will build on. Both the convenience
    // form and the explicit `GetOptions` form.
    let head = store.get_range(&key, 0..5).await.unwrap();
    assert_eq!(&head[..], b"hello");
    let opts = GetOptions {
        range: Some(GetRange::Bounded(0..5)),
        ..Default::default()
    };
    let head2 = store.get_opts(&key, opts).await.unwrap().bytes().await.unwrap();
    assert_eq!(&head2[..], b"hello");

    // Delimiter listing — the directory-synthesis primitive (flat keyspace → dirs via common prefixes).
    store
        .put(&ObjPath::from("dir/sub/n.txt"), PutPayload::from_static(b"x"))
        .await
        .unwrap();
    let res = store
        .list_with_delimiter(Some(&ObjPath::from("dir")))
        .await
        .unwrap();
    assert!(
        res.objects.iter().any(|o| o.location == key),
        "expected dir/hello.txt as a direct object"
    );
    assert!(
        !res.common_prefixes.is_empty(),
        "expected dir/sub/ synthesized as a common prefix"
    );
}

// --- S3Source (trait) tests -------------------------------------------------------------------------

/// Start s3s-fs on its OWN thread + runtime (serving forever), returning the bound address. The
/// `S3Source` tests call the synchronous `Source` trait, which `block_on`s its own runtime from this
/// test thread — so the server must not share this thread's runtime (block_on can't nest).
fn spawn_server_thread(root: PathBuf) -> SocketAddr {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let fs = FileSystem::new(&root).unwrap();
            let service = {
                let mut b = S3ServiceBuilder::new(fs);
                b.set_auth(SimpleAuth::from_single(ACCESS_KEY, SECRET_KEY));
                b.build()
            };
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            let http = ConnBuilder::new(TokioExecutor::new());
            loop {
                let Ok((socket, _)) = listener.accept().await else { break };
                let svc = service.clone();
                let http = http.clone();
                tokio::spawn(async move {
                    let _ = http.serve_connection(TokioIo::new(socket), svc).await;
                });
            }
        });
    });
    rx.recv().unwrap()
}

/// Minimal in-memory `ContentReader` for write tests.
struct BytesReader(Vec<u8>);
impl ContentReader for BytesReader {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError> {
        let start = (offset as usize).min(self.0.len());
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

#[test]
fn s3_source_lists_reads_writes_and_removes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(BUCKET)).unwrap();
    let addr = spawn_server_thread(dir.path().to_path_buf());

    let src = S3Source::connect(s3_config(addr), "").unwrap();

    // Write a top-level file + a nested file (SourceMut).
    src.write_file(&RelPath::root().child("a.txt"), &BytesReader(b"alpha".to_vec()))
        .unwrap();
    src.write_file(
        &RelPath::root().child("sub").child("b.txt"),
        &BytesReader(b"beta".to_vec()),
    )
    .unwrap();

    // read_dir(root): one file + one synthesized directory.
    let mut entries = src.read_dir(&RelPath::root()).unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(entries.len(), 2, "got {entries:?}");
    assert_eq!(entries[0].name, "a.txt");
    assert_eq!(entries[0].kind, EntryKind::File);
    assert_eq!(entries[0].size, 5);
    assert_eq!(entries[1].name, "sub");
    assert_eq!(entries[1].kind, EntryKind::Dir);

    // open + ranged read_at (the streaming primitive the engine compares with).
    let reader = src.open(&RelPath::root().child("a.txt")).unwrap();
    assert_eq!(reader.len(), 5);
    let mut buf = [0u8; 3];
    let n = reader.read_at(0, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"alp");
    let n2 = reader.read_at(3, &mut buf).unwrap();
    assert_eq!(&buf[..n2], b"ha");

    // Remove the file, then the directory subtree.
    src.remove(&RelPath::root().child("a.txt")).unwrap();
    src.remove(&RelPath::root().child("sub")).unwrap();
    // All files are gone. (A filesystem-backed S3 server like s3s-fs can leave an empty `sub/` dir
    // placeholder after deleting its last object — real S3 has no empty dirs — so tolerate a lingering
    // empty directory but assert no file survives and the subtree is empty.)
    let remaining = src.read_dir(&RelPath::root()).unwrap();
    assert!(
        remaining.iter().all(|e| e.kind == EntryKind::Dir),
        "no files should survive removal, got {remaining:?}"
    );
    assert!(
        src.read_dir(&RelPath::root().child("sub")).unwrap().is_empty(),
        "the removed subtree must be empty"
    );
}

/// Reproduction for the >1000-key directory listing data-loss report.
///
/// A directory with more keys than one `ListObjectsV2` page (AWS caps a page at 1000 keys) must
/// still list ALL of them — `read_dir` is expected to follow pagination to completion.
///
/// ROOT CAUSE of the original symptom: it is a **test-server limitation of `s3s-fs`, not a bug in
/// our code or in `object_store`**. Against real AWS S3 `read_dir` is correct, because:
///   - `object_store` 0.13.2's `list_with_delimiter` DOES paginate: it drains a paginated stream
///     (`client/list.rs::list_with_delimiter` → `list_paginated` → `pagination.rs::stream_paginated`)
///     until the `NextContinuationToken` is `None`, aggregating every page.
///   - It follows pagination off the `NextContinuationToken` element of the XML response and
///     deliberately ignores `IsTruncated` (S3 guarantees the token is present iff truncated).
///   - `s3s-fs` 0.13.0 (`src/s3.rs::list_objects_v2`) caps the response at `max_keys` (default 1000)
///     and DOES set `is_truncated = true`, but it NEVER sets `next_continuation_token` and ignores any
///     incoming `continuation_token` (only `start_after`-style resumption is implemented). With no
///     token in the response, `object_store` correctly concludes the listing is complete and stops at
///     page 1 — silently dropping keys 1001..N. Real AWS sets the token, so real AWS paginates fully.
///
/// THE FIX: `S3Source::read_dir` no longer uses `list_with_delimiter` (token-only pagination).
/// It drives `PaginatedListStore::list_paginated` itself and advances on EITHER signal — the
/// `NextContinuationToken` (real AWS) OR `start-after`/`offset` advancement past the last key seen
/// (works against s3s-fs, which only supports `start_after`). So this test passes against the test
/// server AND the same code paginates correctly against real AWS.
#[test]
fn s3_source_read_dir_lists_more_than_one_page() {
    const N: usize = 1500;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(BUCKET)).unwrap();
    let addr = spawn_server_thread(dir.path().to_path_buf());

    // PUT N objects directly on the backing store (fast: no per-file S3Source round-trip framing).
    let put_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = client(addr);
    put_rt.block_on(async {
        for i in 0..N {
            store
                .put(
                    &ObjPath::from(format!("bulk/f{i:05}.txt")),
                    PutPayload::from_static(b"x"),
                )
                .await
                .unwrap();
        }
    });

    let src = S3Source::connect(s3_config(addr), "").unwrap();
    let entries = src.read_dir(&RelPath::root().child("bulk")).unwrap();

    // Every entry is a file (flat dir), and we must see all N of them.
    assert!(
        entries.iter().all(|e| e.kind == EntryKind::File),
        "bulk/ holds only files"
    );
    assert_eq!(
        entries.len(),
        N,
        "read_dir must return all {N} keys (paginated), got {}",
        entries.len()
    );
}

/// Reproduction for the >1000-key directory *removal* partial-delete report (the DELETE path).
///
/// `SourceMut::remove` deletes a directory subtree by first listing every object under it. The old
/// implementation listed via `store.list`, which (like `list_with_delimiter`) follows ONLY the
/// `NextContinuationToken`. Against `s3s-fs` — which sets `IsTruncated=true` but never emits a token
/// (see the `read_dir` reproduction above for the full root cause) — the listing stopped after the
/// first 1000 keys, so `remove` silently deleted only ~1000 of >1000 objects: a partial delete.
///
/// THE FIX: the subtree-listing step now drives `list_paginated` with `delimiter: None` and advances
/// on EITHER the continuation token (real AWS) OR `start-after`/`offset` past the last key seen
/// (s3s-fs), so it enumerates — and deletes — the ENTIRE subtree on both servers.
#[test]
fn s3_source_remove_deletes_more_than_one_page() {
    const N: usize = 1500;

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(BUCKET)).unwrap();
    let addr = spawn_server_thread(dir.path().to_path_buf());

    // PUT N objects under `bulk/` directly on the backing store (fast — no S3Source framing).
    let put_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = client(addr);
    put_rt.block_on(async {
        for i in 0..N {
            store
                .put(
                    &ObjPath::from(format!("bulk/f{i:05}.txt")),
                    PutPayload::from_static(b"x"),
                )
                .await
                .unwrap();
        }
    });

    let src = S3Source::connect(s3_config(addr), "").unwrap();

    // Sanity: the subtree starts fully populated.
    assert_eq!(
        src.read_dir(&RelPath::root().child("bulk")).unwrap().len(),
        N,
        "precondition: all {N} objects present before remove"
    );

    // Remove the whole subtree.
    src.remove(&RelPath::root().child("bulk")).unwrap();

    // The subtree must be COMPLETELY gone — not just the first page. Assert via S3Source...
    let remaining = src.read_dir(&RelPath::root().child("bulk")).unwrap();
    assert!(
        remaining.is_empty(),
        "remove must delete the entire >1-page subtree, but {} entries survived",
        remaining.len()
    );
    // ...and via a raw, independently-paginated list straight at the backing store (belt and braces).
    // Use the concrete `AmazonS3` (not the `impl ObjectStore` from `client`) so `list_paginated` —
    // the `PaginatedListStore` trait method — is in scope, and advance on token OR start-after so the
    // check itself isn't subject to the s3s-fs token-only truncation it's guarding against.
    let concrete = s3_config(addr).build_store().unwrap();
    let raw = put_rt.block_on(async {
        let mut token: Option<String> = None;
        let mut offset: Option<String> = None;
        let mut count = 0usize;
        loop {
            let opts = PaginatedListOptions {
                offset: offset.clone(),
                page_token: token.clone(),
                ..Default::default()
            };
            let page = concrete.list_paginated(Some("bulk/"), opts).await.unwrap();
            let mut max_key: Option<String> = None;
            for obj in &page.result.objects {
                let k = obj.location.as_ref().to_owned();
                if max_key.as_deref().is_none_or(|m| k.as_str() > m) {
                    max_key = Some(k);
                }
            }
            count += page.result.objects.len();
            if let Some(t) = page.page_token {
                token = Some(t);
                continue;
            }
            match (max_key, &offset) {
                (Some(k), None) => offset = Some(k),
                (Some(k), Some(prev)) if &k > prev => offset = Some(k),
                _ => break count,
            }
            token = None;
        }
    });
    assert_eq!(raw, 0, "raw list under bulk/ must be empty after remove");
}
