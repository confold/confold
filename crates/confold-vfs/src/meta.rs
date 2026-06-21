//! Entry metadata and content fingerprints.

use serde::{Deserialize, Serialize};

use crate::path::RelPath;

/// Kind of a tree entry, as reported without following symlinks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link (the link itself, not its target).
    Symlink,
}

impl EntryKind {
    /// `true` if this entry is a directory.
    pub fn is_dir(self) -> bool {
        matches!(self, EntryKind::Dir)
    }
}

/// Lightweight metadata for one tree entry — obtained without reading file contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryMeta {
    /// The entry's own name (final path component).
    pub name: String,
    /// Path relative to the compared root — the match key between sources.
    pub rel_path: RelPath,
    /// File / directory / symlink.
    pub kind: EntryKind,
    /// Size in bytes (0 for directories).
    pub size: u64,
    /// Last-modified time as Unix epoch milliseconds, if the backend exposes it. A stable, language-
    /// neutral wire format (the JSON contract consumed by frontends).
    pub mtime: Option<i64>,
    /// Creation time as Unix epoch milliseconds, if the OS/backend exposes it (macOS/Windows local;
    /// commonly unavailable on Linux and over SFTP). `None` when unknown.
    pub created: Option<i64>,
}

/// A cheap content fingerprint a backend may expose to avoid transferring bytes (e.g. an S3 ETag).
///
/// Phase-1 [`LocalSource`](crate::LocalSource) returns `None`; this is the seam that lets network
/// backends do metadata/ETag-first triage later without the engine changing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint(pub Vec<u8>);
