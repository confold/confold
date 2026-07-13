//! Confold desktop backend (Tauri). Exposes the compare engine (`confold-core`) to the web UI.

mod apply;
mod diff;
mod plan;
mod recents;
mod scan;
mod shell;
mod sources;

// ── External crate imports (re-exported to submodules via `use super::*`) ───────────────────────
#[allow(unused_imports)]
use confold_core::{
    compare as engine_compare, compare_at_with_progress as engine_compare_with_progress,
    compare_file as engine_compare_file, full_equal, list_level as engine_list_level,
    Capabilities, CompareConfig, CompareMethod, ContentReader, DiffEntry, DiffReport, DiffStatus,
    EntryMeta, FilterSet, LocalSource, RelPath, Source, SourceError, SourceMut,
    DEFAULT_LARGE_FILE_THRESHOLD,
};
#[allow(unused_imports)]
use confold_s3::{S3Config, S3Source};
#[allow(unused_imports)]
use confold_sftp::{SftpAuth, SftpConfig, SftpSource};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::{BTreeMap, HashMap};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicU64, Ordering};
#[allow(unused_imports)]
use std::sync::{Arc, LazyLock, Mutex};
#[allow(unused_imports)]
use tauri::{AppHandle, Emitter, State};
#[allow(unused_imports)]
use confold_sync::{apply as sync_apply, ActionOutcome, SyncAction, SyncOp};
#[allow(unused_imports)]
use confold_textdiff::{diff_text, diff_hunks, is_binary_bytes, FileDiff, FileDiffHunks};

// ── Event name constants — shared across modules ────────────────────────────────────────────────
pub(crate) const ENTRY_RESOLVED: &str = "entry-resolved";
pub(crate) const MIGRATE_PROGRESS: &str = "migrate-progress";
pub(crate) const MIGRATE_DONE: &str = "migrate-done";
pub(crate) const MIGRATE_PHASE: &str = "migrate-phase";
pub(crate) const PLAN_PROGRESS: &str = "plan-progress";
pub(crate) const PLAN_READY: &str = "plan-ready";


#[cfg(test)]
#[allow(unused_imports)]
use {apply::*, diff::*, plan::*, scan::*, sources::*};


use scan::{AppState, compare, compare_level, test_source};
use plan::{migrate_actions, sync_actions};
use apply::{migrate_apply, migrate_cancel, plan_actions, apply_actions};
use diff::{diff_file, diff_file_large, diff_strings, save_file, hex_compare, read_bytes, source_types};
use shell::{install_shell_integration, uninstall_shell_integration, shell_integration_status};
use recents::{load_recents, save_recents, path_exists};

pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}));
        builder = builder.plugin(tauri_plugin_deep_link::init());
    }

    builder
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            compare, compare_level, test_source,
            migrate_actions, sync_actions,
            migrate_apply, migrate_cancel, plan_actions, apply_actions,
            diff_file, diff_file_large, diff_strings, save_file, hex_compare, read_bytes, source_types,
            install_shell_integration, uninstall_shell_integration, shell_integration_status,
            load_recents, save_recents, path_exists,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// An `fs` `SourceSpec` rooted at `root` (test helper).
    fn fs_spec(root: String) -> SourceSpec {
        SourceSpec {
            kind: "fs".to_string(),
            fields: BTreeMap::from([("root".to_string(), root)]),
        }
    }

    /// A `FileRef` for a file named `name` in directory `dir` (an `fs` source rooted at `dir`).
    fn fref(dir: &Path, name: &str) -> FileRef {
        FileRef {
            source: fs_spec(dir.to_str().unwrap().to_string()),
            rel: name.to_string(),
        }
    }

    #[test]
    fn migrate_actions_collects_per_flags() {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        // only in origin (a file and a whole subtree)
        fs::write(l.path().join("new.txt"), b"x").unwrap();
        fs::create_dir(l.path().join("newdir")).unwrap();
        fs::write(l.path().join("newdir").join("a.txt"), b"a").unwrap();
        // different content on both sides
        fs::write(l.path().join("diff.txt"), b"left").unwrap();
        fs::write(r.path().join("diff.txt"), b"right!").unwrap();
        // only in destination
        fs::write(r.path().join("extra.txt"), b"e").unwrap();
        // identical
        fs::write(l.path().join("same.txt"), b"s").unwrap();
        fs::write(r.path().join("same.txt"), b"s").unwrap();

        let cfg = CompareConfig {
            method: CompareMethod::Full,
            recursive: true,
            filters: FilterSet::default(),
        };
        let report =
            engine_compare(&LocalSource::new(l.path()), &LocalSource::new(r.path()), &cfg).unwrap();

        let key = |a: &MigrateAction| a.rel_path.join("/");
        let collect = |flags: MigrateFlags| {
            let mut v = Vec::new();
            collect_migrate(&report.root, flags, &mut v);
            v
        };

        // All categories enabled.
        let all = collect(MigrateFlags {
            copy_new: true,
            overwrite_different: true,
            delete_extra: true,
        });
        let mut got: Vec<(String, MigrateReason, bool)> =
            all.iter().map(|a| (key(a), a.reason, a.is_dir)).collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            vec![
                ("diff.txt".to_string(), MigrateReason::Different, false),
                ("extra.txt".to_string(), MigrateReason::Extra, false),
                ("new.txt".to_string(), MigrateReason::New, false),
                ("newdir".to_string(), MigrateReason::New, true),
            ]
        );
        // The new subtree is ONE recursive copy action — its child is not enumerated separately.
        assert!(!all.iter().any(|a| key(a) == "newdir/a.txt"));
        // Identical items are never touched.
        assert!(!all.iter().any(|a| key(a) == "same.txt"));

        // copy_new only → just the origin-only items.
        let copy_only = collect(MigrateFlags {
            copy_new: true,
            overwrite_different: false,
            delete_extra: false,
        });
        let mut paths: Vec<String> = copy_only.iter().map(&key).collect();
        paths.sort();
        assert_eq!(paths, vec!["new.txt", "newdir"]);

        // delete_extra only → just the destination-only item, as a delete-on-right.
        let del_only = collect(MigrateFlags {
            copy_new: false,
            overwrite_different: false,
            delete_extra: true,
        });
        assert_eq!(del_only.len(), 1);
        assert_eq!(key(&del_only[0]), "extra.txt");
        assert_eq!(del_only[0].reason, MigrateReason::Extra);

        // Everything off → no actions.
        assert!(collect(MigrateFlags {
            copy_new: false,
            overwrite_different: false,
            delete_extra: false,
        })
        .is_empty());
    }

    #[test]
    fn run_migrate_applies_reports_and_cancels() {
        use std::cell::Cell;

        // Origin with two files and a subtree; destination empty.
        let l = tempfile::tempdir().unwrap();
        let actions = vec![
            MigrateAction { rel_path: vec!["a.txt".to_string()], op: SyncOp::CopyLeftToRight, is_dir: false, reason: MigrateReason::New, item_count: 1, leaves: vec![] },
            MigrateAction { rel_path: vec!["b.txt".to_string()], op: SyncOp::CopyLeftToRight, is_dir: false, reason: MigrateReason::New, item_count: 1, leaves: vec![] },
            // Directory with pre-expanded leaves (the c.txt file inside).
            MigrateAction { rel_path: vec!["d".to_string()], op: SyncOp::CopyLeftToRight, is_dir: true, reason: MigrateReason::New, item_count: 2,
                leaves: vec![LeafOp { rel_path: vec!["d".to_string(), "c.txt".to_string()], is_dir: false }] },
        ];
        fs::write(l.path().join("a.txt"), b"A").unwrap();
        fs::write(l.path().join("b.txt"), b"B").unwrap();
        fs::create_dir(l.path().join("d")).unwrap();
        fs::write(l.path().join("d").join("c.txt"), b"C").unwrap();
        let left = LocalSource::new(l.path());

        // No cancellation: everything applies; the directory action expands into per-descendant outcomes.
        let r = tempfile::tempdir().unwrap();
        let right = LocalSource::new(r.path());
        let mut seen: Vec<(String, bool)> = Vec::new();
        let summary = run_migrate(&left, &right, &actions, || false, |o, _reason| {
            seen.push((o.rel_path.components().join("/"), o.ok))
        });
        assert!(!summary.cancelled);
        assert_eq!(summary.failed, 0);
        assert_eq!(fs::read(r.path().join("a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(r.path().join("d").join("c.txt")).unwrap(), b"C");
        // Per-file granularity: the directory's child shows up as its own outcome.
        assert!(seen.iter().any(|(p, ok)| p == "d/c.txt" && *ok));

        // Cancel after the first action is applied: only a.txt lands; b.txt and d/ are skipped.
        let r2 = tempfile::tempdir().unwrap();
        let right2 = LocalSource::new(r2.path());
        let applied = Cell::new(0usize);
        let summary2 = run_migrate(
            &left,
            &right2,
            &actions,
            || applied.get() >= 1, // polled BEFORE each action
            |_, _reason| applied.set(applied.get() + 1),
        );
        assert!(summary2.cancelled);
        assert!(r2.path().join("a.txt").exists());
        assert!(!r2.path().join("b.txt").exists());
        assert!(!r2.path().join("d").exists());
    }

    #[test]
    fn run_migrate_reports_per_item_failure_and_keeps_going() {
        // The whole point of M1's per-item apply: one action failing must NOT abort the run, must be
        // tallied as `failed`, and must surface its error to the progress callback — while the other
        // actions still apply. We force a deterministic failure with a copy whose source is missing
        // (open fails → ok:false), portable across platforms (no permission/chmod tricks).
        let l = tempfile::tempdir().unwrap();
        fs::write(l.path().join("ok1.txt"), b"1").unwrap();
        fs::write(l.path().join("ok2.txt"), b"2").unwrap();
        // NB: "ghost.txt" is intentionally NOT created on the left.
        let left = LocalSource::new(l.path());
        let r = tempfile::tempdir().unwrap();
        let right = LocalSource::new(r.path());

        let new = |name: &str| MigrateAction {
            rel_path: vec![name.to_string()],
            op: SyncOp::CopyLeftToRight,
            is_dir: false,
            reason: MigrateReason::New,
            item_count: 1,
            leaves: vec![],
        };
        // Failure is in the MIDDLE, so we also prove the run continues past it.
        let actions = vec![new("ok1.txt"), new("ghost.txt"), new("ok2.txt")];

        let mut outcomes: Vec<(String, bool, Option<String>)> = Vec::new();
        let summary = run_migrate(&left, &right, &actions, || false, |o, _reason| {
            outcomes.push((
                o.rel_path.components().join("/"),
                o.ok,
                o.error.clone(),
            ))
        });

        // Tally: three attempted, two ok, one failed, not cancelled.
        assert!(!summary.cancelled);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.ok, 2);
        assert_eq!(summary.failed, 1);

        // The failed item is reported with ok:false AND a non-empty error string.
        let ghost = outcomes.iter().find(|(p, ..)| p == "ghost.txt").unwrap();
        assert!(!ghost.1, "ghost.txt should have failed");
        assert!(ghost.2.as_deref().is_some_and(|e| !e.is_empty()), "expected an error message");

        // The run kept going: the action AFTER the failure still applied to the destination.
        assert_eq!(fs::read(r.path().join("ok1.txt")).unwrap(), b"1");
        assert_eq!(fs::read(r.path().join("ok2.txt")).unwrap(), b"2");
        assert!(!r.path().join("ghost.txt").exists());
    }

    #[test]
    fn run_migrate_cancels_within_directory_copy() {
        // A single directory action with two files inside. Cancelling after the first leaf means the
        // second file is NOT copied — per-file cancellation works inside a directory.
        let l = tempfile::tempdir().unwrap();
        fs::create_dir(l.path().join("d")).unwrap();
        fs::write(l.path().join("d").join("f1.txt"), b"F1").unwrap();
        fs::write(l.path().join("d").join("f2.txt"), b"F2").unwrap();
        let left = LocalSource::new(l.path());
        let r = tempfile::tempdir().unwrap();
        let right = LocalSource::new(r.path());

        let actions = vec![MigrateAction {
            rel_path: vec!["d".to_string()],
            op: SyncOp::CopyLeftToRight,
            is_dir: true,
            reason: MigrateReason::New,
            item_count: 2,
            leaves: vec![
                LeafOp { rel_path: vec!["d".to_string(), "f1.txt".to_string()], is_dir: false },
                LeafOp { rel_path: vec!["d".to_string(), "f2.txt".to_string()], is_dir: false },
            ],
        }];

        let applied = std::cell::Cell::new(0usize);
        let summary = run_migrate(
            &left,
            &right,
            &actions,
            || applied.get() >= 1, // cancel after the first leaf
            |_, _| applied.set(applied.get() + 1),
        );
        assert!(summary.cancelled);
        // First leaf landed; second was skipped.
        assert!(r.path().join("d").join("f1.txt").exists());
        assert!(!r.path().join("d").join("f2.txt").exists());
    }

    #[test]
    fn run_migrate_cancels_within_directory_delete() {
        // A single delete action with two files. Cancelling after the first means the second survives.
        let r = tempfile::tempdir().unwrap();
        fs::create_dir(r.path().join("d")).unwrap();
        fs::write(r.path().join("d").join("f1.txt"), b"F1").unwrap();
        fs::write(r.path().join("d").join("f2.txt"), b"F2").unwrap();
        let left  = LocalSource::new(r.path()); // unused for delete but required by signature
        let right = LocalSource::new(r.path());

        let actions = vec![MigrateAction {
            rel_path: vec!["d".to_string()],
            op: SyncOp::DeleteRight,
            is_dir: true,
            reason: MigrateReason::Extra,
            item_count: 2,
            leaves: vec![
                LeafOp { rel_path: vec!["d".to_string(), "f1.txt".to_string()], is_dir: false },
                LeafOp { rel_path: vec!["d".to_string(), "f2.txt".to_string()], is_dir: false },
                LeafOp { rel_path: vec!["d".to_string()], is_dir: true },
            ],
        }];

        let applied = std::cell::Cell::new(0usize);
        let summary = run_migrate(
            &left, &right, &actions,
            || applied.get() >= 1,
            |_, _| applied.set(applied.get() + 1),
        );
        assert!(summary.cancelled);
        assert!(!r.path().join("d").join("f1.txt").exists()); // first file deleted
        assert!(r.path().join("d").join("f2.txt").exists());  // second survived
        assert!(r.path().join("d").exists());                 // dir still there (not yet deleted)
    }

    // ── M2: move semantics (delete origin after verified-complete migration) ─────────────────────

    /// Full compare of two `fs` roots with the given excludes — mimics the M2 re-verification pass.
    fn reverify(left: &Path, right: &Path, exclude: &[&str]) -> DiffReport {
        let exclude: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
        let cfg = CompareConfig {
            method: CompareMethod::Full,
            recursive: true,
            filters: FilterSet::new(&[], &exclude).unwrap(),
        };
        engine_compare(&LocalSource::new(left), &LocalSource::new(right), &cfg).unwrap()
    }

    #[test]
    fn move_empties_origin_when_all_verified_identical() {
        // Post-apply state: origin and destination are byte-identical (files + a subtree). The move
        // must delete every origin file AND prune the now-empty directory, leaving the origin empty.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        for root in [l.path(), r.path()] {
            fs::write(root.join("a.txt"), b"A").unwrap();
            fs::write(root.join("b.txt"), b"B").unwrap();
            fs::create_dir(root.join("d")).unwrap();
            fs::write(root.join("d").join("c.txt"), b"C").unwrap();
        }
        let left = LocalSource::new(l.path());
        let right = LocalSource::new(r.path());

        let mut deleted: Vec<String> = Vec::new();
        let summary = move_origin(
            &left,
            &right,
            FilterSet::default(),
            || false,
            |o| {
                assert!(o.ok, "delete failed: {:?}", o.error);
                deleted.push(o.rel_path.components().join("/"));
            },
        )
        .unwrap();

        assert!(summary.origin_deleted, "origin should have been deleted");
        assert!(summary.blockers.is_empty());
        assert_eq!(summary.files_deleted, 3); // a.txt, b.txt, d/c.txt
        assert_eq!(summary.dirs_pruned, 1); // d
        assert_eq!(summary.failed, 0);
        // The whole origin is gone; the destination is untouched.
        assert_eq!(fs::read_dir(l.path()).unwrap().count(), 0, "origin not empty");
        assert!(r.path().join("a.txt").exists() && r.path().join("d").join("c.txt").exists());
        assert!(deleted.contains(&"d/c.txt".to_string()) && deleted.contains(&"d".to_string()));
    }

    #[test]
    fn move_keeps_whole_origin_when_one_item_not_identical() {
        // All-or-nothing: a single non-identical origin item (here, one only-in-origin file — e.g. its
        // copy was skipped or failed) blocks the move. NOTHING is deleted from the origin.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        fs::write(l.path().join("ok.txt"), b"same").unwrap();
        fs::write(r.path().join("ok.txt"), b"same").unwrap();
        fs::write(l.path().join("orphan.txt"), b"only in origin").unwrap(); // not in destination
        let left = LocalSource::new(l.path());
        let right = LocalSource::new(r.path());

        let summary = move_origin(&left, &right, FilterSet::default(), || false, |_| {
            panic!("nothing must be deleted when the move is blocked");
        })
        .unwrap();

        assert!(!summary.origin_deleted);
        assert!(summary.attempted);
        assert_eq!(summary.files_deleted, 0);
        assert_eq!(summary.blockers.len(), 1);
        assert!(summary.blockers[0].contains("orphan.txt"));
        assert!(summary.blockers[0].contains("only in origin"));
        // Both origin files survive — even the one that WAS identical.
        assert!(l.path().join("ok.txt").exists() && l.path().join("orphan.txt").exists());
    }

    #[test]
    fn move_leaves_excluded_files_in_origin() {
        // "Delete the whole origin minus exclusions": an excluded file does not block the move, is not
        // deleted, and its parent directory (now holding only the survivor) is NOT pruned.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        // Top-level identical file → deleted.
        fs::write(l.path().join("move.txt"), b"M").unwrap();
        fs::write(r.path().join("move.txt"), b"M").unwrap();
        // A directory with one identical file (deleted) and one excluded log (kept → dir survives).
        for root in [l.path(), r.path()] {
            fs::create_dir(root.join("d")).unwrap();
            fs::write(root.join("d").join("data.bin"), b"D").unwrap();
        }
        fs::write(l.path().join("d").join("keep.log"), b"L").unwrap(); // only in origin, excluded
        let left = LocalSource::new(l.path());
        let right = LocalSource::new(r.path());

        let summary =
            move_origin(&left, &right, FilterSet::new(&[], &["*.log".to_string()]).unwrap(), || false, |o| {
                assert!(o.ok, "delete failed: {:?}", o.error);
            })
            .unwrap();

        assert!(summary.origin_deleted, "excluded file must not block the move");
        assert!(summary.blockers.is_empty());
        assert_eq!(summary.files_deleted, 2); // move.txt + d/data.bin
        assert_eq!(summary.dirs_pruned, 0); // d kept (still holds keep.log)
        assert!(!l.path().join("move.txt").exists());
        assert!(!l.path().join("d").join("data.bin").exists());
        assert!(l.path().join("d").join("keep.log").exists(), "excluded file must survive");
        assert!(l.path().join("d").exists(), "dir with a survivor must not be pruned");
    }

    #[test]
    fn move_cancellation_leaves_origin_untouched() {
        // Cancelling before the first removal: the re-compare ran, but no origin item is deleted.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        fs::write(l.path().join("a.txt"), b"A").unwrap();
        fs::write(r.path().join("a.txt"), b"A").unwrap();
        let left = LocalSource::new(l.path());
        let right = LocalSource::new(r.path());

        let summary = move_origin(&left, &right, FilterSet::default(), || true, |_| {
            panic!("nothing must be deleted once cancelled");
        })
        .unwrap();

        assert!(summary.cancelled);
        assert!(!summary.origin_deleted);
        assert_eq!(summary.files_deleted, 0);
        assert!(l.path().join("a.txt").exists());
    }

    #[test]
    fn plan_origin_delete_classifies_each_status() {
        // Drive the pure gate from a real re-comparison covering every category at once.
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        // identical file + identical subtree → deletable
        fs::write(l.path().join("same.txt"), b"s").unwrap();
        fs::write(r.path().join("same.txt"), b"s").unwrap();
        for root in [l.path(), r.path()] {
            fs::create_dir(root.join("kept")).unwrap();
            fs::write(root.join("kept").join("inner.txt"), b"i").unwrap();
        }
        // different content → blocker
        fs::write(l.path().join("diff.txt"), b"left").unwrap();
        fs::write(r.path().join("diff.txt"), b"right").unwrap();
        // only in origin → blocker
        fs::write(l.path().join("orphan.txt"), b"o").unwrap();
        // only in destination → ignored (no origin to delete)
        fs::write(r.path().join("extra.txt"), b"e").unwrap();
        // excluded → neither deleted nor a blocker
        fs::write(l.path().join("skip.log"), b"x").unwrap();

        let report = reverify(l.path(), r.path(), &["*.log"]);
        let plan = plan_origin_delete(&report.root);

        let files: Vec<String> = plan.files.iter().map(|p| p.components().join("/")).collect();
        let dirs: Vec<String> = plan.dirs.iter().map(|p| p.components().join("/")).collect();
        let blockers: Vec<(String, &str)> =
            plan.blockers.iter().map(|b| (b.rel_path.join("/"), b.reason)).collect();

        assert!(files.contains(&"same.txt".to_string()));
        assert!(files.contains(&"kept/inner.txt".to_string()));
        assert!(!files.iter().any(|f| f == "skip.log"), "excluded file must not be deletable");
        assert_eq!(dirs, vec!["kept".to_string()]);
        assert_eq!(blockers.len(), 2);
        assert!(blockers.iter().any(|(p, why)| p == "diff.txt" && *why == "differs in destination"));
        assert!(blockers.iter().any(|(p, why)| p == "orphan.txt" && *why == "only in origin (not copied)"));
    }

    // ── Sync (S1): bidirectional reconciliation ─────────────────────────────────────────────────

    /// Compare two `fs` roots (full, recursive) and collect sync actions for `flags`.
    fn sync_plan(left: &Path, right: &Path, flags: SyncFlags) -> Vec<MigrateAction> {
        let cfg = CompareConfig {
            method: CompareMethod::Full,
            recursive: true,
            filters: FilterSet::default(),
        };
        let report =
            engine_compare(&LocalSource::new(left), &LocalSource::new(right), &cfg).unwrap();
        let mut actions = Vec::new();
        collect_sync(&report.root, flags, &mut actions);
        actions
    }

    /// A common fixture: a left-only file+dir, a right-only file+dir, a conflict, and an identical file.
    fn sync_fixture() -> (tempfile::TempDir, tempfile::TempDir) {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        fs::write(l.path().join("new_left.txt"), b"L").unwrap();
        fs::create_dir(l.path().join("ldir")).unwrap();
        fs::write(l.path().join("ldir").join("a.txt"), b"a").unwrap();
        fs::write(r.path().join("new_right.txt"), b"R").unwrap();
        fs::create_dir(r.path().join("rdir")).unwrap();
        fs::write(r.path().join("rdir").join("b.txt"), b"b").unwrap();
        // Conflict: present on both, different content (left larger, for the Larger rule).
        fs::write(l.path().join("conflict.txt"), b"left-is-bigger").unwrap();
        fs::write(r.path().join("conflict.txt"), b"r").unwrap();
        // Identical: never touched.
        fs::write(l.path().join("same.txt"), b"s").unwrap();
        fs::write(r.path().join("same.txt"), b"s").unwrap();
        (l, r)
    }

    /// (path, op) pairs, sorted — the shape we assert against.
    fn ops(actions: &[MigrateAction]) -> Vec<(String, SyncOp)> {
        let mut v: Vec<(String, SyncOp)> =
            actions.iter().map(|a| (a.rel_path.join("/"), a.op)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    #[test]
    fn collect_sync_trust_left_only_is_migrate_left_to_right() {
        let (l, r) = sync_fixture();
        let flags = SyncFlags {
            trust_left: true,
            trust_right: false,
            delete_diffs: false,
            conflict_rule: ConflictRule::Newer,
        };
        // Left is the sole authority: its unique items + the conflict flow right; right-only items are
        // left untouched (delete off); the conflict is won by left.
        assert_eq!(
            ops(&sync_plan(l.path(), r.path(), flags)),
            vec![
                ("conflict.txt".to_string(), SyncOp::CopyLeftToRight),
                ("ldir".to_string(), SyncOp::CopyLeftToRight),
                ("new_left.txt".to_string(), SyncOp::CopyLeftToRight),
            ]
        );
    }

    #[test]
    fn collect_sync_trust_left_with_delete_removes_right_extras() {
        let (l, r) = sync_fixture();
        let flags = SyncFlags {
            trust_left: true,
            trust_right: false,
            delete_diffs: true,
            conflict_rule: ConflictRule::Newer,
        };
        // Same as above plus the untrusted side's (right) extras are deleted.
        assert_eq!(
            ops(&sync_plan(l.path(), r.path(), flags)),
            vec![
                ("conflict.txt".to_string(), SyncOp::CopyLeftToRight),
                ("ldir".to_string(), SyncOp::CopyLeftToRight),
                ("new_left.txt".to_string(), SyncOp::CopyLeftToRight),
                ("new_right.txt".to_string(), SyncOp::DeleteRight),
                ("rdir".to_string(), SyncOp::DeleteRight),
            ]
        );
    }

    #[test]
    fn collect_sync_trust_right_only_mirrors_to_left() {
        let (l, r) = sync_fixture();
        let flags = SyncFlags {
            trust_left: false,
            trust_right: true,
            delete_diffs: true,
            conflict_rule: ConflictRule::Newer,
        };
        // Right is the sole authority: its items flow left, the conflict is won by right, and left's
        // extras are deleted.
        assert_eq!(
            ops(&sync_plan(l.path(), r.path(), flags)),
            vec![
                ("conflict.txt".to_string(), SyncOp::CopyRightToLeft),
                ("ldir".to_string(), SyncOp::DeleteLeft),
                ("new_left.txt".to_string(), SyncOp::DeleteLeft),
                ("new_right.txt".to_string(), SyncOp::CopyRightToLeft),
                ("rdir".to_string(), SyncOp::CopyRightToLeft),
            ]
        );
    }

    #[test]
    fn collect_sync_trust_both_is_union_with_conflict_rule() {
        let (l, r) = sync_fixture();
        // Larger wins → the conflict (left is bigger) flows left→right. delete_diffs is irrelevant when
        // both are trusted (every one-sided item is copied, never deleted).
        let larger = SyncFlags {
            trust_left: true,
            trust_right: true,
            delete_diffs: true,
            conflict_rule: ConflictRule::Larger,
        };
        assert_eq!(
            ops(&sync_plan(l.path(), r.path(), larger)),
            vec![
                ("conflict.txt".to_string(), SyncOp::CopyLeftToRight),
                ("ldir".to_string(), SyncOp::CopyLeftToRight),
                ("new_left.txt".to_string(), SyncOp::CopyLeftToRight),
                ("new_right.txt".to_string(), SyncOp::CopyRightToLeft),
                ("rdir".to_string(), SyncOp::CopyRightToLeft),
            ]
        );

        // Manual → the conflict is omitted (left for the user to resolve in Compare); copies still flow.
        let manual = SyncFlags { conflict_rule: ConflictRule::Manual, ..larger };
        let plan = sync_plan(l.path(), r.path(), manual);
        assert!(!plan.iter().any(|a| a.rel_path.join("/") == "conflict.txt"));
        assert_eq!(plan.len(), 4); // the four one-sided copies, no conflict
    }

    #[test]
    fn resolve_conflict_honours_rules_and_leaves_ties_manual() {
        let entry = |size_l: u64, mtime_l: Option<i64>, size_r: u64, mtime_r: Option<i64>| {
            let meta = |size, mtime| EntryMeta {
                name: "f".into(),
                rel_path: RelPath::root().child("f"),
                kind: confold_core::EntryKind::File,
                size,
                mtime,
                created: None,
            };
            DiffEntry {
                rel_path: RelPath::root().child("f"),
                name: "f".into(),
                is_dir: false,
                status: DiffStatus::Different,
                left: Some(meta(size_l, mtime_l)),
                right: Some(meta(size_r, mtime_r)),
                detail: None,
                children: vec![],
            }
        };
        let both = |rule| SyncFlags {
            trust_left: true,
            trust_right: true,
            delete_diffs: false,
            conflict_rule: rule,
        };

        // Newer wins by mtime; equal/unknown mtime → manual (None).
        let e = entry(1, Some(200), 1, Some(100));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Newer)), Some(SyncOp::CopyLeftToRight));
        let e = entry(1, Some(100), 1, Some(200));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Newer)), Some(SyncOp::CopyRightToLeft));
        let e = entry(1, Some(100), 1, Some(100));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Newer)), None); // tie
        let e = entry(1, None, 1, Some(100));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Newer)), None); // unknown

        // Larger wins by size; equal size → manual.
        let e = entry(10, None, 5, None);
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Larger)), Some(SyncOp::CopyLeftToRight));
        let e = entry(5, None, 5, None);
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Larger)), None); // tie

        // Manual never auto-resolves, even with a clear winner.
        let e = entry(10, Some(200), 5, Some(100));
        assert_eq!(resolve_conflict(&e, both(ConflictRule::Manual)), None);

        // One-sided trust: the trusted side always wins, regardless of metadata or rule.
        let e = entry(1, Some(100), 999, Some(999));
        let left_only = SyncFlags { trust_right: false, ..both(ConflictRule::Newer) };
        assert_eq!(resolve_conflict(&e, left_only), Some(SyncOp::CopyLeftToRight));
        let right_only = SyncFlags { trust_left: false, ..both(ConflictRule::Newer) };
        assert_eq!(resolve_conflict(&e, right_only), Some(SyncOp::CopyRightToLeft));

        // Neither trusted → nothing (the UI prevents this state).
        let neither = SyncFlags { trust_left: false, trust_right: false, ..both(ConflictRule::Newer) };
        assert_eq!(resolve_conflict(&e, neither), None);
    }

    #[test]
    fn fingerprint_is_stable_and_distinguishes() {
        assert_eq!(fingerprint(b"hello"), fingerprint(b"hello"));
        assert_ne!(fingerprint(b"hello"), fingerprint(b"world"));
    }

    #[test]
    fn detect_eol_picks_crlf_else_lf() {
        assert_eq!(detect_eol("a\r\nb"), "\r\n");
        assert_eq!(detect_eol("a\nb"), "\n");
        assert_eq!(detect_eol("no newline"), "\n");
    }

    #[test]
    fn read_capped_rejects_files_over_cap() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("small.txt"), b"ok").unwrap();
        assert!(read_capped(&fref(dir.path(), "small.txt"), TEXT_CAP).is_ok());
        fs::write(
            dir.path().join("big.bin"),
            vec![0u8; (TEXT_CAP + 1) as usize],
        )
        .unwrap();
        assert!(read_capped(&fref(dir.path(), "big.bin"), TEXT_CAP).is_err());
    }

    #[test]
    fn read_side_cached_treats_missing_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("here.txt"), b"hi").unwrap();
        let src = LocalSource::new(dir.path());
        assert_eq!(
            read_side_cached(&src, &rel_from_str("here.txt"), TEXT_CAP).unwrap().0,
            b"hi"
        );
        // A missing file (present on only one side) reads as empty, not an error.
        assert_eq!(
            read_side_cached(&src, &rel_from_str("gone.txt"), TEXT_CAP).unwrap().0,
            Vec::<u8>::new()
        );
    }

    #[test]
    fn rel_from_str_splits_on_slash() {
        assert_eq!(rel_from_str("a.txt"), RelPath::root().child("a.txt"));
        assert_eq!(
            rel_from_str("sub/dir/f.txt"),
            RelPath::root().child("sub").child("dir").child("f.txt")
        );
        assert!(rel_from_str("").is_root());
    }

    #[test]
    fn diff_file_distinguishes_text_and_binary() {
        let dir = tempfile::tempdir().unwrap();
        let src = LocalSource::new(dir.path());
        fs::write(dir.path().join("a.txt"), "x\ny\n").unwrap();
        fs::write(dir.path().join("b.txt"), "x\nz\n").unwrap();
        let res = diff_file_inner(&src, &rel_from_str("a.txt"), &src, &rel_from_str("b.txt")).unwrap();
        assert!(matches!(res.result, DiffResult::Text { .. }));
        assert!(!res.left.fp.is_empty() && !res.right.fp.is_empty());

        fs::write(dir.path().join("a.bin"), [0xff, 0xfe, 0x00]).unwrap();
        fs::write(dir.path().join("b.bin"), [0xff, 0xfe, 0x00]).unwrap();
        match diff_file_inner(&src, &rel_from_str("a.bin"), &src, &rel_from_str("b.bin"))
            .unwrap()
            .result
        {
            DiffResult::Binary { identical } => assert!(identical),
            _ => panic!("expected a binary verdict"),
        }
    }

    #[test]
    fn diff_file_signals_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let src = LocalSource::new(dir.path());
        fs::write(dir.path().join("big.txt"), vec![b'a'; (TEXT_CAP + 1) as usize]).unwrap();
        fs::write(dir.path().join("small.txt"), b"hi").unwrap();
        match diff_file_inner(&src, &rel_from_str("big.txt"), &src, &rel_from_str("small.txt")).unwrap().result {
            DiffResult::TooLarge { left_size, right_size } => {
                assert_eq!(left_size, TEXT_CAP + 1);
                assert_eq!(right_size, 2);
            }
            _ => panic!("expected TooLarge"),
        }
    }

    #[test]
    fn save_file_guards_conflicts_and_forces() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("c.txt"), "old").unwrap();
        let old_fp = fingerprint(b"old");
        let f = fref(dir.path(), "c.txt");
        let on_disk = || fs::read_to_string(dir.path().join("c.txt")).unwrap();

        // expected fp matches what's on disk → writes.
        assert!(matches!(
            save_file(f.clone(), "new".into(), Some(old_fp.clone()), false).unwrap(),
            SaveResult::Saved { .. }
        ));
        assert_eq!(on_disk(), "new");

        // file changed under us (stale expected fp) → conflict, nothing written.
        assert!(matches!(
            save_file(f.clone(), "newer".into(), Some(old_fp.clone()), false).unwrap(),
            SaveResult::Conflict
        ));
        assert_eq!(on_disk(), "new");

        // force overrides the conflict.
        assert!(matches!(
            save_file(f, "forced".into(), Some(old_fp), true).unwrap(),
            SaveResult::Saved { .. }
        ));
        assert_eq!(on_disk(), "forced");
    }

    #[test]
    fn save_file_creates_a_new_file_when_no_expected_fp() {
        let dir = tempfile::tempdir().unwrap();
        // No file on disk + expect=None → current(None)==expect(None) → writes a fresh file.
        assert!(matches!(
            save_file(fref(dir.path(), "new.txt"), "hi".into(), None, false).unwrap(),
            SaveResult::Saved { .. }
        ));
        assert_eq!(
            fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "hi"
        );
    }

    #[test]
    fn save_file_creates_missing_target_even_with_an_expected_fp() {
        // One-sided file: the UI read the (absent) other side as empty, so `expect` is the empty-bytes fp,
        // but the target doesn't exist → this is a create, NOT a conflict.
        let dir = tempfile::tempdir().unwrap();
        let empty_fp = fingerprint(b"");
        assert!(matches!(
            save_file(
                fref(dir.path(), "created.txt"),
                "made".into(),
                Some(empty_fp),
                false
            )
            .unwrap(),
            SaveResult::Saved { .. }
        ));
        assert_eq!(
            fs::read_to_string(dir.path().join("created.txt")).unwrap(),
            "made"
        );
    }

    #[test]
    fn hex_compare_reports_lengths_uncapped() {
        let dir = tempfile::tempdir().unwrap();
        let src = LocalSource::new(dir.path());
        fs::write(dir.path().join("l"), vec![1u8; 10]).unwrap();
        fs::write(dir.path().join("r"), vec![2u8; 20]).unwrap();
        let h = hex_compare_inner(&src, &rel_from_str("l"), &src, &rel_from_str("r"));
        assert_eq!((h.left_len, h.right_len), (10, 20));
        assert_eq!(h.left.len(), 10);
        assert!(!h.truncated);
    }

    #[test]
    fn hex_compare_truncates_large_file_without_rejecting() {
        let dir = tempfile::tempdir().unwrap();
        let src = LocalSource::new(dir.path());
        // A file bigger than HEX_CAP must NOT be rejected — only the prefix is read, truncated=true.
        fs::write(dir.path().join("big"), vec![7u8; HEX_CAP + 1000]).unwrap();
        fs::write(dir.path().join("small"), vec![7u8; 5]).unwrap();
        let h = hex_compare_inner(&src, &rel_from_str("big"), &src, &rel_from_str("small"));
        assert_eq!(h.left_len, (HEX_CAP + 1000) as u64);
        assert_eq!(h.left.len(), HEX_CAP); // prefix only
        assert!(h.truncated);
    }

    #[test]
    fn read_bytes_errors_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_bytes(fref(dir.path(), "nope")).is_err());
    }

    #[test]
    fn test_source_reports_dir_file_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f.txt"), b"hi").unwrap();

        let d = test_source(fs_spec(dir.path().to_str().unwrap().to_string()));
        assert!(d.ok && d.is_dir);

        let f = test_source(fs_spec(
            dir.path().join("f.txt").to_str().unwrap().to_string(),
        ));
        assert!(f.ok && !f.is_dir);

        let missing = test_source(fs_spec(
            dir.path().join("nope").to_str().unwrap().to_string(),
        ));
        assert!(!missing.ok);
    }

    #[test]
    fn source_types_catalog_lists_registered_kinds() {
        let types = source_types();
        let ids: Vec<_> = types.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["fs", "sftp", "s3"]);
        let sftp = types.iter().find(|t| t.id == "sftp").unwrap();
        assert!(sftp.capabilities.write);
        assert!(sftp.fields.iter().any(|f| f.key == "host"));
        // Credential fields are marked secret so the UI masks them.
        assert!(sftp.fields.iter().any(|f| f.secret));
        // S3 registered with a secret credential field and an icon — the picker renders it generically.
        let s3 = types.iter().find(|t| t.id == "s3").unwrap();
        assert!(!s3.icon.is_empty());
        assert!(s3.fields.iter().any(|f| f.key == "bucket"));
        assert!(s3.fields.iter().any(|f| f.key == "secret_access_key" && f.secret));
    }

    #[test]
    fn sftp_spec_deserializes_generic_and_maps_auth() {
        let json = serde_json::json!({
            "kind": "sftp",
            "fields": {
                "host": "example.com",
                "username": "alice",
                "auth.method": "password",
                "auth.password": "s3cret"
            }
        });
        let spec: SourceSpec = serde_json::from_value(json).unwrap();
        assert_eq!(spec.kind, "sftp");
        assert_eq!(spec.fields.get("host").map(String::as_str), Some("example.com"));
        // Auth maps to the domain type (defaults like port/root are applied at build time, not here).
        assert!(matches!(
            sftp_auth_from_fields(&spec.fields).unwrap(),
            SftpAuth::Password(p) if p == "s3cret"
        ));
        // The SFTP kind's cache key is secret-free and identity-stable.
        let key = kind_for("sftp").unwrap().cache_key(&spec.fields);
        assert!(key.starts_with("sftp:alice@example.com"));
        assert!(!key.contains("s3cret"));
    }
}
