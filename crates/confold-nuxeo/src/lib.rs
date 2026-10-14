//! Nuxeo read-only data source for Confold.
//!
//! Wraps the Nuxeo REST API (LTS 2023/2025) behind the [`confold_vfs::Source`] trait so the
//! compare/migrate/sync engine treats a Nuxeo repository like any other backend. Read-only:
//! `capabilities = list + read + fingerprint` (no write — Nuxeo is a migration *origin*).
//!
//! **Tree mapping** (design note: `confold.nuxeo-paperless-connectors`, option B):
//! - a folderish document (facet `Folderish`) maps to a directory, named by its **title**;
//! - a leaf document with exactly one blob (the main `file:content`) maps to a **file** named by
//!   its title plus the blob's extension (the extension drives the UI's viewer selection);
//! - a leaf document with several blobs — the main document plus `files:files` attachments, e.g. a
//!   contract and its signed scan — maps to a **directory** holding those blobs as files, so the
//!   document↔attachment association survives the projection;
//! - a leaf document with no `file`/`files` blob (e.g. `Note` documents) is skipped (D-nux-3), as
//!   are soft-deleted (trashed) documents.
//!
//! **Naming is display-only.** Nuxeo path segments diverge from titles (truncated to ~24 chars,
//! sanitized, renames keep the old segment), so names are never reversed back into paths: every
//! listing records an exact, lossless identity (real document path + blob xpath) in a cache keyed
//! by rel path, and `open`/`quick_fingerprint`/`read_dir` resolve through it (with a parent
//! re-listing fallback on a miss).
//!
//! `read_at` is a **ranged GET** on the `@blob` download endpoint (the download servlet honours
//! `Range`), so the reader streams in bounded windows like the SFTP/S3 readers — no buffering, no
//! whole-blob downloads. Servers that ignore `Range` (a plain 200) fall back to a one-time
//! temp-file download serving every window from disk. Blob digests (MD5 by default) surface
//! through [`Source::quick_fingerprint`].
//!
//! The async reqwest API is driven to completion on an owned tokio runtime behind the synchronous
//! trait (same approach as `confold-sftp`/`confold-s3`); callers must invoke from a non-async thread
//! (the engine does — rayon workers).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use confold_vfs::{
    Capabilities, ContentReader, EntryKind, EntryMeta, Fingerprint, RelPath, Source, SourceError,
};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Page size for `@children` listings. Well under the default `maxPageSize` cap (1000) so any
/// standard install accepts it; the listing loop drives pagination to completion.
const PAGE_SIZE: u64 = 200;

/// Route-picker page size: small enough to keep each picker response quick on large workspaces.
const ROUTE_PAGE_SIZE: u64 = 50;

const MAX_ROUTE_QUERY_LEN: usize = 128;

/// Schemas requested on document fetches — just what the tree mapping needs (blob lists).
const PROPERTIES: &str = "file,files";

/// Connection config for a Nuxeo server. Secret fields are never logged (no `Debug` derive).
#[derive(Clone)]
pub struct NuxeoConfig {
    /// Server base URL including the context path, e.g. `http://host:8080/nuxeo`.
    pub base_url: String,
    /// Basic-auth user name.
    pub username: String,
    /// Basic-auth password (a secret — never log it).
    pub password: String,
    /// Nuxeo document path this source is rooted at, e.g. `/default-domain/workspaces`.
    /// Defaults to `/` (the repository root) when empty.
    pub root: String,
}

/// One folderish Nuxeo document shown in the route picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NuxeoRoute {
    pub name: String,
    pub title: String,
    pub path: String,
}

/// One incrementally loaded route-picker page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NuxeoRoutePage {
    pub entries: Vec<NuxeoRoute>,
    pub has_more: bool,
}

/// A [`Source`] over a Nuxeo repository (read-only).
///
/// Owns the tokio runtime that drives reqwest's async calls; the runtime is dropped last so the
/// HTTP client stays alive for the source's lifetime. Connect is **cheap** (no network call) — the
/// first `read_dir`/`open` does the round-trip, like the S3 backend.
pub struct NuxeoSource {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    /// REST API root: `base_url + /api/v1` (no trailing slash).
    api: reqwest::Url,
    /// Document-path prefix this source is rooted at (`""` or `/a/b`, no trailing slash).
    root: String,
    username: String,
    password: String,
    /// Where each listed entry lives in Nuxeo, keyed by rel-path string. Listings populate it;
    /// `open`/`quick_fingerprint` resolve through it (with a fresh-listing fallback on a miss), so
    /// entry NAMES (titles, which are free text) never have to be reversed back into paths.
    identities: Mutex<HashMap<String, NuxeoIdentity>>,
}

/// Where one listed entry lives in Nuxeo. Entry names are display-only (titles + blob filenames,
/// deduplicated per directory); this is the exact, lossless address.
#[derive(Clone, Debug)]
enum NuxeoIdentity {
    /// A folderish document — listable, never openable.
    Folder { doc_path: String },
    /// A leaf document presenting as a single file: its main blob.
    File {
        doc_path: String,
        length: u64,
        digest: Option<String>,
    },
    /// A multi-blob document presenting as a directory: blob display name → (xpath, length).
    DocDir {
        doc_path: String,
        blobs: BTreeMap<String, (String, u64)>,
    },
}

/// One resolved `open` target: the owning document's absolute Nuxeo path, the blob's xpath inside
/// it, and the blob metadata (size for the reader length).
struct ResolvedBlob {
    doc_path: String,
    xpath: String,
    blob: NuxeoBlob,
}

impl NuxeoSource {
    /// Build a Nuxeo source for `config`. Cheap — no eager connection; the first listing/open does
    /// the network round-trip (and surfaces any auth/config problem then).
    pub fn connect(config: NuxeoConfig) -> Result<NuxeoSource, SourceError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| SourceError::io("<nuxeo runtime>", e))?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("confold-nuxeo/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|e| SourceError::Other(format!("Nuxeo HTTP client: {e}")))?;
        let base = config.base_url.trim_end_matches('/');
        let api = reqwest::Url::parse(&format!("{base}/api/v1"))
            .map_err(|e| SourceError::Other(format!("invalid Nuxeo URL '{base}': {e}")))?;
        let root = normalize_root(&config.root);
        Ok(NuxeoSource {
            runtime,
            client,
            api,
            root,
            username: config.username,
            password: config.password,
            identities: Mutex::new(HashMap::new()),
        })
    }

    /// The Nuxeo document path for `rel` (config root + the rel components), always starting `/`.
    fn nuxeo_path(&self, rel: &RelPath) -> String {
        let mut path = self.root.clone();
        for c in rel.components() {
            path.push('/');
            path.push_str(c);
        }
        if path.is_empty() {
            "/".to_owned()
        } else {
            path
        }
    }

    /// Like [`NuxeoSource::doc_url`], for an absolute Nuxeo document path (as stored in the
    /// identity cache). Each path segment is percent-encoded as a unit (spaces, accents, … survive
    /// round-trips).
    fn doc_url_str(&self, doc_path: &str, tail: &[&str]) -> Result<reqwest::Url, SourceError> {
        let mut url = self.api.clone();
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| SourceError::Other("invalid API base URL".to_owned()))?;
        segments.push("path");
        if doc_path == "/" {
            segments.push("");
        } else {
            segments.extend(doc_path.split('/').filter(|s| !s.is_empty()));
        }
        segments.extend(tail.iter().copied());
        drop(segments);
        Ok(url)
    }

    /// GET the document at an absolute Nuxeo path (identity-cache entries store these). `Ok(None)`
    /// on 404.
    fn fetch_doc_str(&self, doc_path: &str) -> Result<Option<NuxeoDoc>, SourceError> {
        let url = self.doc_url_str(doc_path, &[])?;
        let status_body = self.runtime.block_on(async {
            let resp = self
                .client
                .get(url)
                .basic_auth(&self.username, Some(&self.password))
                .query(&[("properties", PROPERTIES)])
                .send()
                .await
                .map_err(|e| SourceError::Other(format!("Nuxeo request at {doc_path}: {e}")))?;
            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| SourceError::Other(format!("Nuxeo read at {doc_path}: {e}")))?;
            Ok::<_, SourceError>((status, body))
        })?;
        match status_body.0.as_u16() {
            200 => serde_json::from_str(&status_body.1)
                .map(Some)
                .map_err(|e| SourceError::Other(format!("Nuxeo parse at {doc_path}: {e}"))),
            404 => Ok(None),
            code => Err(SourceError::Other(format!(
                "Nuxeo GET {doc_path}: HTTP {code}"
            ))),
        }
    }

    /// List the immediate children of the document at `doc_path` (whose tree identity is `rel`) via
    /// `@children`, driving pagination to completion (a page loop guarded by both
    /// `isNextPageAvailable` and forward progress on new document uids, so a server that repeats a
    /// page cannot loop us forever — the S3 lesson). Every classified child is recorded in the
    /// identity cache under its rel path.
    fn list_children(&self, rel: &RelPath, doc_path: &str) -> Result<Vec<EntryMeta>, SourceError> {
        self.forget_children_of(rel);
        let mut entries = Vec::new();
        let mut used: HashSet<String> = HashSet::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut page_index: u64 = 0;
        loop {
            let page = self.fetch_children_page_at(doc_path, page_index, PAGE_SIZE, PROPERTIES)?;

            let mut new_this_page = 0;
            for doc in &page.entries {
                if seen.insert(doc.uid.clone()) {
                    new_this_page += 1;
                    if let Some((entry, identity)) = classify_child(rel, doc, &mut used) {
                        self.insert_identity(&entry.rel_path, identity);
                        entries.push(entry);
                    }
                }
            }
            let page_len = page.entries.len() as u64;
            if !page.is_next_page_available || page_len == 0 || new_this_page == 0 {
                break;
            }
            page_index += 1;
        }
        Ok(entries)
    }

    /// The absolute Nuxeo document path for a listable (folderish or multi-blob) entry, from the
    /// identity cache. `None` = not cached, or the entry is a plain file / unknown.
    fn listable_doc_path(&self, rel: &RelPath) -> Option<String> {
        match self.lock_identities().get(&rel.to_string())? {
            NuxeoIdentity::Folder { doc_path } | NuxeoIdentity::DocDir { doc_path, .. } => {
                Some(doc_path.clone())
            }
            NuxeoIdentity::File { .. } => None,
        }
    }

    /// Lock the identity cache, recovering from a poisoned lock: the map is a plain cache whose
    /// entries are inserted atomically, so a panic mid-update leaves nothing worse than a partial
    /// listing — failing every subsequent operation forever would be strictly worse.
    fn lock_identities(&self) -> std::sync::MutexGuard<'_, HashMap<String, NuxeoIdentity>> {
        self.identities.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Drop cached identities for entries directly under `rel` (a re-list replaces them; stale
    /// entries would resolve `open` to a document that no longer exists under that name).
    fn forget_children_of(&self, rel: &RelPath) {
        let prefix = if rel.is_root() {
            String::new()
        } else {
            format!("{rel}/")
        };
        let mut identities = self.lock_identities();
        {
            identities.retain(|key, _| {
                !key.strip_prefix(&prefix)
                    .map(|rest| !rest.is_empty() && !rest.contains('/'))
                    .unwrap_or(false)
            });
        }
    }

    fn insert_identity(&self, rel: &RelPath, identity: NuxeoIdentity) {
        self.lock_identities().insert(rel.to_string(), identity);
    }

    /// Resolve `rel` through the identity cache. `None` = not cached. A file is either a
    /// single-blob leaf document (cached under its own rel) or a blob inside a multi-blob
    /// document-directory (resolved through the directory's cached identity).
    fn identity_lookup(&self, rel: &RelPath) -> Option<ResolvedBlob> {
        let name = rel.file_name()?;
        let identities = self.lock_identities();
        if let Some(NuxeoIdentity::File {
            doc_path,
            length,
            digest,
        }) = identities.get(&rel.to_string())
        {
            return Some(ResolvedBlob {
                doc_path: doc_path.clone(),
                xpath: "file:content".to_owned(),
                blob: NuxeoBlob {
                    name: Some(name.to_owned()),
                    length: Some(*length),
                    digest: digest.clone(),
                },
            });
        }
        match identities.get(&rel.parent()?.to_string())? {
            NuxeoIdentity::DocDir { doc_path, blobs } => {
                let (xpath, length) = blobs.get(name)?;
                Some(ResolvedBlob {
                    doc_path: doc_path.clone(),
                    xpath: xpath.clone(),
                    blob: NuxeoBlob {
                        name: Some(name.to_owned()),
                        length: Some(*length),
                        digest: None,
                    },
                })
            }
            _ => None,
        }
    }
    /// Load one page of immediate folderish children for the route picker. The source's configured
    /// root is the directory being browsed — the picker overrides `root` per navigation step.
    pub fn browse_directories_page(&self, page_index: u64) -> Result<NuxeoRoutePage, SourceError> {
        let browse_root = self.nuxeo_path(&RelPath::root());
        let page =
            self.fetch_children_page_at(&browse_root, page_index, ROUTE_PAGE_SIZE, "dublincore")?;
        let entries = page
            .entries
            .into_iter()
            .filter(NuxeoDoc::is_folderish)
            .filter_map(|doc| {
                Some(NuxeoRoute {
                    name: doc.path_segment()?,
                    title: doc.title.clone(),
                    path: doc.path,
                })
            })
            .collect();
        Ok(NuxeoRoutePage {
            entries,
            has_more: page.is_next_page_available,
        })
    }

    /// Search folderish documents globally by title or path, returning one page for the route picker.
    pub fn search_directories_page(
        &self,
        query: &str,
        page_index: u64,
    ) -> Result<NuxeoRoutePage, SourceError> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > MAX_ROUTE_QUERY_LEN {
            return Err(SourceError::Other(format!(
                "Nuxeo route search must contain 1–{MAX_ROUTE_QUERY_LEN} characters"
            )));
        }
        // NXQL LIKE/ILIKE treat `%` and `_` as wildcards; escape them so a literal search still
        // matches literally. Quotes are escaped per NXQL string-literal rules.
        let escaped = query
            .replace('\\', "\\\\")
            .replace('\'', "''")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let predicate = if escaped.starts_with('/') {
            format!("ecm:path STARTSWITH '{escaped}'")
        } else {
            format!("dc:title ILIKE '%{escaped}%'")
        };
        let nxql = format!(
            "SELECT * FROM Document WHERE ecm:isTrashed = 0 AND ecm:isVersion = 0 AND ecm:isProxy = 0 AND ecm:mixinType = 'Folderish' AND {predicate} ORDER BY dc:title"
        );
        let mut url = self.api.clone();
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| SourceError::Other("invalid API base URL".to_owned()))?;
        segments.extend(["search", "lang", "NXQL", "execute"]);
        drop(segments);
        let result = self.runtime.block_on(async {
            let resp = self
                .client
                .get(url)
                .basic_auth(&self.username, Some(&self.password))
                .query(&[
                    ("query", nxql.as_str()),
                    ("properties", "dublincore"),
                    ("pageSize", &ROUTE_PAGE_SIZE.to_string()),
                    ("currentPageIndex", &page_index.to_string()),
                ])
                .send()
                .await
                .map_err(|e| SourceError::Other(format!("Nuxeo route search: {e}")))?;
            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| SourceError::Other(format!("Nuxeo route search: {e}")))?;
            Ok::<_, SourceError>((status, body))
        })?;
        if !result.0.is_success() {
            return Err(SourceError::Other(format!(
                "Nuxeo route search: HTTP {}",
                result.0.as_u16()
            )));
        }
        let page: NuxeoDocumentsPage = serde_json::from_str(&result.1)
            .map_err(|e| SourceError::Other(format!("Nuxeo route search parse: {e}")))?;
        let entries = page
            .entries
            .into_iter()
            .filter(NuxeoDoc::is_folderish)
            .filter_map(|doc| {
                Some(NuxeoRoute {
                    name: doc.path_segment()?,
                    title: doc.title.clone(),
                    path: doc.path,
                })
            })
            .collect();
        Ok(NuxeoRoutePage {
            entries,
            has_more: page.is_next_page_available,
        })
    }

    fn fetch_children_page_at(
        &self,
        doc_path: &str,
        page_index: u64,
        page_size: u64,
        properties: &str,
    ) -> Result<NuxeoDocumentsPage, SourceError> {
        let url = self.doc_url_str(doc_path, &["@children"])?;
        let result = self.runtime.block_on(async {
            let resp = self
                .client
                .get(url)
                .basic_auth(&self.username, Some(&self.password))
                .query(&[
                    ("properties", properties),
                    ("pageSize", &page_size.to_string()),
                    ("currentPageIndex", &page_index.to_string()),
                ])
                .send()
                .await
                .map_err(|e| SourceError::Other(format!("Nuxeo listing {doc_path}: {e}")))?;
            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| SourceError::Other(format!("Nuxeo listing {doc_path}: {e}")))?;
            Ok::<_, SourceError>((status, body))
        })?;
        if !result.0.is_success() {
            return Err(SourceError::Other(format!(
                "Nuxeo listing {doc_path}: HTTP {}",
                result.0.as_u16()
            )));
        }
        serde_json::from_str(&result.1)
            .map_err(|e| SourceError::Other(format!("Nuxeo listing parse {doc_path}: {e}")))
    }

    /// Resolve the file `rel` to its owning document + blob, shared by [`Source::open`] and
    /// [`Source::quick_fingerprint`]. Names are display-only, so resolution goes through the
    /// identity cache (populated by the listing that produced the name); on a miss the parent is
    /// listed once and the lookup retried — never guessed back from the name.
    fn resolve_blob(&self, rel: &RelPath) -> Result<ResolvedBlob, SourceError> {
        if let Some(resolved) = self.identity_lookup(rel) {
            return Ok(resolved);
        }
        let parent = rel
            .parent()
            .ok_or_else(|| SourceError::Other(format!("Nuxeo: '{rel}' has no parent")))?;
        // Lists (and caches) the parent — whether it is a folderish document or a multi-blob
        // document-directory; errors (auth, network, missing) propagate honestly.
        self.read_dir(&parent)?;
        self.identity_lookup(rel).ok_or_else(|| {
            SourceError::Other(format!(
                "Nuxeo: no file '{name}' at {parent}",
                name = rel.file_name().unwrap_or("?")
            ))
        })
    }

    /// Build a streaming reader for `xpath` on the document at the absolute Nuxeo `doc_path`.
    fn blob_reader(
        &self,
        doc_path: &str,
        xpath: &str,
        blob: &NuxeoBlob,
    ) -> Result<Box<dyn ContentReader>, SourceError> {
        let len = blob.length.ok_or_else(|| {
            SourceError::Other(format!("Nuxeo: blob {xpath} at {doc_path} has no size"))
        })?;
        let mut tail: Vec<&str> = vec!["@blob"];
        tail.extend(xpath.split('/'));
        let url = self.doc_url_str(doc_path, &tail)?;
        Ok(Box::new(NuxeoContentReader {
            client: self.client.clone(),
            handle: self.runtime.handle().clone(),
            url,
            len,
            username: self.username.clone(),
            password: self.password.clone(),
            fallback: Mutex::new(None),
        }))
    }
}

/// Normalize a configured root document path: leading `/`, no trailing `/`; empty → `""` (the
/// repository root renders as `/` in [`NuxeoSource::nuxeo_path`]).
fn normalize_root(root: &str) -> String {
    let trimmed = root.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

/// A Nuxeo document JSON (the subset the mapping needs).
#[derive(Deserialize)]
struct NuxeoDoc {
    #[serde(default)]
    uid: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    #[serde(rename = "lastModified")]
    last_modified: String,
    /// Lifecycle state (`project` = live, `deleted` = soft-deleted/trash).
    #[serde(default)]
    state: String,
    #[serde(default)]
    facets: Vec<String>,
    #[serde(default)]
    properties: NuxeoProperties,
}

impl NuxeoDoc {
    /// `true` when the document carries the `Folderish` facet (maps to a directory).
    fn is_folderish(&self) -> bool {
        self.facets.iter().any(|f| f == "Folderish")
    }

    /// `true` for soft-deleted (trashed) documents: they still appear in `@children` but must not
    /// be compared or migrated.
    fn is_trashed(&self) -> bool {
        self.state == "deleted"
    }

    /// The final `/`-component of the Nuxeo path (our entry name — unique among siblings).
    fn path_segment(&self) -> Option<String> {
        self.path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }

    /// The document's blobs in display order: main (`file:content`) first, then the `files:files`
    /// attachments. Empty for blob-less documents (Notes, …).
    fn blobs(&self) -> Vec<BlobRef<'_>> {
        let mut blobs = Vec::new();
        if let Some(main) = &self.properties.file_content {
            blobs.push(BlobRef {
                xpath: "file:content".to_owned(),
                blob: main,
            });
        }
        for (i, entry) in self.properties.files.iter().enumerate() {
            if let Some(b) = &entry.file {
                blobs.push(BlobRef {
                    xpath: format!("files:files/{i}/file"),
                    blob: b,
                });
            }
        }
        blobs
    }
}

/// One addressable blob within a document.
struct BlobRef<'a> {
    xpath: String,
    blob: &'a NuxeoBlob,
}

/// The `file` + `files` schemas of a document's properties.
#[derive(Deserialize, Default)]
struct NuxeoProperties {
    #[serde(default, rename = "file:content")]
    file_content: Option<NuxeoBlob>,
    #[serde(default, rename = "files:files")]
    files: Vec<NuxeoFilesEntry>,
}

/// One `files:files` entry (`{ "file": { …blob… } }`).
#[derive(Deserialize, Default)]
struct NuxeoFilesEntry {
    #[serde(default)]
    file: Option<NuxeoBlob>,
}

/// A Nuxeo blob (the subset the mapping + reader need). `length` arrives as a string on some
/// versions and a number on others — both deserialize.
#[derive(Deserialize, Clone, Default)]
struct NuxeoBlob {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_len")]
    length: Option<u64>,
    #[serde(default)]
    digest: Option<String>,
}

/// A `@children` page of documents (paginated response entity).
#[derive(Deserialize)]
struct NuxeoDocumentsPage {
    #[serde(default)]
    entries: Vec<NuxeoDoc>,
    #[serde(default, rename = "isNextPageAvailable")]
    is_next_page_available: bool,
}

/// `length` as string-or-number → `Option<u64>`.
fn deserialize_len<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Len {
        Num(u64),
        Str(String),
    }
    match Option::<Len>::deserialize(deserializer)? {
        Some(Len::Num(n)) => Ok(Some(n)),
        Some(Len::Str(s)) => s.parse().map(Some).map_err(serde::de::Error::custom),
        None => Ok(None),
    }
}

/// Parse a Nuxeo `lastModified` (RFC 3339) into Unix epoch milliseconds.
fn parse_millis(ts: &str) -> Option<i64> {
    OffsetDateTime::parse(ts, &Rfc3339)
        .ok()
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// Classify one child document of `parent` into a tree entry + identity, or `None` to skip it
/// (zero-blob leaves — Notes and other blob-less document types; D-nux-3). Display names come from
/// the document TITLE (the full human name — Nuxeo truncates path segments to ~24 chars), then the
/// blob's own filename, then the segment; siblings are deduplicated with ` (n)` suffixes.
fn classify_child(
    parent: &RelPath,
    doc: &NuxeoDoc,
    used: &mut HashSet<String>,
) -> Option<(EntryMeta, NuxeoIdentity)> {
    if doc.is_trashed() {
        // Soft-deleted documents still surface in `@children`; comparing or migrating trash would
        // silently resurrect it in the destination.
        return None;
    }
    if doc.facets.iter().any(|f| f == "Proxy") {
        // Publishing proxies point at content living elsewhere; migrating a section view would
        // duplicate the published document under a second name.
        return None;
    }
    let doc_path = doc.path.clone();
    let segment = doc.path_segment().unwrap_or_default();
    let mtime = parse_millis(&doc.last_modified);
    let meta = |name: String, kind: EntryKind, size: u64| EntryMeta {
        rel_path: parent.child(&name),
        kind,
        size,
        mtime,
        created: None,
        name,
    };
    if doc.is_folderish() {
        let name = unique_name(sanitize_name(&display_name(doc, None, &segment)), used);
        Some((
            meta(name, EntryKind::Dir, 0),
            NuxeoIdentity::Folder { doc_path },
        ))
    } else {
        let blobs = doc.blobs();
        match blobs.len() {
            0 => None, // blob-less leaf (e.g. a Note): skipped
            1 => {
                let main = blobs[0].blob;
                // Title first (what the user named the document); the blob filename and the path
                // segment are fallbacks. Make sure the true file extension survives so the UI
                // picks the right viewer.
                let mut base = sanitize_name(&display_name(doc, main.name.as_deref(), &segment));
                if let Some(ext) = main.name.as_deref().and_then(extension_of) {
                    if !base
                        .to_lowercase()
                        .ends_with(&format!(".{}", ext.to_lowercase()))
                    {
                        base = format!("{base}.{ext}");
                    }
                }
                let name = unique_name(base, used);
                Some((
                    meta(name, EntryKind::File, main.length.unwrap_or(0)),
                    NuxeoIdentity::File {
                        doc_path,
                        length: main.length.unwrap_or(0),
                        digest: main.digest.clone(),
                    },
                ))
            }
            // Multi-blob document → a directory of its blobs (association preserved).
            _ => {
                let name = unique_name(sanitize_name(&display_name(doc, None, &segment)), used);
                let mut map = BTreeMap::new();
                for (blob_name, xpath, blob) in blob_entries(doc) {
                    map.insert(blob_name, (xpath, blob.length.unwrap_or(0)));
                }
                Some((
                    meta(name, EntryKind::Dir, 0),
                    NuxeoIdentity::DocDir {
                        doc_path,
                        blobs: map,
                    },
                ))
            }
        }
    }
}

/// The human display base for a document: its title (full name), then a blob filename hint, then
/// the (possibly truncated) path segment — whichever is the first non-empty after sanitizing.
fn display_name(doc: &NuxeoDoc, blob_name: Option<&str>, segment: &str) -> String {
    [Some(doc.title.as_str()), blob_name, Some(segment)]
        .into_iter()
        .flatten()
        .map(sanitize_name)
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "document".to_owned())
}

/// Make free text (a Nuxeo title) safe as a tree entry name. Beyond separators, strip the
/// Windows-reserved character set: the origin is read-only, but migrated files must be creatable
/// on Windows/macOS/Linux destinations (a title like `Presupuesto: 2011` would fail on Windows).
fn sanitize_name(raw: &str) -> String {
    raw.replace(['/', '\\', '\0', ':', '*', '?', '"', '<', '>', '|'], "_")
        .trim()
        .to_owned()
}

/// The final `.`-suffix of a filename, if any (`report.pdf` → `pdf`).
fn extension_of(name: &str) -> Option<&str> {
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext)
}

/// The blobs of a multi-blob document as tree entries: `(display name, xpath, blob)` in display
/// order, deduplicated with ` (n)` suffixes. Each attachment keeps its own blob filename (the
/// document title names the directory, not its attachments); a blob with no filename falls back
/// to `attachment-N`.
fn blob_entries(doc: &NuxeoDoc) -> Vec<(String, String, NuxeoBlob)> {
    let mut used: HashSet<String> = HashSet::new();
    doc.blobs()
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let base = match b.blob.name.as_deref() {
                Some(n) => sanitize_name(n),
                None => attachment_fallback(i),
            };
            let name = unique_name(base, &mut used);
            (name, b.xpath.clone(), b.blob.clone())
        })
        .collect()
}

/// Display name for an attachment blob that carries no filename of its own.
fn attachment_fallback(i: usize) -> String {
    format!("attachment-{}", i + 1)
}

/// Deduplicate a display name with ` (n)` before the extension (`scan.pdf`, `scan (2).pdf`, …).
/// The `used` set is keyed CASE-INVARIANTLY: destination filesystems are commonly
/// case-insensitive (macOS/Windows), so `Report.ods` and `report.ods` from Nuxeo must not collapse
/// into one migrated file (a silent overwrite).
fn unique_name(base: String, used: &mut HashSet<String>) -> String {
    let key = base.to_lowercase();
    if used.insert(key.clone()) {
        return base;
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
        _ => (base.clone(), String::new()),
    };
    for n in 2.. {
        let candidate = format!("{stem} ({n}){ext}");
        if used.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!("the counter eventually produces an unused name")
}

impl Source for NuxeoSource {
    fn capabilities(&self) -> Capabilities {
        // Read-only: list + read + fingerprint (blob digests). No write — Nuxeo is an origin.
        Capabilities {
            list: true,
            read: true,
            fingerprint: true,
            write: false,
        }
    }

    fn read_dir(&self, rel: &RelPath) -> Result<Vec<EntryMeta>, SourceError> {
        // The repository root (empty configured root) is not itself a fetchable document — only its
        // children are listable (the original root-discovery lesson). A non-empty configured root is
        // a real user-provided path. Any deeper directory is addressed by its cached identity: entry
        // names are display-only (titles), and Nuxeo path segments diverge from titles (truncation,
        // sanitization, renames), so a name-derived path would 404. On a cache miss, list the parent
        // (which classifies and caches this folder) and retry once.
        if rel.is_root() && self.root.is_empty() {
            return self.list_children(rel, "/");
        }
        let doc_path = if rel.is_root() {
            self.nuxeo_path(rel)
        } else {
            match self.listable_doc_path(rel) {
                Some(doc_path) => doc_path,
                None => {
                    let parent = rel.parent().ok_or_else(|| {
                        SourceError::Other(format!("Nuxeo: '{rel}' has no parent"))
                    })?;
                    self.read_dir(&parent)?;
                    // Still unknown after listing the parent: the child does not exist in Nuxeo.
                    // Typed Io(NotFound) so the engine's one-sided-folder handling classifies the
                    // other side instead of failing the whole comparison.
                    self.listable_doc_path(rel).ok_or_else(|| SourceError::Io {
                        path: rel.to_string(),
                        source: std::io::Error::from(std::io::ErrorKind::NotFound),
                    })?
                }
            }
        };
        let doc = match self.fetch_doc_str(&doc_path)? {
            Some(doc) => doc,
            // Typed Io(NotFound), never `Other`: the engine's one-sided-folder handling recognizes
            // this kind as "this side has no such directory" and classifies the other side.
            None => {
                return Err(SourceError::Io {
                    path: rel.to_string(),
                    source: std::io::Error::from(std::io::ErrorKind::NotFound),
                })
            }
        };
        if doc.is_folderish() {
            self.list_children(rel, &doc_path)
        } else if doc.blobs().len() > 1 {
            // A multi-blob document presents as a directory of its blobs. Cache its identity so
            // `open` on the blob names below resolves without any name→path guessing.
            let mtime = parse_millis(&doc.last_modified);
            let mut blobs = BTreeMap::new();
            let mut used: HashSet<String> = HashSet::new();
            let entries = blob_entries(&doc)
                .into_iter()
                .map(|(name, xpath, blob)| {
                    let display = unique_name(name, &mut used);
                    blobs.insert(display.clone(), (xpath, blob.length.unwrap_or(0)));
                    EntryMeta {
                        rel_path: rel.child(&display),
                        kind: EntryKind::File,
                        size: blob.length.unwrap_or(0),
                        mtime,
                        created: None,
                        name: display,
                    }
                })
                .collect();
            self.insert_identity(rel, NuxeoIdentity::DocDir { doc_path, blobs });
            Ok(entries)
        } else {
            // A single-blob (or blob-less) leaf presents as a file in its parent's listing.
            Err(SourceError::Other(format!(
                "Nuxeo: '{rel}' is not a directory"
            )))
        }
    }

    fn open(&self, rel: &RelPath) -> Result<Box<dyn ContentReader>, SourceError> {
        let resolved = self.resolve_blob(rel)?;
        self.blob_reader(&resolved.doc_path, &resolved.xpath, &resolved.blob)
    }

    fn quick_fingerprint(&self, rel: &RelPath) -> Result<Option<Fingerprint>, SourceError> {
        let resolved = self.resolve_blob(rel)?;
        Ok(resolved
            .blob
            .digest
            .as_deref()
            .map(|d| Fingerprint(d.as_bytes().to_vec())))
    }
}

/// A streaming [`ContentReader`] over a Nuxeo blob. Each [`read_at`](ContentReader::read_at) issues
/// a ranged GET for just that window (the download servlet honours `Range`), so memory stays
/// bounded and the engine's block short-circuit can stop at the first differing block. `as_slice`
/// is `None` on purpose.
///
/// Some servers/proxies ignore `Range` and answer 200 with the WHOLE body. On the first such
/// response the blob is downloaded once to a temp file and all windows are served from disk
/// afterwards — memory stays bounded, and the copy costs the same bytes it would have anyway.
///
/// Streaming reader: valid only while its [`NuxeoSource`]'s runtime is alive (see the
/// `ContentReader` lifetime contract); `read_at` `block_on`s the runtime, so it must run on a
/// synchronous thread.
struct NuxeoContentReader {
    client: reqwest::Client,
    handle: tokio::runtime::Handle,
    url: reqwest::Url,
    len: u64,
    username: String,
    password: String,
    /// Set on the first Range-ignored (200) response: the downloaded blob, serving later windows.
    fallback: Mutex<Option<tempfile::NamedTempFile>>,
}

impl ContentReader for NuxeoContentReader {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError> {
        if buf.is_empty() || offset >= self.len {
            return Ok(0);
        }
        // Once the fallback exists, windows come from disk — no more HTTP round-trips. Poisoning
        // recovery mirrors lock_identities: the file is complete or absent, never half-valid.
        let mut guard = self.fallback.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(file) = guard.as_mut() {
            return read_window(file, offset, buf);
        }
        drop(guard);
        let end = (offset + buf.len() as u64).min(self.len);
        let range = format!("bytes={}-{}", offset, end - 1);
        let expected_len = self.len;
        let client = self.client.clone();
        let url = self.url.clone();
        let username = self.username.clone();
        let password = self.password.clone();
        let result = self.handle.block_on(async move {
            let resp = client
                .get(url)
                .basic_auth(&username, Some(&password))
                .header(reqwest::header::RANGE, range)
                .send()
                .await
                .map_err(|e| SourceError::Other(format!("Nuxeo ranged read: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                return Err(SourceError::Other(format!(
                    "Nuxeo ranged read: HTTP {}",
                    status.as_u16()
                )));
            }
            if status.as_u16() == 206 {
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| SourceError::Other(format!("Nuxeo ranged read: {e}")))?;
                // The VFS contract: an interior read_at must fill its buffer — a short read means
                // EOF. `end` is exactly EOF at the tail, so a 206 body must match `end - offset`
                // exactly; anything less is a truncated transfer, not a shorter file.
                let window_len = end - offset;
                if bytes.len() as u64 != window_len {
                    return Err(SourceError::Other(format!(
                        "Nuxeo ranged read: requested {window_len} bytes, got {}",
                        bytes.len()
                    )));
                }
                buf[..bytes.len()].copy_from_slice(&bytes);
                return Ok((None, bytes.len()));
            }
            // A 200 to a ranged request means the server plays loose with Range. Two shapes seen in
            // the wild (the owner's deployment does BOTH): the WHOLE body (Range truly ignored) or
            // just the requested window re-labelled as 200. Decide by body length: full body → keep
            // it as the one-shot fallback file; window-sized body → serve it directly.
            use std::io::Write;
            let mut file = tempfile::NamedTempFile::new()
                .map_err(|e| SourceError::Other(format!("Nuxeo fallback write: {e}")))?;
            let mut downloaded: u64 = 0;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = futures_util::TryStreamExt::try_next(&mut stream)
                .await
                .map_err(|e| SourceError::Other(format!("Nuxeo fallback read: {e}")))?
            {
                file.as_file_mut()
                    .write_all(&chunk)
                    .map_err(|e| SourceError::Other(format!("Nuxeo fallback write: {e}")))?;
                downloaded += chunk.len() as u64;
            }
            let window_len = end - offset;
            if downloaded == expected_len {
                // Whole blob on disk: byte `offset` of the file is byte `offset` of the blob.
                let n = read_window(&file, offset, buf)?;
                return Ok((Some(file), n));
            }
            if downloaded == window_len {
                // The body IS the requested window: its byte 0 maps to blob offset `offset`.
                let n = read_window(&file, 0, buf)?;
                return Ok((None, n));
            }
            Err(SourceError::Other(format!(
                "Nuxeo blob size mismatch: declared {expected_len}, got {downloaded}"
            )))
        });
        let (fallback, n) = result?;
        if let Some(file) = fallback {
            let mut guard = self.fallback.lock().unwrap_or_else(|p| p.into_inner());
            *guard = Some(file);
        }
        Ok(n)
    }
}

fn read_window(
    file: &tempfile::NamedTempFile,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, SourceError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut handle = file
        .reopen()
        .map_err(|e| SourceError::Other(format!("Nuxeo fallback read: {e}")))?;
    handle
        .seek(SeekFrom::Start(offset))
        .map_err(|e| SourceError::Other(format!("Nuxeo fallback read: {e}")))?;
    handle
        .read(buf)
        .map_err(|e| SourceError::Other(format!("Nuxeo fallback read: {e}")))
}
