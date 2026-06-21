// Pure Migrate (M1) domain logic: plan/progress arithmetic and the checked/skipped split. Kept free of
// Svelte runes and Tauri so it can be unit-tested directly; +page.svelte holds only thin `$derived`
// wrappers and the I/O wiring around these functions.

export type MigrateReason = "new" | "different" | "extra" | "moved";
export type MigrateAction = {
  rel_path: string[];
  op: string;
  is_dir: boolean;
  reason: MigrateReason;
  item_count: number;
};
export type MigrateProgressEvt = {
  generation: number;
  rel_path: string[];
  op: string;
  reason: MigrateReason;
  ok: boolean;
  error: string | null;
};
export type MigrateSummary = { total: number; ok: number; failed: number; cancelled: boolean };
/** MOVE (M2) result: the outcome of deleting the origin after a verified-complete migration. */
export type MoveSummary = {
  /** True once the move was attempted (apply was clean). False → never reached the verify gate. */
  attempted: boolean;
  /** True if the origin was actually deleted (every item verified identical, delete ran to the end). */
  origin_deleted: boolean;
  files_deleted: number;
  dirs_pruned: number;
  failed: number;
  cancelled: boolean;
  /** If the move did NOT proceed, the origin items that blocked it (capped for display). */
  blockers: string[];
};
export type MigrateOutcome = {
  path: string;
  reason: MigrateReason;
  ok: boolean;
  error: string | null;
  skipped?: boolean;
  /** The concrete op (carries direction: copy_left_to_right / copy_right_to_left / delete_*). Optional
   *  so pure-arithmetic call sites don't have to supply it; the progress UI uses it for the ←/→ label. */
  op?: string;
};

/** The checkbox identity of a plan action: its relative path joined with "/". */
export function actionKey(a: MigrateAction): string {
  return a.rel_path.join("/");
}

/** Parse the comma-separated exclude field into trimmed, non-empty globs. */
export function parseExclude(s: string): string[] {
  return s
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);
}

/** Sum of `item_count` over actions of a given reason (so a new dir of 45 files counts as 45, not 1). */
export function planItems(actions: MigrateAction[], reason: MigrateReason): number {
  return actions.filter((a) => a.reason === reason).reduce((s, a) => s + a.item_count, 0);
}

/** Number of plan actions currently unchecked (i.e. that will be skipped on apply). */
export function uncheckedCount(actions: MigrateAction[], checked: ReadonlySet<string>): number {
  return actions.filter((a) => !checked.has(actionKey(a))).length;
}

/** Sum of `item_count` over the *checked* actions — the reference total shown while applying. */
export function checkedItemTotal(actions: MigrateAction[], checked: ReadonlySet<string>): number {
  return actions
    .filter((a) => checked.has(actionKey(a)))
    .reduce((s, a) => s + a.item_count, 0);
}

/**
 * Split a plan by its checkbox state: the checked actions to send to the backend, plus pre-built
 * `skipped` outcomes for the unchecked ones (shown immediately in the progress list, greyed out).
 */
export function splitChecked(
  actions: MigrateAction[],
  checked: ReadonlySet<string>,
): { toApply: MigrateAction[]; skipped: MigrateOutcome[] } {
  const toApply: MigrateAction[] = [];
  const skipped: MigrateOutcome[] = [];
  for (const a of actions) {
    if (checked.has(actionKey(a))) {
      toApply.push(a);
    } else {
      skipped.push({ path: actionKey(a), reason: a.reason, ok: true, error: null, skipped: true, op: a.op });
    }
  }
  return { toApply, skipped };
}

/** Live per-category progress count: applied (non-skipped) outcomes of a given reason. */
export function appliedByReason(outcomes: MigrateOutcome[], reason: MigrateReason): number {
  return outcomes.filter((o) => o.reason === reason && !o.skipped).length;
}

/** Count of outcomes that were actually applied (not skipped). */
export function appliedCount(outcomes: MigrateOutcome[]): number {
  return outcomes.filter((o) => !o.skipped).length;
}

/** Count of outcomes that were skipped (unchecked items pre-populated at apply time). */
export function skippedCount(outcomes: MigrateOutcome[]): number {
  return outcomes.filter((o) => o.skipped).length;
}
