//! Integration tests for sync actions, including a round-trip against the compare engine.

use std::fs;
use std::path::Path;

use confold_core::{compare, CompareConfig};
use confold_sync::{apply, apply_mixed, SyncAction, SyncOp};
use confold_vfs::{LocalSource, RelPath};

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn rel(parts: &[&str]) -> RelPath {
    let mut r = RelPath::root();
    for p in parts {
        r = r.child(p);
    }
    r
}

#[test]
fn copy_left_to_right_makes_trees_identical() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "a.txt", b"alpha");
    write(l.path(), "sub/b.txt", b"beta");
    write(l.path(), "sub/deep/c.txt", b"gamma");

    let actions = vec![
        SyncAction {
            rel_path: rel(&["a.txt"]),
            op: SyncOp::CopyLeftToRight,
            is_dir: false,
        },
        SyncAction {
            rel_path: rel(&["sub"]),
            op: SyncOp::CopyLeftToRight,
            is_dir: true,
        },
    ];
    let outcomes = apply(
        &LocalSource::new(l.path()),
        &LocalSource::new(r.path()),
        &actions,
        false,
    );
    assert!(
        outcomes.iter().all(|o| o.ok),
        "all ops succeed: {outcomes:?}"
    );

    // The two trees must now be identical per the compare engine.
    let report = compare(
        &LocalSource::new(l.path()),
        &LocalSource::new(r.path()),
        &CompareConfig::default(),
    )
    .unwrap();
    assert!(!report.has_differences(), "trees identical after copy");
}

#[test]
fn dry_run_writes_nothing() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "a.txt", b"alpha");

    let actions = vec![SyncAction {
        rel_path: rel(&["a.txt"]),
        op: SyncOp::CopyLeftToRight,
        is_dir: false,
    }];
    let outcomes = apply(
        &LocalSource::new(l.path()),
        &LocalSource::new(r.path()),
        &actions,
        true,
    );

    assert!(outcomes.iter().all(|o| o.ok));
    assert!(!r.path().join("a.txt").exists(), "dry run must not write");
}

#[test]
fn delete_right_removes_recursively() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(r.path(), "junk/x.txt", b"x");
    write(r.path(), "junk/y.txt", b"y");

    let actions = vec![SyncAction {
        rel_path: rel(&["junk"]),
        op: SyncOp::DeleteRight,
        is_dir: true,
    }];
    let outcomes = apply(
        &LocalSource::new(l.path()),
        &LocalSource::new(r.path()),
        &actions,
        false,
    );

    assert!(outcomes.iter().all(|o| o.ok));
    assert!(!r.path().join("junk").exists(), "directory removed");
}

#[test]
fn overlapping_dir_and_child_selection_copies_cleanly() {
    // Reproduces "select a folder AND its content, then copy" — the dir copy already handles the child
    // recursively, and the explicit child action must not break it.
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "d/a.txt", b"A");
    write(l.path(), "d/sub/b.txt", b"B");

    // Both the directory and one of its children are selected (overlap).
    let actions = vec![
        SyncAction {
            rel_path: rel(&["d"]),
            op: SyncOp::CopyLeftToRight,
            is_dir: true,
        },
        SyncAction {
            rel_path: rel(&["d", "a.txt"]),
            op: SyncOp::CopyLeftToRight,
            is_dir: false,
        },
    ];
    let outcomes = apply(
        &LocalSource::new(l.path()),
        &LocalSource::new(r.path()),
        &actions,
        false,
    );

    let failed: Vec<_> = outcomes.iter().filter(|o| !o.ok).collect();
    assert!(failed.is_empty(), "no op should fail: {failed:?}");

    let report = compare(
        &LocalSource::new(l.path()),
        &LocalSource::new(r.path()),
        &CompareConfig::default(),
    )
    .unwrap();
    assert!(
        !report.has_differences(),
        "trees identical after overlapping copy"
    );
}

#[test]
fn overlapping_dir_and_child_delete_succeeds() {
    // Deleting a directory AND a child of it: the recursive dir delete removes the child, so the explicit
    // child delete would hit "not found" unless overlapping actions are deduped.
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(r.path(), "d/a.txt", b"A");
    write(r.path(), "d/b.txt", b"B");

    let actions = vec![
        SyncAction {
            rel_path: rel(&["d"]),
            op: SyncOp::DeleteRight,
            is_dir: true,
        },
        SyncAction {
            rel_path: rel(&["d", "a.txt"]),
            op: SyncOp::DeleteRight,
            is_dir: false,
        },
    ];
    let outcomes = apply(
        &LocalSource::new(l.path()),
        &LocalSource::new(r.path()),
        &actions,
        false,
    );

    let failed: Vec<_> = outcomes.iter().filter(|o| !o.ok).collect();
    assert!(failed.is_empty(), "no op should fail: {failed:?}");
    assert!(!r.path().join("d").exists(), "directory removed");
}

// ── Read-only origins (e.g. Nuxeo): a side only needs SourceMut when an op writes to it ─────────

/// Delegating wrapper that is a `Source` but deliberately NOT `SourceMut` — the type-level shape of
/// a read-only backend.
struct ReadOnly<'a>(&'a LocalSource);

impl confold_vfs::Source for ReadOnly<'_> {
    fn capabilities(&self) -> confold_vfs::Capabilities {
        confold_vfs::Capabilities {
            list: true,
            read: true,
            fingerprint: false,
            write: false,
        }
    }
    fn read_dir(
        &self,
        rel: &RelPath,
    ) -> Result<Vec<confold_vfs::EntryMeta>, confold_vfs::SourceError> {
        self.0.read_dir(rel)
    }
    fn open(
        &self,
        rel: &RelPath,
    ) -> Result<Box<dyn confold_vfs::ContentReader>, confold_vfs::SourceError> {
        self.0.open(rel)
    }
}

#[test]
fn read_only_origin_can_be_copied_from_but_not_written_to() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "doc.txt", b"alpha");

    let left = ReadOnly(&LocalSource::new(l.path()));
    let right = LocalSource::new(r.path());
    let actions = vec![SyncAction {
        rel_path: rel(&["doc.txt"]),
        op: SyncOp::CopyLeftToRight,
        is_dir: false,
    }];

    // Copy FROM the read-only side works: only the destination must be mutable.
    let outcomes = apply_mixed(&left, &right, None, Some(&right), &actions, false);
    assert!(outcomes.iter().all(|o| o.ok), "copy out: {outcomes:?}");
    assert_eq!(fs::read(r.path().join("doc.txt")).unwrap(), b"alpha");

    // The reverse copy (writes the read-only side) fails per-op, not per-batch.
    let back = vec![SyncAction {
        rel_path: rel(&["doc.txt"]),
        op: SyncOp::CopyRightToLeft,
        is_dir: false,
    }];
    let outcomes = apply_mixed(&left, &right, None, Some(&right), &back, false);
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].ok);
    assert!(outcomes[0].error.as_deref().unwrap().contains("read-only"));

    // So does deleting on the read-only side.
    let del = vec![SyncAction {
        rel_path: rel(&["doc.txt"]),
        op: SyncOp::DeleteLeft,
        is_dir: false,
    }];
    let outcomes = apply_mixed(&left, &right, None, Some(&right), &del, false);
    assert!(!outcomes[0].ok);
    assert!(
        fs::exists(l.path().join("doc.txt")).unwrap(),
        "origin untouched"
    );
}

#[test]
fn dry_run_previews_without_any_mutable_handle() {
    let l = tempfile::tempdir().unwrap();
    let r = tempfile::tempdir().unwrap();
    write(l.path(), "doc.txt", b"alpha");

    let left = ReadOnly(&LocalSource::new(l.path()));
    let right = LocalSource::new(r.path());
    let actions = vec![
        SyncAction {
            rel_path: rel(&["doc.txt"]),
            op: SyncOp::CopyLeftToRight,
            is_dir: false,
        },
        SyncAction {
            rel_path: rel(&["other.txt"]),
            op: SyncOp::DeleteRight,
            is_dir: false,
        },
    ];
    let outcomes = apply_mixed(&left, &right, None, None, &actions, true);
    assert!(outcomes.iter().all(|o| o.ok), "preview: {outcomes:?}");
    assert!(!r.path().join("doc.txt").exists(), "dry run wrote nothing");
    assert!(fs::exists(l.path().join("doc.txt")).unwrap());
}
