//! Synchronization actions over the VFS: copy/delete items left↔right, with dry-run planning.
//!
//! Actions are expressed at the item level (a file or a directory) and executed through
//! [`confold_vfs::SourceMut`], so they work for any backend (local now; SMB/S3 later). A directory copy is
//! expanded recursively into per-file operations; the returned [`ActionOutcome`]s report each concrete
//! operation. With `dry_run = true`, nothing is written — the same outcome list is produced as a preview.

use confold_vfs::{RelPath, SourceError, SourceMut};
use serde::{Deserialize, Serialize};

/// A synchronization operation requested on one compared item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOp {
    /// Copy the item from the left side to the right (create/overwrite on the right).
    CopyLeftToRight,
    /// Copy the item from the right side to the left.
    CopyRightToLeft,
    /// Delete the item on the left side.
    DeleteLeft,
    /// Delete the item on the right side.
    DeleteRight,
}

/// A requested action on a single compared item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncAction {
    /// Path of the item, relative to the compared roots.
    pub rel_path: RelPath,
    /// What to do with it.
    pub op: SyncOp,
    /// Whether the item is a directory (drives recursive expansion for copies).
    pub is_dir: bool,
}

/// The result of one concrete operation (a single file copy, directory creation, or removal).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOutcome {
    /// Path the operation acted on.
    pub rel_path: RelPath,
    /// The originating operation kind.
    pub op: SyncOp,
    /// `true` if it succeeded (or would succeed, in a dry run).
    pub ok: bool,
    /// Error message if it failed.
    pub error: Option<String>,
}

/// Execute (or, with `dry_run`, preview) a batch of sync actions between two mutable sources.
///
/// Returns one [`ActionOutcome`] per concrete operation, in order. A failure on one operation is
/// recorded and does not abort the rest.
pub fn apply(
    left: &dyn SourceMut,
    right: &dyn SourceMut,
    actions: &[SyncAction],
    dry_run: bool,
) -> Vec<ActionOutcome> {
    // Drop actions already covered by an ancestor directory action of the same op (a recursive copy or
    // delete of a dir handles its descendants) — avoids redundant copies and "not found" on deletes.
    let effective = dedup_covered(actions);
    let mut out = Vec::new();
    for action in &effective {
        match action.op {
            SyncOp::CopyLeftToRight => copy(
                left,
                right,
                &action.rel_path,
                action.is_dir,
                action.op,
                dry_run,
                &mut out,
            ),
            SyncOp::CopyRightToLeft => copy(
                right,
                left,
                &action.rel_path,
                action.is_dir,
                action.op,
                dry_run,
                &mut out,
            ),
            SyncOp::DeleteLeft => delete(left, &action.rel_path, action.op, dry_run, &mut out),
            SyncOp::DeleteRight => delete(right, &action.rel_path, action.op, dry_run, &mut out),
        }
    }
    out
}

/// Recursively copy `rel` from `from` to `to`. Directories expand into per-file operations.
fn copy(
    from: &dyn SourceMut,
    to: &dyn SourceMut,
    rel: &RelPath,
    is_dir: bool,
    op: SyncOp,
    dry_run: bool,
    out: &mut Vec<ActionOutcome>,
) {
    if is_dir {
        let result = if dry_run {
            Ok(())
        } else {
            to.create_dir_all(rel)
        };
        push(out, rel, op, result);
        match from.read_dir(rel) {
            Ok(mut entries) => {
                entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
                for entry in entries {
                    copy(
                        from,
                        to,
                        &entry.rel_path,
                        entry.kind.is_dir(),
                        op,
                        dry_run,
                        out,
                    );
                }
            }
            Err(e) => push(out, rel, op, Err(e)),
        }
    } else {
        let result = if dry_run {
            Ok(())
        } else {
            from.open(rel)
                .and_then(|reader| to.copy_from(rel, reader.as_ref()))
        };
        push(out, rel, op, result);
    }
}

/// Delete `rel` on `target` (recursive for directories — [`SourceMut::remove`] handles that).
fn delete(
    target: &dyn SourceMut,
    rel: &RelPath,
    op: SyncOp,
    dry_run: bool,
    out: &mut Vec<ActionOutcome>,
) {
    let result = if dry_run { Ok(()) } else { target.remove(rel) };
    push(out, rel, op, result);
}

fn push(out: &mut Vec<ActionOutcome>, rel: &RelPath, op: SyncOp, result: Result<(), SourceError>) {
    out.push(ActionOutcome {
        rel_path: rel.clone(),
        op,
        ok: result.is_ok(),
        error: result.err().map(|e| e.to_string()),
    });
}

/// Keep only actions not already covered by another action that is a directory with the same op and an
/// ancestor path (that recursive dir op already handles the descendant).
fn dedup_covered(actions: &[SyncAction]) -> Vec<SyncAction> {
    actions
        .iter()
        .filter(|a| {
            !actions.iter().any(|other| {
                other.op == a.op
                    && other.is_dir
                    && is_strict_descendant(&a.rel_path, &other.rel_path)
            })
        })
        .cloned()
        .collect()
}

/// `true` if `child` is strictly longer than `ancestor` and shares its full prefix.
fn is_strict_descendant(child: &RelPath, ancestor: &RelPath) -> bool {
    let a = ancestor.components();
    let c = child.components();
    c.len() > a.len() && &c[..a.len()] == a
}
