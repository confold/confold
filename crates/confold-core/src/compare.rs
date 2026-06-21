//! The comparison pipeline: walk + match both trees, triage, compare contents, assemble the report.

use std::collections::BTreeMap;

use confold_vfs::{EntryMeta, RelPath, Source};
use rayon::prelude::*;

use crate::config::{CompareConfig, CompareMethod};
use crate::content;
use crate::error::EngineError;
use crate::model::{DiffEntry, DiffReport, DiffStatus};

/// The left- and right-side metadata for one name within a directory (either may be absent).
type SidePair = (Option<EntryMeta>, Option<EntryMeta>);

/// A progress callback invoked once per entry examined during a recursive compare. Used by streaming
/// UIs to show a live "examined N items" counter. Called concurrently from rayon worker threads, so it
/// must be `Sync` (e.g. it increments an `AtomicU64` and occasionally emits an event).
pub type ProgressFn<'a> = &'a (dyn Fn() + Sync);

/// Compare two directory trees and produce a [`DiffReport`].
///
/// Both `left` and `right` are treated as the roots to compare. A failure listing either root is fatal
/// (`Err`); failures deeper in the tree become [`DiffStatus::Error`] nodes.
pub fn compare(
    left: &dyn Source,
    right: &dyn Source,
    cfg: &CompareConfig,
) -> Result<DiffReport, EngineError> {
    compare_at(left, right, &RelPath::root(), cfg)
}

/// Like [`compare`], but starts the walk at `start` (a sub-path within both sources) instead of the
/// roots. Lets the lazy UI load one directory level deep into a tree while keeping a single connected
/// source — every child's `rel_path` is reported relative to the source roots (i.e. prefixed by
/// `start`), so the caller never has to re-base paths.
pub fn compare_at(
    left: &dyn Source,
    right: &dyn Source,
    start: &RelPath,
    cfg: &CompareConfig,
) -> Result<DiffReport, EngineError> {
    compare_at_with_progress(left, right, start, cfg, &|| {})
}

/// Like [`compare_at`], but invokes `progress` once per entry examined — lets a streaming UI show a live
/// counter while the (otherwise opaque) recursive compare runs. `progress` runs on rayon worker threads.
pub fn compare_at_with_progress(
    left: &dyn Source,
    right: &dyn Source,
    start: &RelPath,
    cfg: &CompareConfig,
    progress: ProgressFn,
) -> Result<DiffReport, EngineError> {
    let children = compare_directory(left, right, start, cfg, true, progress)?;
    let status = aggregate_status(&children);
    let root = DiffEntry {
        rel_path: start.clone(),
        name: start.file_name().unwrap_or(".").to_owned(),
        is_dir: true,
        status,
        left: None,
        right: None,
        detail: None,
        children,
    };
    let summary = crate::model::Summary::tally(&root);
    Ok(DiffReport { root, summary })
}

/// List one directory level at `start` WITHOUT reading file contents — a fast metadata-only pass for
/// streaming UIs. Files present on both sides come back as `Skipped` / `"comparing"` (a placeholder
/// resolved separately via [`compare_file`] and streamed in); directories present on both sides are
/// `Skipped` / `"not descended"`; items on only one side are fully classified (no content needed).
/// Always non-recursive regardless of `cfg.recursive`.
pub fn list_level(
    left: &dyn Source,
    right: &dyn Source,
    start: &RelPath,
    cfg: &CompareConfig,
) -> Result<DiffReport, EngineError> {
    let children = compare_directory(left, right, start, cfg, false, &|| {})?;
    let status = aggregate_status(&children);
    let root = DiffEntry {
        rel_path: start.clone(),
        name: start.file_name().unwrap_or(".").to_owned(),
        is_dir: true,
        status,
        left: None,
        right: None,
        detail: None,
        children,
    };
    let summary = crate::model::Summary::tally(&root);
    Ok(DiffReport { root, summary })
}

/// Compute the verdict for a single regular file present on both sides — used by streaming UIs to
/// resolve a [`list_level`] entry's `"comparing"` placeholder. `lm`/`rm` are the file's metadata.
pub fn compare_file(
    left: &dyn Source,
    right: &dyn Source,
    lm: &EntryMeta,
    rm: &EntryMeta,
    rel: &RelPath,
    cfg: &CompareConfig,
) -> (DiffStatus, Option<String>) {
    compare_files(left, right, lm, rm, rel, cfg)
}

/// Compare the contents of a directory present on both sides; returns its child nodes. When
/// `read_files` is false, regular files on both sides are left as `Skipped` / `"comparing"` instead of
/// being read (the listing pass for streaming); subdirectories are then never descended.
fn compare_directory(
    left: &dyn Source,
    right: &dyn Source,
    rel: &RelPath,
    cfg: &CompareConfig,
    read_files: bool,
    progress: ProgressFn,
) -> Result<Vec<DiffEntry>, EngineError> {
    let left_entries = left.read_dir(rel)?;
    let right_entries = right.read_dir(rel)?;

    // Union by name; BTreeMap gives deterministic ordering.
    let mut by_name: BTreeMap<String, SidePair> = BTreeMap::new();
    for entry in left_entries {
        let name = entry.name.clone();
        by_name.entry(name).or_default().0 = Some(entry);
    }
    for entry in right_entries {
        let name = entry.name.clone();
        by_name.entry(name).or_default().1 = Some(entry);
    }
    let items: Vec<(String, SidePair)> = by_name.into_iter().collect();

    let children = items
        .par_iter()
        .map(|(name, (l, r))| {
            compare_entry(left, right, rel, name, l.as_ref(), r.as_ref(), cfg, read_files, progress)
        })
        .collect();
    Ok(children)
}

/// Classify a single named entry that may be present on the left, the right, or both.
#[allow(clippy::too_many_arguments)]
fn compare_entry(
    left: &dyn Source,
    right: &dyn Source,
    parent: &RelPath,
    name: &str,
    lm: Option<&EntryMeta>,
    rm: Option<&EntryMeta>,
    cfg: &CompareConfig,
    read_files: bool,
    progress: ProgressFn,
) -> DiffEntry {
    let rel = parent.child(name);
    let is_dir = lm.or(rm).map(|m| m.kind.is_dir()).unwrap_or(false);
    progress(); // count every entry examined (files + dirs, any side) for the live progress counter

    if cfg.filters.is_excluded(&rel, is_dir) {
        return leaf(
            rel,
            name,
            is_dir,
            DiffStatus::Skipped,
            lm,
            rm,
            Some("filtered".to_owned()),
        );
    }

    match (lm, rm) {
        (Some(l), Some(r)) => {
            if l.kind == confold_vfs::EntryKind::Symlink
                || r.kind == confold_vfs::EntryKind::Symlink
            {
                return leaf(
                    rel,
                    name,
                    is_dir,
                    DiffStatus::Skipped,
                    lm,
                    rm,
                    Some("symlink (not followed)".to_owned()),
                );
            }
            match (l.kind.is_dir(), r.kind.is_dir()) {
                (true, true) => {
                    if cfg.recursive && read_files {
                        match compare_directory(left, right, &rel, cfg, read_files, progress) {
                            Ok(children) => {
                                let status = aggregate_status(&children);
                                dir_node(rel, name, status, lm, rm, None, children)
                            }
                            Err(e) => leaf(
                                rel,
                                name,
                                true,
                                DiffStatus::Error,
                                lm,
                                rm,
                                Some(e.to_string()),
                            ),
                        }
                    } else {
                        leaf(
                            rel,
                            name,
                            true,
                            DiffStatus::Skipped,
                            lm,
                            rm,
                            Some("not descended".to_owned()),
                        )
                    }
                }
                (false, false) => {
                    let (status, detail) = if read_files {
                        compare_files(left, right, l, r, &rel, cfg)
                    } else {
                        // Listing pass: defer the verdict — streamed in later via compare_file.
                        (DiffStatus::Skipped, Some("comparing".to_owned()))
                    };
                    DiffEntry {
                        rel_path: rel,
                        name: name.to_owned(),
                        is_dir: false,
                        status,
                        left: Some(l.clone()),
                        right: Some(r.clone()),
                        detail,
                        children: Vec::new(),
                    }
                }
                _ => leaf(
                    rel,
                    name,
                    is_dir,
                    DiffStatus::Different,
                    lm,
                    rm,
                    Some("type differs (file vs directory)".to_owned()),
                ),
            }
        }
        (Some(l), None) => collect_one_side(left, &rel, name, l, Side::Left, cfg),
        (None, Some(r)) => collect_one_side(right, &rel, name, r, Side::Right, cfg),
        (None, None) => unreachable!("an entry is always present on at least one side"),
    }
}

/// Compare two files (both sides are regular files) per the configured method.
fn compare_files(
    left: &dyn Source,
    right: &dyn Source,
    l: &EntryMeta,
    r: &EntryMeta,
    rel: &RelPath,
    cfg: &CompareConfig,
) -> (DiffStatus, Option<String>) {
    let size_eq = l.size == r.size;
    let mtime_eq = l.mtime == r.mtime;

    match cfg.method {
        CompareMethod::Size => binary_verdict(size_eq, "size differs"),
        CompareMethod::Mtime => binary_verdict(mtime_eq, "modified time differs"),
        CompareMethod::SizeAndMtime => {
            binary_verdict(size_eq && mtime_eq, "size or modified time differs")
        }
        CompareMethod::Full | CompareMethod::Quick { .. } => {
            if !size_eq {
                return (DiffStatus::Different, Some("size differs".to_owned()));
            }
            if l.size == 0 {
                return (DiffStatus::Identical, None);
            }
            let la = match left.open(rel) {
                Ok(reader) => reader,
                Err(e) => return (DiffStatus::Error, Some(e.to_string())),
            };
            let rb = match right.open(rel) {
                Ok(reader) => reader,
                Err(e) => return (DiffStatus::Error, Some(e.to_string())),
            };
            let binary = content::is_binary(la.as_ref()) || content::is_binary(rb.as_ref());
            let sampled = matches!(cfg.method, CompareMethod::Quick { large_file_threshold }
                if l.size > large_file_threshold);
            let result = if sampled {
                content::quick_equal(la.as_ref(), rb.as_ref())
            } else {
                content::full_equal(la.as_ref(), rb.as_ref())
            };
            match result {
                Ok(true) => (DiffStatus::Identical, identical_detail(binary, sampled)),
                Ok(false) => (DiffStatus::Different, Some(diff_detail(binary))),
                Err(e) => (DiffStatus::Error, Some(e.to_string())),
            }
        }
    }
}

fn binary_verdict(equal: bool, diff_msg: &str) -> (DiffStatus, Option<String>) {
    if equal {
        (DiffStatus::Identical, None)
    } else {
        (DiffStatus::Different, Some(diff_msg.to_owned()))
    }
}

fn identical_detail(binary: bool, sampled: bool) -> Option<String> {
    match (binary, sampled) {
        (false, false) => None,
        (true, false) => Some("binary".to_owned()),
        (false, true) => Some("identical by sampling (large file)".to_owned()),
        (true, true) => Some("binary; identical by sampling (large file)".to_owned()),
    }
}

fn diff_detail(binary: bool) -> String {
    if binary {
        "binary content differs".to_owned()
    } else {
        "content differs".to_owned()
    }
}

/// Which single side a unique item belongs to.
#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

impl Side {
    fn status(self) -> DiffStatus {
        match self {
            Side::Left => DiffStatus::LeftOnly,
            Side::Right => DiffStatus::RightOnly,
        }
    }
}

/// Build the node for an item present on exactly one side, enumerating a unique directory's subtree
/// (when recursive) so the report fully lists what exists on only that side.
fn collect_one_side(
    src: &dyn Source,
    rel: &RelPath,
    name: &str,
    meta: &EntryMeta,
    side: Side,
    cfg: &CompareConfig,
) -> DiffEntry {
    let (left, right) = match side {
        Side::Left => (Some(meta.clone()), None),
        Side::Right => (None, Some(meta.clone())),
    };
    let mut children = Vec::new();
    if meta.kind.is_dir() && cfg.recursive {
        match src.read_dir(rel) {
            Ok(mut entries) => {
                entries.sort_by(|a, b| a.name.cmp(&b.name));
                children = entries
                    .par_iter()
                    .map(|e| collect_one_side(src, &e.rel_path, &e.name, e, side, cfg))
                    .collect();
            }
            Err(e) => {
                return DiffEntry {
                    rel_path: rel.clone(),
                    name: name.to_owned(),
                    is_dir: true,
                    status: DiffStatus::Error,
                    left,
                    right,
                    detail: Some(e.to_string()),
                    children: Vec::new(),
                };
            }
        }
    }
    DiffEntry {
        rel_path: rel.clone(),
        name: name.to_owned(),
        is_dir: meta.kind.is_dir(),
        status: side.status(),
        left,
        right,
        detail: None,
        children,
    }
}

/// A directory's status is divergent if any direct child diverges, else identical.
fn aggregate_status(children: &[DiffEntry]) -> DiffStatus {
    if children.iter().any(|c| c.status.is_divergent()) {
        DiffStatus::Different
    } else {
        DiffStatus::Identical
    }
}

/// Construct a childless node.
fn leaf(
    rel: RelPath,
    name: &str,
    is_dir: bool,
    status: DiffStatus,
    lm: Option<&EntryMeta>,
    rm: Option<&EntryMeta>,
    detail: Option<String>,
) -> DiffEntry {
    DiffEntry {
        rel_path: rel,
        name: name.to_owned(),
        is_dir,
        status,
        left: lm.cloned(),
        right: rm.cloned(),
        detail,
        children: Vec::new(),
    }
}

/// Construct a directory node with children.
#[allow(clippy::too_many_arguments)]
fn dir_node(
    rel: RelPath,
    name: &str,
    status: DiffStatus,
    lm: Option<&EntryMeta>,
    rm: Option<&EntryMeta>,
    detail: Option<String>,
    children: Vec<DiffEntry>,
) -> DiffEntry {
    DiffEntry {
        rel_path: rel,
        name: name.to_owned(),
        is_dir: true,
        status,
        left: lm.cloned(),
        right: rm.cloned(),
        detail,
        children,
    }
}
