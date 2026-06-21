//! S3 / S3-compatible data source for Confold.
//!
//! Wraps the pure-Rust [`object_store`] AWS backend (rustls — no native deps). [`S3Source`] implements
//! the [`confold_vfs::Source`]/[`SourceMut`] traits so the compare/migrate/sync engine treats an S3
//! bucket like any other backend. S3 is a flat keyspace with no real directories and no rename, so:
//! - directories are *synthesized* from key prefixes (a delimiter listing → `common_prefixes`), and
//!   the listing is paginated to completion so directories with more than one S3 page (1000 keys) are
//!   fully enumerated — see `read_dir`;
//! - `read_at` is a ranged GET (streaming, bounded memory, like the SFTP reader);
//! - writes are a single atomic `PUT` (so `supports_atomic_replace` is `false` and the crash-safe
//!   `copy_from` falls back to a direct write — which is already atomic on S3);
//! - `create_dir_all` is a no-op (prefixes are implicit); `remove` deletes the key and its subtree.
//!
//! The async object_store API is driven to completion on an owned tokio runtime behind the synchronous
//! trait (same approach as `confold-sftp`); callers must invoke from a non-async thread (the engine does).

use std::sync::Arc;

use bytes::Bytes;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::list::{PaginatedListOptions, PaginatedListStore};
use object_store::path::Path as ObjPath;
use object_store::{ObjectStoreExt, PutPayload};

use confold_vfs::{
    Capabilities, ContentReader, EntryKind, EntryMeta, RelPath, Source, SourceError, SourceMut,
};

/// Connection config for an S3 / S3-compatible bucket. Secret fields are never logged (no `Debug`).
#[derive(Clone)]
pub struct S3Config {
    /// Custom endpoint for S3-compatible servers (MinIO, a local `s3s-fs`, …). `None` → real AWS S3.
    pub endpoint: Option<String>,
    /// AWS region (the SigV4 signing scope). Defaults to `us-east-1` when empty.
    pub region: String,
    /// Bucket name.
    pub bucket: String,
    /// Access key id (identity — part of the cache key).
    pub access_key_id: String,
    /// Secret access key (a secret — never log it).
    pub secret_access_key: String,
}

impl S3Config {
    /// Build an [`object_store`] S3 client from this config.
    ///
    /// Uses **path-style** addressing (`<endpoint>/<bucket>/<key>`), which works with any
    /// S3-compatible endpoint (MinIO, `s3s-fs`) as well as AWS. A custom `endpoint` implies a
    /// dev/self-hosted server, so plain HTTP is allowed there; real AWS (no endpoint) stays HTTPS.
    pub fn build_store(&self) -> Result<AmazonS3, object_store::Error> {
        let region = if self.region.is_empty() {
            "us-east-1"
        } else {
            self.region.as_str()
        };
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(self.bucket.as_str())
            .with_region(region)
            .with_access_key_id(self.access_key_id.as_str())
            .with_secret_access_key(self.secret_access_key.as_str())
            .with_virtual_hosted_style_request(false); // false = path style
        if let Some(endpoint) = &self.endpoint {
            builder = builder.with_endpoint(endpoint.as_str()).with_allow_http(true);
        }
        builder.build()
    }
}

/// A [`Source`]/[`SourceMut`] over an S3 bucket (optionally rooted at an in-bucket key `prefix`).
///
/// Owns the tokio runtime that drives object_store's async calls; the runtime is dropped last so the
/// HTTP client stays alive for the source's lifetime.
pub struct S3Source {
    runtime: tokio::runtime::Runtime,
    store: Arc<AmazonS3>,
    /// In-bucket key prefix this source is rooted at (no leading/trailing slash; may be empty).
    prefix: String,
}

impl S3Source {
    /// Build an S3 source for `config`, rooted at the in-bucket key `prefix`. Cheap — object_store is
    /// lazy (no eager connection); the first `read_dir`/`open` does the network round-trip.
    pub fn connect(config: S3Config, prefix: impl Into<String>) -> Result<S3Source, SourceError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| SourceError::io("<s3 runtime>", e))?;
        let store = config
            .build_store()
            .map_err(|e| SourceError::Other(format!("S3 config: {e}")))?;
        Ok(S3Source {
            runtime,
            store: Arc::new(store),
            prefix: prefix.into().trim_matches('/').to_owned(),
        })
    }

    /// The full in-bucket key string for `rel` (the source prefix + the rel components, `/`-joined).
    fn key_string(&self, rel: &RelPath) -> String {
        let mut parts: Vec<&str> = Vec::new();
        parts.extend(self.prefix.split('/').filter(|s| !s.is_empty()));
        parts.extend(rel.components().iter().map(String::as_str));
        parts.join("/")
    }

    /// The object_store [`Path`](ObjPath) for `rel`.
    fn key(&self, rel: &RelPath) -> ObjPath {
        ObjPath::from(self.key_string(rel))
    }
}

fn map_s3(rel: &RelPath, e: object_store::Error) -> SourceError {
    SourceError::Other(format!("S3 error at {rel}: {e}"))
}

impl Source for S3Source {
    fn capabilities(&self) -> Capabilities {
        // List + read + write; no cheap fingerprint (we read bytes to compare content).
        Capabilities::FS_RW
    }

    fn read_dir(&self, rel: &RelPath) -> Result<Vec<EntryMeta>, SourceError> {
        // S3 caps a single `ListObjectsV2` page at 1000 keys, so a directory with more entries must be
        // assembled across multiple pages. We drive the paginated listing ourselves (rather than
        // `list_with_delimiter`, which paginates only via the response's `NextContinuationToken`) so we
        // are robust to BOTH pagination signals a server may use:
        //   - `page_token` (the canonical `NextContinuationToken`) — what real AWS S3 returns; and
        //   - `start-after` (`offset`) advancement — for S3-compatible servers that signal truncation
        //     (`IsTruncated=true`) but never emit a continuation token (e.g. the `s3s-fs` test server),
        //     against which a token-only client would silently stop after the first 1000 keys.
        // One `read_dir` is one non-recursive directory level, so we keep the delimiter so the server
        // collapses deeper keys into common prefixes (synthesized subdirectories).
        let at_root = rel.is_root() && self.prefix.is_empty();
        // `list_with_delimiter` appends a trailing delimiter to scope the prefix to one level; the
        // lower-level `list_paginated` does not, so we add it ourselves (a non-empty prefix is a dir).
        let prefix = if at_root {
            None
        } else {
            Some(format!("{}/", self.key_string(rel)))
        };

        let mut files: Vec<EntryMeta> = Vec::new();
        // Common prefixes can repeat across pages on servers that recompute them per page, so dedupe.
        let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        let mut page_token: Option<String> = None;
        // Largest object key seen so far — used as the exclusive `start-after` offset to advance on
        // servers that don't return a continuation token. `None` until we've seen an object.
        let mut offset: Option<String> = None;

        loop {
            let opts = PaginatedListOptions {
                offset: offset.clone(),
                delimiter: Some("/".into()),
                page_token: page_token.clone(),
                ..Default::default()
            };
            let page = self
                .runtime
                .block_on(self.store.list_paginated(prefix.as_deref(), opts))
                .map_err(|e| map_s3(rel, e))?;

            let mut max_key_this_page: Option<String> = None;
            // Direct child objects → files.
            for obj in page.result.objects {
                let key = obj.location.as_ref().to_owned();
                if max_key_this_page.as_deref().is_none_or(|m| key.as_str() > m) {
                    max_key_this_page = Some(key);
                }
                let Some(name) = obj.location.filename() else {
                    continue; // a placeholder/marker object with no name
                };
                let name = name.to_owned();
                files.push(EntryMeta {
                    rel_path: rel.child(&name),
                    kind: EntryKind::File,
                    size: obj.size,
                    mtime: Some(obj.last_modified.timestamp_millis()),
                    created: None, // S3 exposes no creation time
                    name,
                });
            }
            // Common prefixes → directories (synthesized; S3 has no real dirs).
            for p in page.result.common_prefixes {
                if let Some(name) = p.filename() {
                    dirs.insert(name.to_owned());
                }
            }

            // Decide whether (and how) to fetch another page. Prefer the canonical continuation token;
            // otherwise advance `start-after` past the largest object key we've seen. We must make
            // forward progress to avoid an infinite loop, so the offset has to strictly increase.
            if let Some(token) = page.page_token {
                page_token = Some(token);
                continue;
            }
            let advanced = match (&max_key_this_page, &offset) {
                (Some(k), None) => Some(k.clone()),
                (Some(k), Some(prev)) if k > prev => Some(k.clone()),
                _ => None, // no new object key → nothing more to page through via start-after
            };
            match advanced {
                Some(k) => {
                    offset = Some(k);
                    page_token = None;
                }
                None => break,
            }
        }

        let mut entries = files;
        for name in dirs {
            entries.push(EntryMeta {
                rel_path: rel.child(&name),
                kind: EntryKind::Dir,
                size: 0,
                mtime: None,
                created: None,
                name,
            });
        }
        Ok(entries)
    }

    fn open(&self, rel: &RelPath) -> Result<Box<dyn ContentReader>, SourceError> {
        let key = self.key(rel);
        let meta = self
            .runtime
            .block_on(self.store.head(&key))
            .map_err(|e| map_s3(rel, e))?;
        Ok(Box::new(S3ContentReader {
            store: Arc::clone(&self.store),
            handle: self.runtime.handle().clone(),
            key,
            len: meta.size,
        }))
    }
}

/// A streaming [`ContentReader`] over an S3 object: each [`read_at`](ContentReader::read_at) issues a
/// ranged GET for just that window (no whole-object download), so memory stays bounded and the engine's
/// block short-circuit can stop at the first differing block. `as_slice` is `None` on purpose.
///
/// Streaming reader: valid only while its [`S3Source`]'s runtime is alive (see the `ContentReader`
/// lifetime contract); `read_at` `block_on`s the runtime, so it must run on a synchronous thread.
struct S3ContentReader {
    store: Arc<AmazonS3>,
    handle: tokio::runtime::Handle,
    key: ObjPath,
    len: u64,
}

impl ContentReader for S3ContentReader {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError> {
        if buf.is_empty() || offset >= self.len {
            return Ok(0);
        }
        let end = (offset + buf.len() as u64).min(self.len);
        let store = Arc::clone(&self.store);
        let key = self.key.clone();
        let bytes: Bytes = self
            .handle
            .block_on(async move { store.get_range(&key, offset..end).await })
            .map_err(|e| SourceError::Other(format!("S3 ranged read at {}: {e}", self.key)))?;
        let n = bytes.len();
        buf[..n].copy_from_slice(&bytes);
        Ok(n)
    }
}

impl SourceMut for S3Source {
    fn write_file(&self, rel: &RelPath, data: &dyn ContentReader) -> Result<(), SourceError> {
        // object_store PUTs the whole payload, so materialize the source into memory. (Multipart
        // streaming for very large uploads is a future optimization — see crate docs.)
        let mut content = Vec::with_capacity(data.len() as usize);
        let mut buf = vec![0u8; 64 * 1024];
        let mut offset = 0u64;
        loop {
            let n = data.read_at(offset, &mut buf)?;
            if n == 0 {
                break;
            }
            content.extend_from_slice(&buf[..n]);
            offset += n as u64;
        }
        let key = self.key(rel);
        self.runtime
            .block_on(self.store.put(&key, PutPayload::from(Bytes::from(content))))
            .map_err(|e| map_s3(rel, e))?;
        Ok(())
    }

    fn rename(&self, from: &RelPath, to: &RelPath) -> Result<(), SourceError> {
        // S3 has no rename; object_store implements it as copy + delete (not atomic — see
        // `supports_atomic_replace`).
        let from_key = self.key(from);
        let to_key = self.key(to);
        self.runtime
            .block_on(self.store.rename(&from_key, &to_key))
            .map_err(|e| map_s3(to, e))
    }

    fn supports_atomic_replace(&self) -> bool {
        // A single PUT atomically replaces an object (a failed PUT leaves the prior object intact), so
        // `copy_from`'s direct-write path is already crash-safe. The temp+rename strategy would only add
        // a non-atomic copy+delete, so we opt out of it.
        false
    }

    fn create_dir_all(&self, _rel: &RelPath) -> Result<(), SourceError> {
        // S3 directories are implicit (key prefixes) — nothing to create.
        Ok(())
    }

    fn remove(&self, rel: &RelPath) -> Result<(), SourceError> {
        let key = self.key(rel);
        // List the full recursive subtree under `key/` to delete it. We drive `list_paginated`
        // ourselves (rather than `store.list`, which paginates only via the response's
        // `NextContinuationToken`) so we are robust to BOTH pagination signals — exactly like
        // `read_dir`, but with `delimiter: None` so the server returns every object at every depth
        // (a recursive subtree) instead of collapsing deeper keys into common prefixes:
        //   - `page_token` (the canonical `NextContinuationToken`) — what real AWS S3 returns; and
        //   - `start-after` (`offset`) advancement — for S3-compatible servers that signal truncation
        //     (`IsTruncated=true`) but never emit a continuation token (e.g. the `s3s-fs` test server),
        //     against which a token-only client would silently delete only the first 1000 keys.
        // The trailing `/` scopes the listing to keys strictly *under* `rel` (never `key` itself), so a
        // non-empty result means `rel` is a directory subtree.
        let prefix = format!("{}/", self.key_string(rel));

        let mut under: Vec<ObjPath> = Vec::new();
        let mut page_token: Option<String> = None;
        // Largest object key seen so far — used as the exclusive `start-after` offset to advance on
        // servers that don't return a continuation token. `None` until we've seen an object.
        let mut offset: Option<String> = None;

        loop {
            let opts = PaginatedListOptions {
                offset: offset.clone(),
                delimiter: None, // recursive: every object at every depth, no common-prefix collapse
                page_token: page_token.clone(),
                ..Default::default()
            };
            let page = self
                .runtime
                .block_on(self.store.list_paginated(Some(prefix.as_str()), opts))
                .map_err(|e| map_s3(rel, e))?;

            let mut max_key_this_page: Option<String> = None;
            for obj in page.result.objects {
                let key = obj.location.as_ref().to_owned();
                if max_key_this_page.as_deref().is_none_or(|m| key.as_str() > m) {
                    max_key_this_page = Some(key);
                }
                under.push(obj.location);
            }

            // Prefer the canonical continuation token; otherwise advance `start-after` past the largest
            // object key we've seen. The offset has to strictly increase to avoid an infinite loop.
            if let Some(token) = page.page_token {
                page_token = Some(token);
                continue;
            }
            let advanced = match (&max_key_this_page, &offset) {
                (Some(k), None) => Some(k.clone()),
                (Some(k), Some(prev)) if k > prev => Some(k.clone()),
                _ => None, // no new object key → nothing more to page through via start-after
            };
            match advanced {
                Some(k) => {
                    offset = Some(k);
                    page_token = None;
                }
                None => break,
            }
        }

        if under.is_empty() {
            // `rel` is a file (or a non-existent / empty-dir path): delete the exact object, tolerating
            // "not found" so removing an already-empty directory prefix is a no-op success.
            return match self.runtime.block_on(self.store.delete(&key)) {
                Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
                Err(e) => Err(map_s3(rel, e)),
            };
        }
        // Directory subtree: delete every object under it (no object exists at `key` itself to delete).
        for location in under {
            self.runtime
                .block_on(self.store.delete(&location))
                .map_err(|e| map_s3(rel, e))?;
        }
        Ok(())
    }
}
