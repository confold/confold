#[allow(unused_imports)]
use crate::sources::*;
#[allow(unused_imports)]
use crate::scan::*;
#[allow(unused_imports)]
use crate::plan::*;
#[allow(unused_imports)]
use crate::diff::*;
#[allow(unused_imports)]
use crate::{ENTRY_RESOLVED, MIGRATE_PROGRESS, MIGRATE_DONE, MIGRATE_PHASE, PLAN_PROGRESS, PLAN_READY};

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

/// Progress of one concrete migrate operation (a `SyncAction` may expand into several — e.g. a
/// directory copy yields one per descendant). Streamed to the UI as each completes.
#[derive(Serialize, Clone)]
pub(crate) struct MigrateProgress {
    /// The generation this run belongs to, so the UI ignores events from a superseded apply.
    pub(crate) generation: u64,
    pub(crate) rel_path: Vec<String>,
    pub(crate) op: SyncOp,
    /// The original diff reason — distinguishes `→ copy` (new) from `→ override` (different) in the UI.
    pub(crate) reason: MigrateReason,
    pub(crate) ok: bool,
    pub(crate) error: Option<String>,
}

/// Final tally of a migrate-apply run.
#[derive(Serialize, Clone, Default)]
pub(crate) struct MigrateSummary {
    /// Concrete operations attempted.
    pub(crate) total: usize,
    pub(crate) ok: usize,
    pub(crate) failed: usize,
    /// True if the run was cancelled partway (some actions may not have been attempted).
    pub(crate) cancelled: bool,
}

/// Apply migrate actions one at a time, reporting each concrete outcome and honouring cancellation.
/// Pure (no Tauri/IO of its own): `cancelled` is polled before each action (so a long single action —
/// e.g. a large directory copy — is not interrupted mid-way; cancellation granularity is per top-level
/// action), and `on_outcome` is called for every concrete outcome. Reuses the sync engine's `apply`.
pub(crate) fn run_migrate(
    left: &dyn SourceMut,
    right: &dyn SourceMut,
    actions: &[MigrateAction],
    cancelled: impl Fn() -> bool,
    mut on_outcome: impl FnMut(&ActionOutcome, MigrateReason),
) -> MigrateSummary {
    let mut summary = MigrateSummary::default();
    for action in actions {
        if cancelled() {
            summary.cancelled = true;
            break;
        }
        if !action.leaves.is_empty() {
            // Directory copy: apply leaf by leaf so cancellation fires between individual files.
            // Deletes and file actions fall through to the single-action path below.
            for leaf in &action.leaves {
                if cancelled() {
                    summary.cancelled = true;
                    return summary;
                }
                for outcome in sync_apply(left, right, &[leaf.to_sync_action(action.op)], false) {
                    summary.total += 1;
                    if outcome.ok { summary.ok += 1; } else { summary.failed += 1; }
                    on_outcome(&outcome, action.reason);
                }
            }
        } else {
            // File action, delete, or manually-constructed action (e.g. tests): single apply call.
            for outcome in sync_apply(left, right, &[action.to_sync_action()], false) {
                summary.total += 1;
                if outcome.ok { summary.ok += 1; } else { summary.failed += 1; }
                on_outcome(&outcome, action.reason);
            }
        }
    }
    summary
}

// ── M2: move semantics (delete origin after a verified-complete migration) ───────────────────────
//
// After the apply, if `delete_origin` is on, we re-compare origin↔destination with a FULL byte
// compare and delete the origin only if EVERY origin item is now verified identical in destination
// (minus exclusions). All-or-nothing: a partial origin-delete would let a later mirror pass see the
// deleted items as destination-only and remove them. The gate is derived purely from the
// re-comparison tree (`plan_origin_delete`), so its logic is unit-testable without Tauri.

/// An origin item the re-verification found NOT identical in destination — each one aborts the move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OriginBlocker {
    pub(crate) rel_path: Vec<String>,
    /// Why it blocks (human-readable, for the summary).
    pub(crate) reason: &'static str,
}

/// What deleting the origin would entail, derived from a fresh full re-comparison after the apply.
#[derive(Debug, Default)]
pub(crate) struct OriginDeletePlan {
    /// Origin files verified identical in destination — safe to delete (one DeleteLeft each, for
    /// per-file progress + cancellation).
    pub(crate) files: Vec<RelPath>,
    /// Directories on the verified-identical paths, **bottom-up** — pruned only if empty at runtime,
    /// so any excluded survivors keep their parent directories alive.
    pub(crate) dirs: Vec<RelPath>,
    /// Origin items not verified identical in destination. Non-empty → the move is aborted, origin kept.
    pub(crate) blockers: Vec<OriginBlocker>,
}

/// Walk a fresh full re-comparison tree and decide what (if anything) can be deleted from the origin.
/// Exclusions appear in the tree as `Skipped` / `"filtered"` and are intentionally left alone (they do
/// not block the move and are never deleted) — this realises "delete the whole origin minus exclusions".
pub(crate) fn plan_origin_delete(root: &DiffEntry) -> OriginDeletePlan {
    let mut plan = OriginDeletePlan::default();
    for child in &root.children {
        visit_for_origin_delete(child, &mut plan);
    }
    // `dirs` are collected pre-order (parent before child); reverse so pruning runs deepest-first.
    plan.dirs.reverse();
    plan
}

/// `true` for an entry the user excluded via the filter (kept in origin, never a blocker).
pub(crate) fn is_filtered(entry: &DiffEntry) -> bool {
    entry.status == DiffStatus::Skipped && entry.detail.as_deref() == Some("filtered")
}

pub(crate) fn visit_for_origin_delete(entry: &DiffEntry, plan: &mut OriginDeletePlan) {
    match entry.status {
        // Verified identical: the whole subtree is deletable from origin.
        DiffStatus::Identical => collect_origin_leaves(entry, plan),
        // Destination-only: nothing in origin to delete; irrelevant to the move.
        DiffStatus::RightOnly => {}
        // Partially-divergent directory: some children identical, some not — recurse to find the
        // blockers (any blocker aborts the whole move, so the identical leaves collected here are moot).
        DiffStatus::Different if entry.is_dir => {
            for child in &entry.children {
                visit_for_origin_delete(child, plan);
            }
        }
        DiffStatus::Different => plan.blockers.push(blocker(entry, "differs in destination")),
        DiffStatus::LeftOnly => plan.blockers.push(blocker(entry, "only in origin (not copied)")),
        DiffStatus::Error => plan.blockers.push(blocker(entry, "comparison error")),
        // Excluded items are left alone; any other skip (symlink, non-descended) cannot be verified.
        DiffStatus::Skipped if is_filtered(entry) => {}
        DiffStatus::Skipped => plan.blockers.push(blocker(entry, "skipped (not verified)")),
    }
}

/// Collect every deletable leaf under a verified-identical entry: files to remove + directories to
/// prune. Excluded survivors (`Skipped`/`"filtered"`) inside an identical dir are left in place.
pub(crate) fn collect_origin_leaves(entry: &DiffEntry, plan: &mut OriginDeletePlan) {
    if entry.is_dir {
        plan.dirs.push(entry.rel_path.clone());
        for child in &entry.children {
            if is_filtered(child) {
                continue;
            }
            collect_origin_leaves(child, plan);
        }
    } else {
        plan.files.push(entry.rel_path.clone());
    }
}

pub(crate) fn blocker(entry: &DiffEntry, reason: &'static str) -> OriginBlocker {
    OriginBlocker {
        rel_path: entry.rel_path.components().to_vec(),
        reason,
    }
}

/// Final tally of the move (origin-delete) phase, carried alongside the apply summary in [`MIGRATE_DONE`].
#[derive(Serialize, Clone, Default)]
pub(crate) struct MoveSummary {
    /// True once the move was attempted (delete_origin on, apply clean). False → never reached the gate.
    pub(crate) attempted: bool,
    /// True if the origin was actually deleted (gate passed and the delete ran to completion).
    pub(crate) origin_deleted: bool,
    /// Origin files deleted.
    pub(crate) files_deleted: usize,
    /// Empty origin directories pruned.
    pub(crate) dirs_pruned: usize,
    /// Failures during the origin-delete itself.
    pub(crate) failed: usize,
    /// True if the origin-delete was cancelled partway.
    pub(crate) cancelled: bool,
    /// If the move did NOT proceed, the origin items that blocked it (capped for display).
    pub(crate) blockers: Vec<String>,
}

/// Remove one origin item, building an [`ActionOutcome`] (mirrors the sync engine's shape so progress
/// events are uniform). `remove` handles both files and empty directories.
pub(crate) fn left_remove(left: &dyn SourceMut, rel: &RelPath) -> ActionOutcome {
    let result = left.remove(rel);
    ActionOutcome {
        rel_path: rel.clone(),
        op: SyncOp::DeleteLeft,
        ok: result.is_ok(),
        error: result.err().map(|e| e.to_string()),
    }
}

/// Execute an [`OriginDeletePlan`] (caller must have checked `blockers` is empty): delete each verified
/// file, then prune directories that are empty at runtime (so excluded survivors keep their parents).
/// Cancellation is polled between operations.
pub(crate) fn run_origin_delete(
    left: &dyn SourceMut,
    plan: &OriginDeletePlan,
    cancelled: impl Fn() -> bool,
    mut on_delete: impl FnMut(&ActionOutcome),
) -> MoveSummary {
    let mut summary = MoveSummary { attempted: true, ..Default::default() };
    for rel in &plan.files {
        if cancelled() {
            summary.cancelled = true;
            return summary;
        }
        let outcome = left_remove(left, rel);
        if outcome.ok {
            summary.files_deleted += 1;
        } else {
            summary.failed += 1;
        }
        on_delete(&outcome);
    }
    for rel in &plan.dirs {
        if cancelled() {
            summary.cancelled = true;
            return summary;
        }
        // Prune only when empty: a directory still holding excluded survivors must stay.
        if !matches!(left.read_dir(rel), Ok(entries) if entries.is_empty()) {
            continue;
        }
        let outcome = left_remove(left, rel);
        if outcome.ok {
            summary.dirs_pruned += 1;
        } else {
            summary.failed += 1;
        }
        on_delete(&outcome);
    }
    summary.origin_deleted = !summary.cancelled && summary.failed == 0;
    summary
}

/// Re-verify (full byte compare) origin↔destination and, all-or-nothing, delete the origin if every
/// item is now verified identical (minus exclusions). Pure of Tauri: `on_delete` receives each removal
/// outcome for progress. Returns the [`MoveSummary`]; an `Err` is a hard re-comparison failure.
pub(crate) fn move_origin(
    left: &dyn SourceMut,
    right: &dyn SourceMut,
    filters: FilterSet,
    cancelled: impl Fn() -> bool,
    on_delete: impl FnMut(&ActionOutcome),
) -> Result<MoveSummary, String> {
    let cfg = CompareConfig {
        method: CompareMethod::Full,
        recursive: true,
        filters,
    };
    // Upcast the mutable sources to read-only for the comparison (SourceMut: Source).
    let lref: &dyn Source = left;
    let rref: &dyn Source = right;
    let report = engine_compare(lref, rref, &cfg).map_err(|e| e.to_string())?;
    let plan = plan_origin_delete(&report.root);
    if !plan.blockers.is_empty() {
        const MAX_SHOWN: usize = 20;
        let blockers = plan
            .blockers
            .iter()
            .take(MAX_SHOWN)
            .map(|b| format!("{} — {}", b.rel_path.join("/"), b.reason))
            .collect();
        return Ok(MoveSummary {
            attempted: true,
            blockers,
            ..Default::default()
        });
    }
    Ok(run_origin_delete(left, &plan, cancelled, on_delete))
}

/// Final tally of a migrate-apply run, emitted as [`MIGRATE_DONE`] when the background thread ends.
#[derive(Serialize, Clone)]
pub(crate) struct MigrateDone {
    pub(crate) generation: u64,
    pub(crate) summary: MigrateSummary,
    /// MOVE result (M2): present only when `delete_origin` was requested. `None` for a plain migration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) move_result: Option<MoveSummary>,
}

/// Signals which long phase of the apply is running, so the UI can show "Verifying…" / "Emptying
/// origin…" during the M2 re-verification (which emits no per-op progress until the origin-delete).
#[derive(Serialize, Clone)]
pub(crate) struct MigratePhase {
    pub(crate) generation: u64,
    /// `"verifying"` (full re-compare) or `"emptying_origin"` (deleting origin items).
    pub(crate) phase: &'static str,
}

/// Start a migration apply on a **background thread** and return immediately. Progress arrives via
/// [`MIGRATE_PROGRESS`] events (one per concrete operation); a final [`MIGRATE_DONE`] event carries
/// the tally. Spinning work off to a thread lets the webview process events in real time instead of
/// queueing them all until a blocking command returns — which would freeze the progress UI.
///
/// When `delete_origin` is set (M2 move semantics), a successful, uncancelled apply is followed by a
/// FULL re-comparison and — all-or-nothing — deletion of the origin (minus the `opts` exclusions). See
/// [`move_origin`].
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command args map 1:1 to the JS invoke payload.
pub(crate) fn migrate_apply(
    state: State<AppState>,
    app: AppHandle,
    left: SourceSpec,
    right: SourceSpec,
    actions: Vec<MigrateAction>,
    generation: u64,
    delete_origin: bool,
    opts: CompareOpts,
) -> Result<(), String> {
    // Generation 0 is the reserved cancel sentinel (see `migrate_cancel`): a real run must use a
    // non-zero, monotonically increasing generation so cancellation can reliably target it. Enforce it
    // here rather than trusting the caller, so a frontend bug surfaces loudly instead of silently
    // breaking cancellation.
    if generation == 0 {
        return Err("invalid migrate generation (0 is reserved for cancellation)".to_owned());
    }
    // For a move, build the exclusion filters up front so a bad pattern fails synchronously to the UI.
    let filters = if delete_origin {
        Some(FilterSet::new(&opts.include, &opts.exclude).map_err(|e| e.to_string())?)
    } else {
        None
    };
    // Build sources before claiming the generation: a connection error must propagate synchronously to
    // the UI without bumping the generation (which would cancel any apply still in flight from a prior run).
    let l = build_source_mut(&left)?;
    let r = build_source_mut(&right)?;
    state.migrate_gen.store(generation, Ordering::SeqCst);
    let cancel_token = Arc::clone(&state.migrate_gen);
    std::thread::spawn(move || {
        let is_cancelled = || cancel_token.load(Ordering::SeqCst) != generation;
        let emit_progress = |outcome: &ActionOutcome, reason: MigrateReason| {
            let _ = app.emit(
                MIGRATE_PROGRESS,
                MigrateProgress {
                    generation,
                    rel_path: outcome.rel_path.components().to_vec(),
                    op: outcome.op,
                    reason,
                    ok: outcome.ok,
                    error: outcome.error.clone(),
                },
            );
        };
        let summary = run_migrate(l.as_ref(), r.as_ref(), &actions, is_cancelled, |o, reason| {
            emit_progress(o, reason)
        });

        // M2 move: re-verify (full) and, all-or-nothing, delete the origin. Only when requested and the
        // apply completed cleanly — any apply failure or cancellation keeps the origin untouched.
        let move_result = match filters {
            Some(_) if summary.cancelled => Some(MoveSummary::default()),
            Some(_) if summary.failed > 0 => Some(MoveSummary {
                attempted: true,
                blockers: vec![format!(
                    "{} item(s) failed to copy — origin kept",
                    summary.failed
                )],
                ..Default::default()
            }),
            Some(filters) => {
                let _ = app.emit(MIGRATE_PHASE, MigratePhase { generation, phase: "verifying" });
                let mut emitted_emptying = false;
                let result = move_origin(l.as_ref(), r.as_ref(), filters, is_cancelled, |outcome| {
                    if !emitted_emptying {
                        emitted_emptying = true;
                        let _ = app.emit(
                            MIGRATE_PHASE,
                            MigratePhase { generation, phase: "emptying_origin" },
                        );
                    }
                    emit_progress(outcome, MigrateReason::Moved);
                });
                Some(result.unwrap_or_else(|e| MoveSummary {
                    attempted: true,
                    blockers: vec![format!("re-verification failed: {e}")],
                    ..Default::default()
                }))
            }
            None => None,
        };

        let _ = app.emit(MIGRATE_DONE, MigrateDone { generation, summary, move_result });
    });
    Ok(())
}

/// Cancel the migrate-apply in progress (if any): sets the generation to 0 (a value no run uses), so
/// the running loop stops before its next action.
#[tauri::command]
pub(crate) fn migrate_cancel(state: State<AppState>) {
    state.migrate_gen.store(0, Ordering::SeqCst);
}

/// Preview (dry-run) the effect of a batch of sync actions — writes nothing.
#[tauri::command]
pub(crate) fn plan_actions(
    left: SourceSpec,
    right: SourceSpec,
    actions: Vec<SyncAction>,
) -> Result<Vec<ActionOutcome>, String> {
    let l = build_source_mut(&left)?;
    let r = build_source_mut(&right)?;
    Ok(sync_apply(l.as_ref(), r.as_ref(), &actions, true))
}

/// Execute a batch of sync actions (copy/delete left↔right) and report per-operation outcomes.
#[tauri::command]
pub(crate) fn apply_actions(
    left: SourceSpec,
    right: SourceSpec,
    actions: Vec<SyncAction>,
) -> Result<Vec<ActionOutcome>, String> {
    let l = build_source_mut(&left)?;
    let r = build_source_mut(&right)?;
    Ok(sync_apply(l.as_ref(), r.as_ref(), &actions, false))
}
