#[allow(unused_imports)]
use crate::sources::*;
#[allow(unused_imports)]
use crate::scan::*;
#[allow(unused_imports)]
use crate::apply::*;
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

/// Which categories of difference a migration (origin → destination) acts on. The destructive ones
/// are opt-in in the UI, each behind a warning. The MOVE step (delete origin after a verified-complete
/// migration, M2) is not a plan category — it is requested at apply time via `migrate_apply`'s
/// `deleteOrigin` flag and realised by [`move_origin`], so it does not belong here.
#[derive(Deserialize, Clone, Copy)]
pub(crate) struct MigrateFlags {
    /// Copy items present only in origin → destination (create them on the destination).
    pub(crate) copy_new: bool,
    /// Overwrite destination files whose content differs from origin.
    pub(crate) overwrite_different: bool,
    /// Delete items present only in destination — mirror, makes the destination match origin.
    pub(crate) delete_extra: bool,
}

/// Why a migration action was generated — carries the original diff status so the UI can show
/// `→ copy` vs `→ override` vs `✕ delete` per side without re-running the comparison.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MigrateReason {
    /// Item is only in origin — will be created in destination.
    New,
    /// Item exists on both sides but content differs — will overwrite destination.
    Different,
    /// Item is only in destination — will be deleted from destination.
    Extra,
    /// MOVE semantics (M2): an origin item deleted after being verified identical in destination.
    /// Only emitted on the post-apply origin-delete progress, never produced by `collect_migrate`.
    Moved,
}

/// A single leaf operation within a directory copy — one file or one empty directory. Used to drive
/// per-file cancellation without exposing the full subtree to the UI.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct LeafOp {
    pub(crate) rel_path: Vec<String>,
    /// `true` only for empty directories (which need an explicit `create_dir_all` since no file copy
    /// would trigger it automatically).
    pub(crate) is_dir: bool,
}

impl LeafOp {
    pub(crate) fn to_sync_action(&self, op: SyncOp) -> SyncAction {
        let mut rel = RelPath::root();
        for component in &self.rel_path {
            rel = rel.child(component);
        }
        SyncAction { rel_path: rel, op, is_dir: self.is_dir }
    }
}

/// A planned migrate operation enriched with its origin diff status (`reason`), so the UI can
/// display what happens on each side without re-running the comparison.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct MigrateAction {
    /// Path components, relative to the compared roots.
    pub(crate) rel_path: Vec<String>,
    pub(crate) op: SyncOp,
    pub(crate) is_dir: bool,
    pub(crate) reason: MigrateReason,
    /// Total items inside a directory action (files + sub-dirs, recursive). 1 for a file action.
    /// Lets the UI show "dir • 47 items" without listing every leaf in the plan.
    pub(crate) item_count: usize,
    /// Pre-expanded leaf operations for directory COPY actions (enables per-file cancellation).
    /// The UI never sees this detail — it's only used by `run_migrate`. Empty for file actions and
    /// delete actions (deletes stay atomic: cancelling a partial delete leaves inconsistent state).
    #[serde(default)]
    pub(crate) leaves: Vec<LeafOp>,
}

impl MigrateAction {
    pub(crate) fn to_sync_action(&self) -> SyncAction {
        let mut rel = RelPath::root();
        for component in &self.rel_path {
            rel = rel.child(component);
        }
        SyncAction {
            rel_path: rel,
            op: self.op,
            is_dir: self.is_dir,
        }
    }
}

/// Count all items (files + directories) in a subtree recursively. Used for directory actions so the
/// UI can show "dir • 47 items" without listing every leaf in the plan.
pub(crate) fn count_items(node: &DiffEntry) -> usize {
    node.children.iter().fold(0, |acc, c| {
        acc + 1 + if c.is_dir { count_items(c) } else { 0 }
    })
}

/// Expand a directory subtree into per-leaf copy operations so `run_migrate` can cancel between
/// individual files. Files and empty directories are added directly; non-empty directories are
/// recursed into (their files will create parent directories automatically via `copy_from`).
pub(crate) fn collect_copy_leaves(node: &DiffEntry, out: &mut Vec<LeafOp>) {
    for child in &node.children {
        if !child.is_dir {
            out.push(LeafOp { rel_path: child.rel_path.components().to_vec(), is_dir: false });
        } else if child.children.is_empty() {
            // Empty directory: needs an explicit create since no file copy will trigger mkdir.
            out.push(LeafOp { rel_path: child.rel_path.components().to_vec(), is_dir: true });
        } else {
            collect_copy_leaves(child, out);
        }
    }
}

/// Expand a directory subtree into per-leaf DELETE operations in bottom-up order (children before
/// parents), so cancellation fires between individual files and dirs are only removed once empty.
/// The caller is responsible for appending the root dir itself after calling this.
pub(crate) fn collect_delete_leaves(node: &DiffEntry, out: &mut Vec<LeafOp>) {
    for child in &node.children {
        if !child.is_dir {
            out.push(LeafOp { rel_path: child.rel_path.components().to_vec(), is_dir: false });
        } else {
            collect_delete_leaves(child, out); // children first
            out.push(LeafOp { rel_path: child.rel_path.components().to_vec(), is_dir: true }); // then the dir (now empty)
        }
    }
}

/// Build a copy action for a diff entry in a given direction. A directory expands into per-leaf copies
/// (`leaves`); a file is a single op. Shared by Migrate and Sync — `op` carries the direction.
pub(crate) fn copy_subtree_action(c: &DiffEntry, op: SyncOp, reason: MigrateReason) -> MigrateAction {
    let mut leaves = Vec::new();
    if c.is_dir {
        collect_copy_leaves(c, &mut leaves);
    }
    MigrateAction {
        rel_path: c.rel_path.components().to_vec(),
        op,
        is_dir: c.is_dir,
        reason,
        item_count: if c.is_dir { count_items(c) } else { 1 },
        leaves,
    }
}

/// Build a delete action for a diff entry on a given side. A directory expands bottom-up (children then
/// the now-empty dir) so cancellation is per-leaf. Shared by Migrate (delete-extra) and Sync.
pub(crate) fn delete_subtree_action(c: &DiffEntry, op: SyncOp) -> MigrateAction {
    let mut leaves = Vec::new();
    if c.is_dir {
        collect_delete_leaves(c, &mut leaves); // children bottom-up
        leaves.push(LeafOp { rel_path: c.rel_path.components().to_vec(), is_dir: true }); // root dir last
    }
    MigrateAction {
        rel_path: c.rel_path.components().to_vec(),
        op,
        is_dir: c.is_dir,
        reason: MigrateReason::Extra,
        item_count: if c.is_dir { count_items(c) } else { 1 },
        leaves,
    }
}

/// Walk a (recursive) diff tree and collect migrate actions enriched with their reason and item count.
/// One action per top-level diff: a one-sided directory becomes a single recursive copy/delete; a
/// "different" directory is a container, so we recurse into it and only act on its diverging children.
pub(crate) fn collect_migrate(node: &DiffEntry, flags: MigrateFlags, out: &mut Vec<MigrateAction>) {
    for c in &node.children {
        match c.status {
            DiffStatus::LeftOnly => {
                if flags.copy_new {
                    out.push(copy_subtree_action(c, SyncOp::CopyLeftToRight, MigrateReason::New));
                }
            }
            DiffStatus::Different => {
                if c.is_dir {
                    collect_migrate(c, flags, out);
                } else if flags.overwrite_different {
                    out.push(copy_subtree_action(c, SyncOp::CopyLeftToRight, MigrateReason::Different));
                }
            }
            DiffStatus::RightOnly => {
                if flags.delete_extra {
                    out.push(delete_subtree_action(c, SyncOp::DeleteRight));
                }
            }
            DiffStatus::Identical | DiffStatus::Skipped | DiffStatus::Error => {}
        }
    }
}

// ── Sync: bidirectional reconciliation (S1) ──────────────────────────────────────────────────────
//
// Sync generalises Migrate. The two "trust" flags declare which side(s) are authoritative; direction
// follows from that (no stored baseline needed). When exactly one side is trusted it behaves like a
// one-directional Migrate (and `delete_diffs` mirrors `delete_extra`); when both are trusted it's a
// bidirectional union and `conflict_rule` decides each "different on both sides" file. Reuses
// `MigrateAction` (the `op` carries direction; `reason` classifies New/Different/Extra for the counts)
// and the whole apply path (`migrate_apply` with `delete_origin = false`).

/// How a "different on both sides" conflict is resolved when BOTH sides are trusted.
#[derive(Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConflictRule {
    /// Never auto-resolve — leave every conflict for the user to handle in Compare afterward. Default:
    /// it's the least destructive (no file is ever auto-overwritten on a content conflict).
    #[default]
    Manual,
    /// The side with the later mtime wins; a tie or unknown mtime is left for manual resolution.
    Newer,
    /// The larger file wins; equal size is left for manual resolution.
    Larger,
}

/// Which side(s) a Sync run trusts, plus how it handles the untrusted side's extras and conflicts. At
/// least one side must be trusted (the UI enforces it; neither-trusted yields no actions).
#[derive(Deserialize, Clone, Copy)]
pub(crate) struct SyncFlags {
    /// Left source is authoritative — its content propagates to the right.
    pub(crate) trust_left: bool,
    /// Right source is authoritative — its content propagates to the left.
    pub(crate) trust_right: bool,
    /// When exactly one side is trusted, also delete the untrusted side's extra items.
    #[serde(default)]
    pub(crate) delete_diffs: bool,
    /// Conflict resolution when BOTH sides are trusted (ignored otherwise).
    #[serde(default)]
    pub(crate) conflict_rule: ConflictRule,
}

/// Decide which way a single "different" file is copied, or `None` to leave it for manual resolution
/// (manual rule, neither side trusted, or a tie under newer/larger).
pub(crate) fn resolve_conflict(c: &DiffEntry, flags: SyncFlags) -> Option<SyncOp> {
    match (flags.trust_left, flags.trust_right) {
        (true, false) => Some(SyncOp::CopyLeftToRight), // left is the sole authority
        (false, true) => Some(SyncOp::CopyRightToLeft), // right is the sole authority
        (false, false) => None,                         // no authority (UI prevents this)
        (true, true) => match flags.conflict_rule {
            ConflictRule::Manual => None,
            ConflictRule::Newer => {
                pick_by(c.left.as_ref().and_then(|m| m.mtime), c.right.as_ref().and_then(|m| m.mtime))
            }
            ConflictRule::Larger => {
                pick_by(c.left.as_ref().map(|m| m.size), c.right.as_ref().map(|m| m.size))
            }
        },
    }
}

/// Copy from the side with the strictly-greater metric; `None` on a tie or any missing value.
pub(crate) fn pick_by<T: Ord>(left: Option<T>, right: Option<T>) -> Option<SyncOp> {
    match (left, right) {
        (Some(l), Some(r)) if l > r => Some(SyncOp::CopyLeftToRight),
        (Some(l), Some(r)) if r > l => Some(SyncOp::CopyRightToLeft),
        _ => None,
    }
}

/// Walk a (recursive) diff tree and collect bidirectional sync actions per the trust flags + conflict
/// rule. Unresolved conflicts (manual rule, or a tie) are omitted — the post-apply re-compare surfaces
/// them for manual resolution in Compare. Mirrors `collect_migrate`'s container/recursion handling.
pub(crate) fn collect_sync(node: &DiffEntry, flags: SyncFlags, out: &mut Vec<MigrateAction>) {
    for c in &node.children {
        match c.status {
            DiffStatus::LeftOnly => {
                if flags.trust_left {
                    out.push(copy_subtree_action(c, SyncOp::CopyLeftToRight, MigrateReason::New));
                } else if flags.trust_right && flags.delete_diffs {
                    out.push(delete_subtree_action(c, SyncOp::DeleteLeft));
                }
            }
            DiffStatus::RightOnly => {
                if flags.trust_right {
                    out.push(copy_subtree_action(c, SyncOp::CopyRightToLeft, MigrateReason::New));
                } else if flags.trust_left && flags.delete_diffs {
                    out.push(delete_subtree_action(c, SyncOp::DeleteRight));
                }
            }
            DiffStatus::Different => {
                if c.is_dir {
                    collect_sync(c, flags, out); // container: recurse into diverging children
                } else if let Some(op) = resolve_conflict(c, flags) {
                    out.push(MigrateAction {
                        rel_path: c.rel_path.components().to_vec(),
                        op,
                        is_dir: false,
                        reason: MigrateReason::Different,
                        item_count: 1,
                        leaves: vec![],
                    });
                }
            }
            DiffStatus::Identical | DiffStatus::Skipped | DiffStatus::Error => {}
        }
    }
}

/// Plan a migration: run a FULL recursive comparison (so nothing is decided on metadata alone, and big
/// files stream rather than load) and return the enriched actions (with their `reason`) for the enabled
/// `flags`. The UI then previews them (`plan_actions`) and, on confirm, applies them. Direction is fixed
/// origin → dest.
/// Live "examined N items" counter while a plan is computed (throttled — one per 256 entries).
#[derive(Serialize, Clone)]
pub(crate) struct PlanProgress {
    pub(crate) token: u64,
    pub(crate) examined: u64,
}

/// Result of a migrate/sync plan computation, streamed to the UI when the background compare finishes.
#[derive(Serialize, Clone)]
pub(crate) struct PlanReady {
    pub(crate) token: u64,
    /// `"migrate"` or `"sync"` — so the UI opens the right plan modal.
    pub(crate) flow: &'static str,
    pub(crate) actions: Vec<MigrateAction>,
    /// Set instead of `actions` when the compare failed.
    pub(crate) error: Option<String>,
}

/// Run a migrate/sync plan computation OFF the main thread (called from a plain `std::thread` so the
/// S3/SFTP sources' own `block_on` is safe — see memory `tauri-heavy-commands-async`). Streams a throttled
/// `PLAN_PROGRESS` counter as the recursive compare walks, then emits `PLAN_READY` (actions or error).
/// A newer preview bumps `plan_token`, so a superseded plan drops its progress + result silently.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_plan(
    app: &AppHandle,
    left: &dyn Source,
    right: &dyn Source,
    cfg: &CompareConfig,
    flow: &'static str,
    token: u64,
    plan_token: &Arc<AtomicU64>,
    collect: impl Fn(&DiffEntry) -> Vec<MigrateAction>,
) {
    let examined = AtomicU64::new(0);
    let progress = || {
        let n = examined.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_multiple_of(256) && plan_token.load(Ordering::SeqCst) == token {
            let _ = app.emit(PLAN_PROGRESS, PlanProgress { token, examined: n });
        }
    };
    let result = engine_compare_with_progress(left, right, &RelPath::root(), cfg, &progress);
    if plan_token.load(Ordering::SeqCst) != token {
        return; // a newer preview superseded this one
    }
    let payload = match result {
        Ok(report) => {
            // Emit the exact final count so the throttled counter lands on the true total instead of
            // freezing at the last multiple of 256 (the tail < 256 entries emit no progress event).
            let _ = app.emit(PLAN_PROGRESS, PlanProgress { token, examined: examined.load(Ordering::Relaxed) });
            PlanReady { token, flow, actions: collect(&report.root), error: None }
        }
        Err(e) => PlanReady { token, flow, actions: Vec::new(), error: Some(e.to_string()) },
    };
    let _ = app.emit(PLAN_READY, payload);
}

#[tauri::command]
pub(crate) fn migrate_actions(
    state: State<AppState>,
    app: AppHandle,
    left: SourceSpec,
    right: SourceSpec,
    opts: CompareOpts,
    flags: MigrateFlags,
    token: u64,
) -> Result<(), String> {
    state.plan_token.store(token, Ordering::SeqCst);
    let filters = FilterSet::new(&opts.include, &opts.exclude).map_err(|e| e.to_string())?;
    let cfg = CompareConfig {
        method: parse_method(&opts.method),
        recursive: true,
        filters,
    };
    let l = cached_source(&state, &left)?;
    let r = cached_source(&state, &right)?;
    let plan_token = Arc::clone(&state.plan_token);
    std::thread::spawn(move || {
        run_plan(&app, l.as_ref(), r.as_ref(), &cfg, "migrate", token, &plan_token, |root| {
            let mut actions = Vec::new();
            collect_migrate(root, flags, &mut actions);
            actions
        });
    });
    Ok(())
}

/// Plan a bidirectional sync: a FULL recursive comparison, then collect the directional actions per the
/// trust flags + conflict rule. Returns `MigrateAction`s (reused — `op` carries the direction); the UI
/// previews them and applies via `migrate_apply` (with `delete_origin = false`).
#[tauri::command]
pub(crate) fn sync_actions(
    state: State<AppState>,
    app: AppHandle,
    left: SourceSpec,
    right: SourceSpec,
    opts: CompareOpts,
    flags: SyncFlags,
    token: u64,
) -> Result<(), String> {
    state.plan_token.store(token, Ordering::SeqCst);
    let filters = FilterSet::new(&opts.include, &opts.exclude).map_err(|e| e.to_string())?;
    let cfg = CompareConfig {
        method: parse_method(&opts.method),
        recursive: true,
        filters,
    };
    let l = cached_source(&state, &left)?;
    let r = cached_source(&state, &right)?;
    let plan_token = Arc::clone(&state.plan_token);
    std::thread::spawn(move || {
        run_plan(&app, l.as_ref(), r.as_ref(), &cfg, "sync", token, &plan_token, |root| {
            let mut actions = Vec::new();
            collect_sync(root, flags, &mut actions);
            actions
        });
    });
    Ok(())
}
