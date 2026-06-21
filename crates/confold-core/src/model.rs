//! The comparison result model.

use confold_vfs::{EntryMeta, RelPath};
use serde::{Deserialize, Serialize};

/// Classification of a single compared item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    /// Present on both sides and equal under the chosen method.
    Identical,
    /// Present on both sides but not equal.
    Different,
    /// Present only on the left side.
    LeftOnly,
    /// Present only on the right side.
    RightOnly,
    /// Not compared (filtered out, a symlink, or a non-descended directory).
    Skipped,
    /// Could not be compared due to an error (e.g. unreadable).
    Error,
}

impl DiffStatus {
    /// `true` if this status represents an actual divergence between the two sides.
    pub fn is_divergent(self) -> bool {
        matches!(
            self,
            DiffStatus::Different
                | DiffStatus::LeftOnly
                | DiffStatus::RightOnly
                | DiffStatus::Error
        )
    }
}

/// One node in the comparison tree (a file or a directory).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffEntry {
    /// Path relative to the compared roots.
    pub rel_path: RelPath,
    /// The item's own name (final path component).
    pub name: String,
    /// Whether this node is a directory.
    pub is_dir: bool,
    /// How the item was classified.
    pub status: DiffStatus,
    /// Left-side metadata, if present.
    pub left: Option<EntryMeta>,
    /// Right-side metadata, if present.
    pub right: Option<EntryMeta>,
    /// Optional human-readable note (e.g. `"size differs"`, `"identical by sampling"`, an error message).
    pub detail: Option<String>,
    /// Child nodes (directories only; empty for files).
    pub children: Vec<DiffEntry>,
}

/// Per-status counts over all nodes in the tree (excluding the synthetic root).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Count of identical items.
    pub identical: u64,
    /// Count of differing items.
    pub different: u64,
    /// Count of items present only on the left.
    pub left_only: u64,
    /// Count of items present only on the right.
    pub right_only: u64,
    /// Count of skipped items.
    pub skipped: u64,
    /// Count of errored items.
    pub errored: u64,
}

impl Summary {
    /// Tally all descendants of `root` (the root node itself is not counted).
    pub fn tally(root: &DiffEntry) -> Summary {
        let mut summary = Summary::default();
        for child in &root.children {
            summary.record(child);
        }
        summary
    }

    fn record(&mut self, entry: &DiffEntry) {
        match entry.status {
            DiffStatus::Identical => self.identical += 1,
            DiffStatus::Different => self.different += 1,
            DiffStatus::LeftOnly => self.left_only += 1,
            DiffStatus::RightOnly => self.right_only += 1,
            DiffStatus::Skipped => self.skipped += 1,
            DiffStatus::Error => self.errored += 1,
        }
        for child in &entry.children {
            self.record(child);
        }
    }
}

/// The full result of a comparison run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffReport {
    /// The root node (the compared roots themselves); its `children` are the top-level items.
    pub root: DiffEntry,
    /// Per-status counts.
    pub summary: Summary,
}

impl DiffReport {
    /// `true` if any item diverged (different / unique to one side / errored).
    pub fn has_differences(&self) -> bool {
        let s = &self.summary;
        s.different + s.left_only + s.right_only + s.errored > 0
    }
}
