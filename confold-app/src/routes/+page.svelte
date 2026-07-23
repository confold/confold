<script lang="ts">
  import { commands } from "$lib/commands";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { DiffReport, DiffStatus, CompareOpts, SyncAction, ActionOutcome, SyncOp, DiffEntry, DiffResult, DiffFileResult, LargeDiffResult, FileDiffHunks, FileMeta, SourceSpec, FileRef, SourceTypeInfo } from "$lib/types";

  import {
    parseExclude, checkedItemTotal, splitChecked, uncheckedCount,
    type MigrateReason, type MigrateAction, type MigrateProgressEvt, type MigrateSummary, type MigrateOutcome,
    type MoveSummary,
  } from "$lib/migrate";
  import { isMetadataMethod, metadataVerdict, matchesGlob, excludeRemovalNeedsRefetch } from "$lib/recompute";
  import { slugOf, sourcesEqual, stripSecrets, missingRequired } from "$lib/sources";
  import { selection, clearSelection, toggle } from "$lib/selection.svelte";
  import { hidden, toggleStatus, nodeVisible } from "$lib/filter.svelte";
  import { sortState, sortEntries } from "$lib/sort.svelte";
  import { isOpen, toggleOpen, clearExpanded, expanded } from "$lib/expand.svelte";
  import { SvelteSet } from "svelte/reactivity";
  import DiffNode from "./DiffNode.svelte";
  import MigrateConfigPanel from "./MigrateConfigPanel.svelte";
  import SideBySideSkeleton from "./SideBySideSkeleton.svelte";
  import MigratePlanModal from "./MigratePlanModal.svelte";
  import MigrateProgressModal from "./MigrateProgressModal.svelte";
  import SyncConfigPanel from "./SyncConfigPanel.svelte";
  import SyncPlanModal from "./SyncPlanModal.svelte";
  import SideBySide from "./SideBySide.svelte";
  import HexView from "./HexView.svelte";
  import ImageView from "./ImageView.svelte";
  import SourcePicker from "./SourcePicker.svelte";
  import { isImagePath } from "$lib/imagediff";
  import { isComparing, hunksToFileDiff, splitPath, fileRefOf, refKey, makeOpts } from "$lib/utils/compare-helpers";

  // Origen / Destino sources (any type). Replaces the old Left/Right path inputs.
  let originSpec = $state<SourceSpec | null>(null);
  let destSpec = $state<SourceSpec | null>(null);
  let originRecents = $state<{ spec: SourceSpec; isDir: boolean; stale?: boolean }[]>([]);
  let destRecents = $state<{ spec: SourceSpec; isDir: boolean; stale?: boolean }[]>([]);
  let originIsDir = $state<boolean | null>(null);
  let destIsDir = $state<boolean | null>(null);
  let originStale = $state(false);
  let destStale = $state(false);
  let cancelConfirm = $state(false); // confirm leaving a comparison back to source selection
  let picking = $state<"origin" | "dest" | null>(null); // which side's modal is open
  let recentPrefill = $state<SourceSpec | null>(null); // a reused recent missing its secret → pre-fill the picker
  let appVersion = $state(""); // "vX.Y.Z" shown next to the title; from Tauri's getVersion() (tauri.conf)
  let sourceTypes = $state<SourceTypeInfo[]>([]); // catalog from source_types()
  let method = $state<CompareOpts["method"]>("quick");
  let lazy = $state(true);
  let exclude = $state("");
  let dateTol = $state(2000); // ms tolerance for mtime orange highlight (0 = exact)
  type AppMode = "compare" | "sync" | "migrate";
  let mode = $state<AppMode>("compare");
  let modeMenuOpen = $state(false);
  const modeLabel: Record<AppMode, string> = { compare: "Compare", sync: "Sync", migrate: "Migrate" };
  const modeDesc: Record<AppMode, string> = {
    compare:  "Explore differences manually and copy/delete individual files.",
    sync:     "Bidirectional sync — trust one or both sides, with conflict-resolution rules.",
    migrate:  "Reconcile destination to match source (copy / overwrite / delete), one direction.",
  };

  // ---- Migrate (M1) config panel state ----
  let migratePanel = $state(false); // the config modal is open
  let migrateMethod = $state<CompareOpts["method"]>("full"); // full by default — slow but safe
  let migrateExclude = $state(""); // exceptions: globs left untouched on BOTH sides
  let migrateFlags = $state({ copy_new: true, overwrite_different: true, delete_extra: false, delete_origin: false });
  // Sync (bidirectional) config. Shares the plan/apply machinery (migrateActions/migrateChecked/progress);
  // `activeFlow` tells the shared apply whether this run is a migrate (move-capable) or a sync.
  let syncFlags = $state<{ trust_left: boolean; trust_right: boolean; delete_diffs: boolean; conflict_rule: "newer" | "larger" | "manual" }>(
    // Default conflict resolution = manual (least destructive — never auto-overwrites a conflicting file).
    { trust_left: true, trust_right: true, delete_diffs: false, conflict_rule: "manual" },
  );
  let syncBackupAck = $state(false);
  let syncPanel = $state(false);
  let syncPlanOpen = $state(false);
  let activeFlow = $state<"migrate" | "sync">("migrate");
  // Plan-list scroll positions, held here so they survive opening a file from the plan and returning.
  let migratePlanScroll = $state(0);
  let syncPlanScroll = $state(0);
  let migrateBackupAck = $state(false); // "I have a backup of both sides" — gates the (future) apply
  let migrateBusy = $state(false); // computing the plan (background thread; feedback via plan-progress)
  let planToken = $state(0); // bumped per preview; the plan-ready/plan-progress listeners ignore stale tokens
  let planProgress = $state(0); // live "examined N items" count streamed while the plan is computed
  // Planned migrate actions (from migrate_actions, written by previewMigrate, consumed by M1.4 plan modal
  // and M1.5 apply). null = not yet computed; [] = computed but nothing to do.
  let migrateActions = $state<MigrateAction[] | null>(null);
  let migratePlanOpen = $state(false);

  // Progress state (M1.5)
  let migrateProgressOpen = $state(false);
  let migrateApplying = $state(false);
  let migrateGeneration = $state(0);
  let migrateOutcomes = $state<MigrateOutcome[]>([]);
  // Apply-progress batching (see `queueOutcome`): coalesce the per-file `migrate-progress` storm so the
  // outcome list + its derived counts re-render ~once/frame instead of once/event (same fix as compare
  // verdicts). Plain (non-$state) buffer; the flush reassigns the $state `migrateOutcomes` once per frame.
  let pendingOutcomes: MigrateOutcome[] = [];
  let outcomeFlushScheduled = false;
  // True while the non-lazy preload auto-expands folders in the background — drives the toolbar spinner so
  // the app doesn't look idle/frozen while deep (collapsed) subtrees are still loading.
  let preloading = $state(false);
  let migrateDoneSummary = $state<MigrateSummary | null>(null);
  // MOVE (M2): result of the post-apply origin-delete (null unless a move was requested). And which
  // long phase the backend is in, so the progress modal can show "Verifying…" / "Emptying origin…".
  let migrateMoveSummary = $state<MoveSummary | null>(null);
  let migratePhase = $state<"" | "verifying" | "emptying_origin">("");
  // Checked items in the plan (SvelteSet for reactive mutations). Initialized from migrateActions in previewMigrate.
  const migrateChecked = new SvelteSet<string>();
  // Any destructive op selected → the backup acknowledgement is required (for the apply step).
  const migrateDestructive = $derived(migrateFlags.overwrite_different || migrateFlags.delete_extra);

  let report = $state<DiffReport | null>(null);
  let error = $state("");
  let loading = $state(false);
  // When a file is opened from the migrate plan modal, track where to return on close.
  let afterFileBack = $state<"plan" | null>(null);
  // The plan action currently being reviewed in read-only mode (used by Keep/Skip to update migrateChecked).
  let planViewingAction = $state<MigrateAction | null>(null);

  // Lazy comparison: loaded children are spliced directly into the report tree (single source of
  // truth — `report` is $state, so deep mutations are reactive). `loaded` marks dirs whose level has
  // been fetched (so we don't refetch, and empty dirs aren't mistaken for unloaded); `pendingLoads`
  // marks in-flight compare_level requests so the row shows a spinner. Both are reactive sets.
  const pendingLoads = new SvelteSet<string>();
  const loaded = new SvelteSet<string>();

  // Streaming verdicts: each scan returns its level's listing immediately (files as "comparing"
  // placeholders) and resolves file verdicts in the background, pushing `entry-resolved` events. We
  // tag every scan with `scanToken` (bumped per new comparison) so stale events are ignored, both
  // here and in the backend's resolver. A verdict that arrives before its level has been spliced into
  // the tree is parked in `verdictBuffer` and applied when the level lands (drainVerdicts).
  let scanToken = $state(0);
  type Verdict = { status: DiffStatus; detail: string | null };
  const verdictBuffer = new Map<string, Verdict>();
  // Per-frame coalescing of streamed verdicts (see `queueVerdict`): the backend emits one event per file,
  // so applying each immediately would re-run the whole-tree deriveds (liveSummary, rows) N times → O(N²).
  const pendingVerdicts = new Map<string, Verdict>();
  let verdictFlushScheduled = false;

  // Is this a file whose verdict is still being computed in the background?
  // (isComparing is imported from $lib/utils/compare-helpers)
  // Patch a streamed verdict onto its file node in the tree. Returns false if the node isn't present
  // yet (its level hasn't been spliced) so the caller can buffer it.
  function applyVerdict(key: string, v: Verdict): boolean {
    if (!report) return false;
    const parts = key.split("/");
    let node: DiffEntry = report.root;
    for (let i = 0; i < parts.length - 1; i++) {
      const next = node.children.find((c) => c.name === parts[i]);
      if (!next) return false;
      node = next;
    }
    const leaf = node.children.find((c) => c.name === parts[parts.length - 1]);
    if (!leaf) return false;
    leaf.status = v.status;
    leaf.detail = v.detail;
    return true;
  }
  function onVerdict(key: string, v: Verdict) {
    if (!applyVerdict(key, v)) verdictBuffer.set(key, v);
  }
  // After a level is spliced in, apply any verdicts that arrived before it (race between the listing
  // response and the background events).
  function drainVerdicts() {
    for (const [key, v] of verdictBuffer) {
      if (applyVerdict(key, v)) verdictBuffer.delete(key);
    }
  }
  // Coalesce streamed verdicts into one batch per animation frame. The backend emits one `entry-resolved`
  // per file; applying each as it arrives mutates the $state tree and re-runs the whole-tree deriveds
  // (`liveSummary`, `rows`) every event → O(N²) at thousands of files. Buffering and flushing once per
  // frame collapses that to ~one recompute/frame regardless of arrival rate — and it's self-tuning: a
  // slow/large file that lands alone in a frame still updates immediately, while a burst of small files
  // coalesces. `setTimeout` fallback covers environments without rAF (tests) and throttled background tabs.
  function flushVerdicts() {
    verdictFlushScheduled = false;
    for (const [key, v] of pendingVerdicts) onVerdict(key, v);
    pendingVerdicts.clear();
  }
  function queueVerdict(key: string, v: Verdict) {
    pendingVerdicts.set(key, v); // last-write-wins per file; a file's verdict is terminal
    if (verdictFlushScheduled) return;
    verdictFlushScheduled = true;
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(flushVerdicts);
    else setTimeout(flushVerdicts, 50);
  }

  // The side-by-side diff result (file-diff mode); folder-compare produces `report` instead. Whether a
  // comparison is folder-vs-folder or file-vs-file is inferred from each side's kind (no manual toggle).
  let diffResult = $state<DiffResult | null>(null);
  let filesMeta = $state<{ left: FileMeta; right: FileMeta } | null>(null);
  // A file opened from a folder-tree row (double-click) → side-by-side without leaving folders mode.
  // Discriminated union: loading=true immediately on click (shows skeleton), loading=false when diff
  // arrives (normal text/binary), hunks=true for large files shown in read-only paginated hunk mode.
  type OpenedFileBase = { name: string; left: FileRef; right: FileRef; leftDate: number | null; rightDate: number | null; leftCreated: number | null; rightCreated: number | null };
  type OpenedFile =
    | (OpenedFileBase & { loading: true })
    | (OpenedFileBase & { loading: false; result: DiffResult; leftMeta: FileMeta; rightMeta: FileMeta })
    | (OpenedFileBase & { loading: false; hunks: FileDiffHunks; leftMeta: FileMeta; rightMeta: FileMeta });
  let openedFile = $state<OpenedFile | null>(null);
  let sbsExitHandler: (() => void) | null = $state(null);

  // Large-file warning dialog state.
  let largeFileConfirm = $state<{ name: string; left: FileRef; right: FileRef; leftSize: number; rightSize: number } | null>(null);

  // Sync-action confirmation flow.
  let plan = $state<ActionOutcome[] | null>(null);
  let pendingActions = $state<SyncAction[] | null>(null);
  let pendingLabel = $state("");
  let applying = $state(false);
  let resultMsg = $state("");
  let msgTimer: ReturnType<typeof setTimeout> | undefined;
  let shellMsg = $state("");
  let shellMsgTimer: ReturnType<typeof setTimeout> | undefined;
  function flashShell(m: string) {
    shellMsg = m;
    clearTimeout(shellMsgTimer);
    shellMsgTimer = setTimeout(() => (shellMsg = ""), 4000);
  }

  // Show a transient banner that auto-dismisses (so a confirmation doesn't linger forever).
  function flashMsg(m: string) {
    resultMsg = m;
    clearTimeout(msgTimer);
    msgTimer = setTimeout(() => (resultMsg = ""), 4000);
  }

  // Inferred from each side's probe (the picker's connection test): a file↔folder mix can't be compared;
  // both files → file diff; otherwise (both folders, or kind not yet known) → folder compare.
  const kindMismatch = $derived(originIsDir !== null && destIsDir !== null && originIsDir !== destIsDir);
  const bothFiles = $derived(originIsDir === false && destIsDir === false);
  const sameSource = $derived(originSpec !== null && destSpec !== null && sourcesEqual(originSpec, destSpec, sourceTypes));
  // Why a comparison can't run (shown as a warning + disables Compare), or null if it's fine.
  const invalidReason = $derived(
    kindMismatch
      ? "A file and a folder can't be compared — pick two of the same kind."
      : sameSource
        ? "Source and destination are the same — pick two different locations."
        : null,
  );
  const canCompare = $derived(originSpec !== null && destSpec !== null && !loading && invalidReason === null);
  // Migrate needs two directories on different locations (not a file-vs-file pair).
  const canMigrate = $derived(canCompare && !bothFiles);
  // Sync needs two directories too (it reconciles trees), same precondition as Migrate.
  const canSync = $derived(canMigrate);
  // Whether the primary (left) split-button action can run in the current mode.
  const canRun = $derived(mode === "compare" ? canCompare : mode === "sync" ? canSync : canMigrate);
  // `loading` is included so the comparison view (compact bar + skeleton) shows *immediately* on Compare,
  // before the (possibly slow, e.g. SFTP) results arrive — perceived agility.
  const comparing = $derived(report !== null || diffResult !== null || loading);
  const selectedCount = $derived(selection.size);

  // Context-aware action availability: an op is offered only if every selected item supports it.
  const sel = $derived([...selection.values()]);
  const canCopyLR = $derived(sel.length > 0 && sel.every((e) => e.left !== null));
  const canCopyRL = $derived(sel.length > 0 && sel.every((e) => e.right !== null));
  const canDelL = $derived(sel.length > 0 && sel.every((e) => e.left !== null));
  const canDelR = $derived(sel.length > 0 && sel.every((e) => e.right !== null));

  // Fetch the source-type catalog + restore persisted recents from ~/.cache/confold/recents.json.
  onMount(async () => {
    try {
      sourceTypes = await commands.sourceTypes();
    } catch (e) {
      error = String(e);
    }
    try {
      const data = await commands.loadRecents();
      if (data.origins.length > 0 || data.destinations.length > 0) {
        originRecents = data.origins;
        destRecents = data.destinations;
      } else {
        const restore = (raw: string | null): { spec: SourceSpec; isDir: boolean }[] =>
          raw
            ? (JSON.parse(raw) as { spec: SourceSpec; isDir: boolean }[]).filter(
                (r) => typeof r?.spec?.kind === "string" && r.spec.fields != null && typeof r.spec.fields === "object",
              )
            : [];
        const oldO = restore(localStorage.getItem("confold-origin-recents"));
        const oldD = restore(localStorage.getItem("confold-dest-recents"));
        if (oldO.length > 0 || oldD.length > 0) {
          originRecents = oldO;
          destRecents = oldD;
          persistRecents();
          localStorage.removeItem("confold-origin-recents");
          localStorage.removeItem("confold-dest-recents");
        }
      }
    } catch { /* non-Tauri environment — recents unavailable */ }
  });

  // Streamed file verdicts from the backend. Ignore events from superseded comparisons (stale token).
  type EntryResolvedEvent = { token: number; rel_path: string[]; status: DiffStatus; detail: string | null };
  onMount(() => {
    let unlisten: UnlistenFn | undefined;
    listen<EntryResolvedEvent>("entry-resolved", (e) => {
      const p = e.payload;
      if (p.token !== scanToken) return;
      queueVerdict(p.rel_path.join("/"), { status: p.status, detail: p.detail });
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  });

  // Streamed migrate/sync plan computation (runs on a backend thread). `plan-progress` updates the live
  // counter; `plan-ready` carries the finished action list (or an error) and opens the right plan modal.
  type PlanProgressEvent = { token: number; examined: number };
  type PlanReadyEvent = { token: number; flow: "migrate" | "sync"; actions: MigrateAction[]; error: string | null };
  onMount(() => {
    let unlistenProgress: UnlistenFn | undefined;
    let unlistenReady: UnlistenFn | undefined;
    listen<PlanProgressEvent>("plan-progress", (e) => {
      if (e.payload.token === planToken) planProgress = e.payload.examined;
    }).then((u) => (unlistenProgress = u));
    listen<PlanReadyEvent>("plan-ready", (e) => {
      const p = e.payload;
      if (p.token !== planToken) return; // a newer preview superseded this one
      migrateBusy = false;
      if (p.error) { error = p.error; return; }
      migrateActions = p.actions;
      migrateChecked.clear();
      for (const a of p.actions) migrateChecked.add(a.rel_path.join("/"));
      if (p.flow === "sync") { syncPlanScroll = 0; syncPlanOpen = true; }
      else { migratePlanScroll = 0; migratePlanOpen = true; }
    }).then((u) => (unlistenReady = u));
    return () => { unlistenProgress?.(); unlistenReady?.(); };
  });

  // App version shown next to the title. Dynamic import + guard so non-Tauri envs (tests, the e2e shim)
  // simply show no label instead of erroring. Source of truth = tauri.conf.json `version`.
  onMount(async () => {
    try {
      const { getVersion } = await import("@tauri-apps/api/app");
      const v = await getVersion();
      if (v) appVersion = `v${v}`;
    } catch { /* non-Tauri environment — no version label */ }
  });

  // Deep-link handler: confold://compare?origin=<path>&destination=<path>
  // Fired by OS context-menu entries (Quick Action / .desktop / registry verb).
  onMount(() => {
    let unlisten: UnlistenFn | undefined;
    import("@tauri-apps/plugin-deep-link")
      .then(async ({ getCurrent, onOpenUrl }) => {
        const handleUrls = async (urls: string[]) => {
          for (const raw of urls) {
            try {
              const url = new URL(raw);
              if (url.protocol !== "confold:" || url.host !== "compare") continue;
              const o = url.searchParams.get("origin");
              const d = url.searchParams.get("destination");
              if (o && d) {
                const oSpec: SourceSpec = { kind: "fs", fields: { root: o } };
                const dSpec: SourceSpec = { kind: "fs", fields: { root: d } };
                originSpec = oSpec;
                destSpec = dSpec;
                const [oTest, dTest] = await Promise.all([
                  commands.testSource(oSpec).catch(() => null),
                  commands.testSource(dSpec).catch(() => null),
                ]);
                originStale = oTest?.ok !== true;
                destStale = dTest?.ok !== true;
                originIsDir = oTest?.ok ? oTest.is_dir : null;
                destIsDir = dTest?.ok ? dTest.is_dir : null;
                if (oTest?.ok && dTest?.ok) {
                  originRecents = prependRecent(originRecents, oSpec, oTest.is_dir);
                  destRecents = prependRecent(destRecents, dSpec, dTest.is_dir);
                  persistRecents();
                  openedFile = null;
                  mode = "compare";
                  await tick();
                  await run();
                }
              } else if (o) {
                const oSpec: SourceSpec = { kind: "fs", fields: { root: o } };
                originSpec = oSpec;
                const oTest = await commands.testSource(oSpec).catch(() => null);
                originStale = oTest?.ok !== true;
                originIsDir = oTest?.ok ? oTest.is_dir : null;
                if (oTest?.ok) {
                  originRecents = prependRecent(originRecents, oSpec, oTest.is_dir);
                  persistRecents();
                }
              }
            } catch { /* malformed URL — ignore */ }
          }
        };

        const startUrls = await getCurrent();
        if (startUrls) handleUrls(startUrls);

        onOpenUrl((urls) => handleUrls(urls)).then((u) => (unlisten = u));
      })
      .catch(() => { /* non-Tauri environment — deep-link unavailable */ });
    return () => unlisten?.();
  });

  // Streamed migrate progress events + final done event from the background thread.
  onMount(() => {
    let unlistenProgress: UnlistenFn | undefined;
    let unlistenDone: UnlistenFn | undefined;
    let unlistenPhase: UnlistenFn | undefined;

    listen<MigrateProgressEvt>("migrate-progress", (e) => {
      const p = e.payload;
      if (p.generation !== migrateGeneration) return;
      queueOutcome({ path: p.rel_path.join("/"), reason: p.reason, ok: p.ok, error: p.error, op: p.op });
    }).then((u) => (unlistenProgress = u));

    listen<{ generation: number; phase: "verifying" | "emptying_origin" }>("migrate-phase", (e) => {
      if (e.payload.generation !== migrateGeneration) return;
      migratePhase = e.payload.phase;
    }).then((u) => (unlistenPhase = u));

    listen<{ generation: number; summary: MigrateSummary; move_result?: MoveSummary }>("migrate-done", (e) => {
      const p = e.payload;
      if (p.generation !== migrateGeneration) return;
      flushOutcomes(); // apply any buffered outcomes before the summary lands, so the list is complete
      migrateDoneSummary = p.summary;
      migrateMoveSummary = p.move_result ?? null;
      migratePhase = "";
      migrateApplying = false;
    }).then((u) => (unlistenDone = u));

    return () => { unlistenProgress?.(); unlistenDone?.(); unlistenPhase?.(); };
  });

  // A source was configured in the picker → set the side + its kind, remember it (session recents), close.
  function onPicked(spec: SourceSpec, isDir: boolean) {
    if (picking === "origin") {
      originSpec = spec;
      originIsDir = isDir;
      originStale = false;
      originRecents = prependRecent(originRecents, spec, isDir);
    } else if (picking === "dest") {
      destSpec = spec;
      destIsDir = isDir;
      destStale = false;
      destRecents = prependRecent(destRecents, spec, isDir);
    }
    picking = null;
    recentPrefill = null;
    persistRecents();
  }
  function prependRecent(
    list: { spec: SourceSpec; isDir: boolean; stale?: boolean }[],
    spec: SourceSpec,
    isDir: boolean,
  ) {
    const slug = slugOf(spec).label;
    return [{ spec, isDir }, ...list.filter((r) => slugOf(r.spec).label !== slug)].slice(0, 5);
  }
  function persistRecents() {
    const stripped = (list: typeof originRecents) =>
      list.map((r) => ({ spec: stripSecrets(r.spec, sourceTypes), isDir: r.isDir }));
    commands.saveRecents(stripped(originRecents), stripped(destRecents)).catch(() => {});
  }

  // Use a recent: if its (persisted, secret-stripped) spec is missing a required field — i.e. a credential
  // that we deliberately didn't store — re-open the picker pre-filled so the user re-enters just the secret;
  // otherwise apply it directly.
  async function pickRecent(r: { spec: SourceSpec; isDir: boolean; stale?: boolean }, side: "origin" | "dest") {
    const info = sourceTypes.find((t) => t.id === r.spec.kind);
    if (info && missingRequired(info, r.spec.fields).length > 0) {
      recentPrefill = r.spec;
      picking = side;
      return;
    }
    const stale = r.spec.kind === "fs"
      ? !(await commands.pathExists(r.spec.fields.root ?? "").catch(() => true))
      : !!r.stale;
    if (side === "origin") { originSpec = r.spec; originIsDir = r.isDir; originStale = stale; }
    else { destSpec = r.spec; destIsDir = r.isDir; destStale = stale; }
  }

  // Split an absolute/remote path into [parent dir, filename] (handles `/` and `\`).
  // Convert a FileDiffHunks (paginated) into a FileDiff understood by SideBySide.
  // hunksToFileDiff, splitPath, fileRefOf, refKey, makeOpts: imported from $lib/utils/compare-helpers

  async function runCompare() {
    if (!originSpec || !destSpec) return;
    error = "";
    resultMsg = "";
    loading = true;
    loaded.clear();
    pendingLoads.clear();
    verdictBuffer.clear();
    pendingVerdicts.clear();
    clearExpanded();
    const token = ++scanToken; // new scan: cancels stale background resolvers + stale verdict events
    // NB: don't null `report` here — a refresh (e.g. after saving from the side-by-side) must keep the
    // surrounding `{#if report}` block mounted, or the open file view would be torn down and lose its
    // in-memory merge. `run()` nulls it explicitly for a genuinely new comparison.
    clearSelection();
    try {
      const opts = makeOpts(method, exclude);
      report = await commands.compare(originSpec, destSpec, opts, token);
      drainVerdicts(); // apply any top-level verdicts that arrived before the listing landed
      // Sync baseline and clear client-side state: the fresh scan is now the source of truth.
      prevExcludePatterns = parseExclude(exclude);
      lastComparedMethod = method;
      savedVerdicts.clear();
      savedContentVerdicts.clear();
      // If a re-scan (autorefresh) dropped the folder we were drilled into, fall back to the root so the
      // tree isn't left blank with a stale breadcrumb pointing at a path that no longer resolves.
      if (focusPath.length && !entryAtPath(report.root, focusPath)) focusPath = [];
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
    // Preload mode: auto-expand all pending dirs in background so the tree fills in progressively.
    if (!lazy && report) preloadAll(report.root.children);
  }

  async function runDiff() {
    if (!originSpec || !destSpec) return;
    error = "";
    resultMsg = "";
    loading = true;
    diffResult = null;
    filesMeta = null;
    try {
      const r = await commands.diffFile(fileRefOf(originSpec), fileRefOf(destSpec));
      diffResult = r.result;
      filesMeta = { left: r.left, right: r.right };
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function run() {
    if (invalidReason) {
      error = invalidReason;
      return;
    }
    report = null;
    diffResult = null;
    openedFile = null;
    focusPath = [];
    scrollTop = 0; // reset the virtual-scroll window for the new comparison (a stale scrollTop left the
    // first render blank until the user scrolled, when the new tree was shorter than the old scroll offset)
    loaded.clear();
    pendingLoads.clear();
    clearExpanded();
    if (bothFiles) await runDiff();
    else await runCompare();
  }

  // The split button's primary action depends on the selected mode.
  function primaryAction() {
    if (mode === "migrate") {
      activeFlow = "migrate";
      migrateBackupAck = false;
      migrateActions = null;
      migratePanel = true;
    } else if (mode === "sync") {
      activeFlow = "sync";
      syncBackupAck = false;
      migrateActions = null;
      syncPanel = true;
    } else {
      run();
    }
  }

  // Sync dry-run: compute the directional action list and open the sync plan review. Writes nothing.
  // The compute runs on a backend thread (heavy + uses S3/SFTP block_on); we fire it and let the
  // `plan-ready` listener open the modal — so the UI stays responsive and shows the "Preparing plan…"
  // overlay with a live counter. `planToken` ignores a superseded preview's late result.
  function previewSync() {
    if (!originSpec || !destSpec) return;
    error = "";
    migrateActions = null;
    syncPanel = false;
    const token = ++planToken;
    planProgress = 0;
    migrateBusy = true;
    const opts = makeOpts(migrateMethod, migrateExclude);
    commands.syncActions(originSpec, destSpec, opts, syncFlags, token).catch((e) => {
      if (token !== planToken) return;
      error = String(e);
      migrateBusy = false;
    });
  }

  // Migrate dry-run (M1.4): start the action-list computation; the `plan-ready` listener opens the plan
  // modal when it finishes. Writes nothing — applying the migration is M1.5.
  function previewMigrate() {
    if (!originSpec || !destSpec) return;
    error = "";
    migrateActions = null;
    migratePanel = false;
    const token = ++planToken;
    planProgress = 0;
    migrateBusy = true;
    const opts = makeOpts(migrateMethod, migrateExclude);
    commands.migrateActions(originSpec, destSpec, opts, migrateFlags, token).catch((e) => {
      if (token !== planToken) return;
      error = String(e);
      migrateBusy = false;
    });
  }

  // Flush the buffered apply outcomes onto the $state list in one batch (one re-render/frame).
  function flushOutcomes() {
    outcomeFlushScheduled = false;
    if (pendingOutcomes.length === 0) return;
    migrateOutcomes = migrateOutcomes.concat(pendingOutcomes);
    pendingOutcomes = [];
  }
  // Buffer a streamed apply outcome and schedule a per-frame flush (rAF; setTimeout fallback for tests /
  // backgrounded tabs). Mirrors `queueVerdict` — avoids the O(N²) of rebuilding the list per event.
  function queueOutcome(o: MigrateOutcome) {
    pendingOutcomes.push(o);
    if (outcomeFlushScheduled) return;
    outcomeFlushScheduled = true;
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(flushOutcomes);
    else setTimeout(flushOutcomes, 50);
  }

  // Apply the migration plan. `migrate_apply` returns immediately after spawning the background
  // thread — progress arrives via `migrate-progress` events, completion via `migrate-done`.
  async function startMigrateApply() {
    if (!originSpec || !destSpec || !migrateActions) return;
    const generation = migrateGeneration + 1;
    migrateGeneration = generation;
    // Unchecked items are shown immediately as skipped; only checked items are sent to the backend.
    const { toApply: checkedActions, skipped: skippedOutcomes } = splitChecked(migrateActions, migrateChecked);
    migrateOutcomes = skippedOutcomes;
    pendingOutcomes = []; // drop any buffered outcomes from a prior apply
    migrateDoneSummary = null;
    migrateMoveSummary = null;
    migratePhase = "";
    migrateApplying = true;
    migratePlanOpen = false;
    migrateProgressOpen = true;
    if (checkedActions.length === 0) {
      migrateApplying = false;
      migrateDoneSummary = { total: 0, ok: 0, failed: 0, cancelled: false };
      return;
    }
    // MOVE is honoured only for a Migrate run with an intact plan: unchecking any item means the user
    // altered the plan into a manual migration, so we must NOT delete the origin (the backend also
    // enforces this). A Sync run never moves.
    const moveRequested =
      activeFlow === "migrate" &&
      migrateFlags.delete_origin &&
      uncheckedCount(migrateActions, migrateChecked) === 0;
    try {
      await commands.migrateApply({
        left: originSpec,
        right: destSpec,
        actions: checkedActions,
        generation,
        deleteOrigin: moveRequested,
        opts: makeOpts(migrateMethod, migrateExclude),
      });
      // Returns immediately; migrateDoneSummary / migrateApplying are set by the migrate-done listener.
    } catch (e) {
      // The apply never started (bad args / connection error) — surface the real error instead of
      // faking a "cancelled" summary, and don't leave a misleading progress window open.
      error = `Migration failed to start: ${e}`;
      migrateApplying = false;
      migrateProgressOpen = false;
    }
  }

  function cancelMigrate() {
    commands.migrateCancel();
    // The running apply will detect the cancelled generation and return soon.
  }

  // After migration (complete or cancelled): close the progress modal and run a fresh comparison so
  // the user can verify the result. The compare uses whatever method is currently configured.
  async function continueMigrate() {
    migrateProgressOpen = false;
    migratePanel = false;
    await run();
  }

  // Open a differing file (clicked in the folder tree) in the side-by-side view.
  async function openFile(entry: DiffEntry) {
    if (!originSpec || !destSpec) return;
    error = "";
    const relPath = entry.rel_path.join("/");
    const leftRef: FileRef = { source: originSpec, rel: relPath };
    const rightRef: FileRef = { source: destSpec, rel: relPath };
    const base = {
      name: relPath, left: leftRef, right: rightRef,
      leftDate: entry.left?.mtime ?? null, rightDate: entry.right?.mtime ?? null,
      leftCreated: entry.left?.created ?? null, rightCreated: entry.right?.created ?? null,
    };
    openedFile = { loading: true, ...base };
    try {
      const r = await commands.diffFile(leftRef, rightRef);
      if (!openedFile) return;
      if (r.result.kind === "too_large") {
        // Backend signals the file is over the cap — show the warning dialog (no error, sizes included).
        openedFile = null;
        largeFileConfirm = { name: relPath, left: leftRef, right: rightRef, leftSize: r.result.left_size, rightSize: r.result.right_size };
      } else {
        openedFile = { loading: false, result: r.result, leftMeta: r.left, rightMeta: r.right, ...base };
      }
    } catch (e) {
      error = String(e);
      openedFile = null;
    }
  }

  // Called when the user clicks "Continue" in the large-file warning dialog. One call to diff_file_large;
  // it returns either a text-hunks view or a binary verdict — discriminated by `kind`.
  async function openLargeFile() {
    if (!largeFileConfirm) return;
    const { name, left, right } = largeFileConfirm;
    const base = { name, left, right, leftDate: null, rightDate: null, leftCreated: null, rightCreated: null };
    largeFileConfirm = null;
    openedFile = { loading: true, ...base };
    try {
      const r = await commands.diffFileLarge(left, right);
      if (!openedFile) return;
      const dates = { leftDate: r.left.mtime, rightDate: r.right.mtime, leftCreated: r.left.created, rightCreated: r.right.created };
      if (r.kind === "text_hunks") {
        openedFile = { loading: false, hunks: r.hunks, leftMeta: r.left, rightMeta: r.right, ...base, ...dates };
      } else {
        openedFile = { loading: false, result: { kind: "binary", identical: r.identical }, leftMeta: r.left, rightMeta: r.right, ...base, ...dates };
      }
    } catch (e) {
      error = String(e);
      openedFile = null;
    }
  }

  // "Load more" in the large-file hunks view: fetch the next page and append to the loaded hunks.
  async function loadMoreHunks() {
    if (!openedFile || !("hunks" in openedFile)) return;
    const startHunk = openedFile.hunks.next_hunk_index ?? 0;
    const r = await commands.diffFileLarge(openedFile.left, openedFile.right, { startHunk });
    if (!openedFile || !("hunks" in openedFile) || r.kind !== "text_hunks") return;
    openedFile = {
      ...openedFile,
      hunks: { ...r.hunks, hunks: [...openedFile.hunks.hunks, ...r.hunks.hunks] },
    };
  }

  // Open a "different" plan action in the side-by-side for inspection. Closes the plan modal and
  // remembers to reopen it when the user closes the diff view.
  async function openFileFromPlan(action: MigrateAction) {
    if (!originSpec || !destSpec) return;
    error = "";
    const relPath = action.rel_path.join("/");
    const leftRef: FileRef = { source: originSpec, rel: relPath };
    const rightRef: FileRef = { source: destSpec, rel: relPath };
    migratePlanOpen = false;
    syncPlanOpen = false;
    afterFileBack = "plan";
    planViewingAction = action;
    openedFile = { loading: true, name: relPath, left: leftRef, right: rightRef, leftDate: null, rightDate: null, leftCreated: null, rightCreated: null };
    try {
      const r = await commands.diffFile(leftRef, rightRef);
      if (!openedFile) return;
      if (r.result.kind === "too_large") {
        // Over the cap → route through the large-file flow (warning → hunks/hex), same as the tree path.
        // afterFileBack/planViewingAction stay set, so OK/Skip still work once the hunks view is shown.
        openedFile = null;
        largeFileConfirm = { name: relPath, left: leftRef, right: rightRef, leftSize: r.result.left_size, rightSize: r.result.right_size };
      } else {
        openedFile = {
          loading: false, name: relPath, left: leftRef, right: rightRef,
          leftDate: r.left.mtime, rightDate: r.right.mtime,
          leftCreated: r.left.created, rightCreated: r.right.created,
          result: r.result, leftMeta: r.left, rightMeta: r.right,
        };
      }
    } catch (e) {
      error = String(e);
      openedFile = null;
    }
  }

  function closeOpenedFile() {
    openedFile = null; // if loading=true, the guard `if (!openedFile) return` in the async fns handles it
    planViewingAction = null;
    reopenPlanIfFromPlan();
  }

  // If the file view was reached from a plan, return to it; otherwise no-op (back to the tree).
  function reopenPlanIfFromPlan() {
    if (afterFileBack === "plan") {
      afterFileBack = null;
      if (activeFlow === "sync") syncPlanOpen = true;
      else migratePlanOpen = true;
    }
  }

  // Cancel the large-file warning → return to wherever we came from (plan or tree).
  function cancelLargeFile() {
    largeFileConfirm = null;
    planViewingAction = null;
    reopenPlanIfFromPlan();
  }

  function keepFromPlan() {
    if (planViewingAction) migrateChecked.add(planViewingAction.rel_path.join("/"));
    closeOpenedFile();
  }
  function skipFromPlan() {
    if (planViewingAction) migrateChecked.delete(planViewingAction.rel_path.join("/"));
    closeOpenedFile();
  }

  // A file was saved from the side-by-side → the folder report is now stale; re-compare so the tree
  // reflects the new on-disk state (a file just made equal stops showing as "different").
  // Preserves the user's tree position (expansion, cursor, scroll, focus path) across the re-scan.
  async function onFileSaved(what: "left" | "right" | "both") {
    const savedExpanded = [...expanded];
    const savedCursor = cursor;
    const savedScrollTop = scrollTop;
    const savedFocusPath = [...focusPath];

    await runCompare();

    for (const k of savedExpanded) expanded.add(k);

    if (report) {
      const reloadKeys = [...new Set([...savedExpanded, savedFocusPath.join("/")])];
      reloadKeys.sort((a, b) => a.split("/").length - b.split("/").length);
      for (const key of reloadKeys) {
        if (!key) continue;
        const entry = entryAtPath(report.root, key.split("/"));
        if (entry && isPending(entry)) await onExpand(entry);
      }
    }

    cursor = savedCursor;
    focusPath = savedFocusPath;

    await tick();
    if (treeEl) treeEl.scrollTop = savedScrollTop;

    const label = what === "both" ? "both files" : `the ${what} file`;
    flashMsg(`Saved ${label} and refreshed the comparison.`);
  }

  // Esc: close the confirmation modal, then a tree-opened file, then leave files mode.
  // Otherwise (in the folder view, not over a form field or modal) drive tree keyboard navigation.
  function onWindowKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (picking) picking = null;
      else if (plan) cancelAction();
      else if (cancelConfirm) cancelConfirm = false;
      else if (openedFile) {
        if (sbsExitHandler) sbsExitHandler();
        else closeOpenedFile();
      }
      else if (focusPath.length) leaveFolder(); // pop one folder level before leaving the comparison
      else if (comparing) cancelConfirm = true; // ask before discarding the comparison
      else quitApp(); // Esc on the entry screen quits the app
      return;
    }
    // Tree navigation only applies to the folder report, never while a modal/file view is open or
    // when typing in a form field (method/exclude/date-tolerance inputs).
    if (!report || openedFile || picking || plan || cancelConfirm) return;
    const tag = (e.target as HTMLElement | null)?.tagName;
    if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
    handleTreeKey(e);
  }

  // Leaving the comparison back to source selection discards the current comparison (confirm first).
  function tryLeaveToSources() {
    cancelConfirm = true;
  }
  function leaveToSources() {
    cancelConfirm = false;
    report = null;
    diffResult = null;
    openedFile = null;
    focusPath = [];
  }
  async function quitApp() {
    try {
      await getCurrentWindow().close();
    } catch {
      /* not running inside Tauri (e.g. a browser preview) */
    }
  }
  // Comparison view: changing a compare parameter re-runs the comparison automatically (autorefresh).
  function refresh() {
    if (report) runCompare();
  }
  // Toggling Lazy is only a load-strategy switch — it must NOT re-run (and so discard) the comparison
  // already computed. Turning it OFF preloads the dirs not yet loaded (onExpand skips loaded ones, so
  // this just fills the gaps); turning it ON simply stops auto-preloading — already-resolved entries stay.
  function onLazyToggle() {
    if (!lazy && report) preloadAll(report.root.children);
  }

  // `savedContentVerdicts`: content verdicts saved before a metadata-method recompute overwrites them.
  // Keyed by rel_path. Lets us restore byte-level results when the user returns to full/quick without
  // a backend round-trip. Cleared when runCompare runs (fresh scan makes them stale).
  type ContentVerdict = { status: DiffStatus; detail: string | null };
  const savedContentVerdicts = new Map<string, ContentVerdict>();

  // True while the tree shows metadata verdicts (some entries have their content verdicts saved away).
  const inMetadataMode = $derived(savedContentVerdicts.size > 0);

  // Walk the tree and re-stamp both-sides files with the current metadata method's verdict.
  // Only saves the existing verdicts if they came from a CONTENT method (full/quick): saving metadata
  // verdicts as "content verdicts" would give wrong results on restore (e.g., size says "identical"
  // for same-size different-content files — restoring that as a full verdict would be incorrect).
  function recomputeMetadataVerdicts(root: DiffEntry) {
    ++scanToken; // stop any in-flight content resolver
    verdictBuffer.clear();
    pendingVerdicts.clear();
    const saveContent = !isMetadataMethod(lastComparedMethod); // only save full/quick verdicts
    const walk = (entries: DiffEntry[]) => {
      for (const e of entries) {
        if (e.is_dir) {
          walk(e.children);
        } else if (e.left && e.right) {
          if (e.detail === "filtered" || e.detail === "symlink (not followed)") continue;
          const key = e.rel_path.join("/");
          // Save the first time only (so size→mtime→size-mtime keeps the original content verdict,
          // not an intermediate metadata verdict). Skip if we don't have content verdicts to save.
          if (saveContent && !savedContentVerdicts.has(key)) {
            savedContentVerdicts.set(key, { status: e.status, detail: e.detail });
          }
          const v = metadataVerdict(e.left, e.right, method);
          if (v) { e.status = v.status; e.detail = v.detail; }
        }
      }
    };
    walk(root.children);
  }

  // Restore the content verdicts saved by recomputeMetadataVerdicts and clear the map.
  function restoreContentVerdicts(root: DiffEntry) {
    const walk = (entries: DiffEntry[]) => {
      for (const e of entries) {
        if (e.is_dir) walk(e.children);
        else {
          const saved = savedContentVerdicts.get(e.rel_path.join("/"));
          if (saved) { e.status = saved.status; e.detail = saved.detail; }
        }
      }
    };
    walk(root.children);
    savedContentVerdicts.clear();
  }

  // Method change: decide whether to recompute in client (metadata case, or restoring saved content
  // verdicts) or re-fetch from the backend (the only way to get byte-level verdicts we don't have).
  function onMethodChange() {
    if (!report) return;
    if (isMetadataMethod(method)) {
      // → metadata: save content verdicts (if any) and recompute from size/mtime.
      recomputeMetadataVerdicts(report.root);
    } else if (inMetadataMode) {
      // Leaving metadata mode. The saved verdicts were produced by `lastComparedMethod`.
      // We can restore them only when they're valid for the target method:
      //   - saved are content verdicts (lastComparedMethod is full or quick, not a metadata method)
      //   - AND saved strength ≥ target (full covers any content method; quick only covers quick)
      const canRestore =
        !isMetadataMethod(lastComparedMethod) &&
        (lastComparedMethod === "full" || method === "quick");
      if (canRestore) {
        restoreContentVerdicts(report.root);
      } else {
        // Saved verdicts aren't strong enough (e.g. quick-saved → want full, or size-saved → want any).
        savedContentVerdicts.clear();
        refresh();
      }
    } else if (method === "full" && lastComparedMethod !== "full") {
      // Already in content mode, quick→full: lossy quick verdicts aren't good enough.
      refresh();
    }
    // Otherwise: no-op — verdicts already valid (full→quick, quick→quick, full→full).
  }

  // `savedVerdicts`: original {status, detail} for entries we filtered client-side (after a compare),
  // keyed by rel_path joined. Lets us restore verdicts when a client-added exclude is removed.
  // Entries that are filtered but NOT in this map were excluded by the backend — we have no verdict.
  type SavedVerdict = { status: DiffStatus; detail: string | null };
  const savedVerdicts = new Map<string, SavedVerdict>();

  // Track the parsed exclude list between changes (needed to compute added/removed sets).
  let prevExcludePatterns = $state<string[]>([]);
  // Method that was active when the last runCompare completed — used to detect safe downgrades.
  let lastComparedMethod: CompareOpts["method"] = "quick";

  // Mark entries matching `patterns` as filtered. Saves the original verdict so we can restore
  // it if the client-added exclude is later removed.
  function recomputeExcludeFilter(root: DiffEntry, patterns: string[]) {
    if (patterns.length === 0) return;
    const walk = (entries: DiffEntry[]) => {
      for (const e of entries) {
        if (patterns.some((p) => matchesGlob(e.rel_path, p))) {
          const key = e.rel_path.join("/");
          if (!savedVerdicts.has(key)) savedVerdicts.set(key, { status: e.status, detail: e.detail });
          e.status = "skipped";
          e.detail = "filtered";
        }
        if (e.is_dir && e.children.length > 0) walk(e.children);
      }
    };
    walk(root.children);
  }

  // Restore entries that are in `savedVerdicts` but no longer match any `activePatterns`.
  // Called when a client-added exclude is removed.
  function recomputeUnfilter(root: DiffEntry, activePatterns: string[]) {
    const walk = (entries: DiffEntry[]) => {
      for (const e of entries) {
        const key = e.rel_path.join("/");
        if (savedVerdicts.has(key) && !activePatterns.some((p) => matchesGlob(e.rel_path, p))) {
          const saved = savedVerdicts.get(key)!;
          e.status = saved.status;
          e.detail = saved.detail;
          savedVerdicts.delete(key);
        }
        if (e.is_dir && e.children.length > 0) walk(e.children);
      }
    };
    walk(root.children);
  }

  function onExcludeChange() {
    const next = parseExclude(exclude);
    if (!report) { prevExcludePatterns = next; return; }

    const added = next.filter((p) => !prevExcludePatterns.includes(p));
    const removed = prevExcludePatterns.filter((p) => !next.includes(p));

    if (removed.length > 0 && excludeRemovalNeedsRefetch(report.root, removed, next, savedVerdicts)) {
      // Some removed patterns covered entries the backend never compared (not in savedVerdicts) — must
      // re-fetch to get their verdicts. If all affected entries have saved verdicts, falls through to
      // the client-only path below.
      prevExcludePatterns = next;
      runCompare();
      return;
    }
    if (added.length > 0) recomputeExcludeFilter(report.root, added);
    if (removed.length > 0) recomputeUnfilter(report.root, next);
    prevExcludePatterns = next;
  }

  // Double-click dispatch: a file opens the side-by-side; a folder drills in (breadcrumb view).
  // The ▶ arrow handles inline expand/collapse separately. Before drilling in we make sure the
  // folder's level is loaded, so the focused view isn't empty in lazy/non-recursive mode (onExpand
  // is a no-op if it's already loaded or loading; reactivity fills the view in when it arrives).
  async function onOpen(entry: DiffEntry) {
    if (entry.is_dir) {
      if (isPending(entry)) await onExpand(entry);
      enterFolder(entry);
    } else {
      openFile(entry);
    }
  }
  function enterFolder(entry: DiffEntry) {
    focusPath = [...entry.rel_path];
    openedFile = null;
    scrollTop = 0;
  }
  function leaveFolder() {
    if (focusPath.length) {
      focusPath = focusPath.slice(0, -1);
      scrollTop = 0;
    }
  }

  async function startAction(op: SyncOp, label: string) {
    if (!originSpec || !destSpec) return;
    const actions: SyncAction[] = [...selection.values()].map((e) => ({
      rel_path: e.rel_path,
      op,
      is_dir: e.is_dir,
    }));
    if (actions.length === 0) return;
    pendingLabel = label;
    pendingActions = actions;
    try {
      plan = await commands.planActions(originSpec, destSpec, actions);
    } catch (e) {
      error = String(e);
      pendingActions = null;
    }
  }

  function cancelAction() {
    plan = null;
    pendingActions = null;
  }

  async function confirmApply() {
    if (!pendingActions || !originSpec || !destSpec) return;
    applying = true;
    try {
      const outcomes = await commands.applyActions(originSpec, destSpec, pendingActions);
      const failed = outcomes.filter((o) => !o.ok).length;
      resultMsg = `${pendingLabel}: ${outcomes.length - failed} ok, ${failed} failed.`;
    } catch (e) {
      error = String(e);
    } finally {
      applying = false;
      plan = null;
      pendingActions = null;
      await runCompare(); // refresh to reflect the new state
    }
  }

  const k = (a: string[]) => a.join("/") || ".";

  // ---- Lazy comparison helpers ----
  function isPending(e: DiffEntry): boolean {
    return e.status === "skipped" && e.detail === "not descended";
  }

  // Recursively compute the resolved status of a pending dir from its loaded subtree.
  // Returns null if the dir hasn't been loaded, any descendant dir isn't fully resolved, or any file
  // is still being compared — the parent can't be decided until its whole subtree is known.
  function resolveDir(e: DiffEntry): DiffStatus | null {
    if (!loaded.has(e.rel_path.join("/"))) return null;
    let hasDiff = false;
    for (const child of e.children) {
      if (child.is_dir && isPending(child)) {
        const s = resolveDir(child);
        if (s === null) return null;
        if (s !== "identical") hasDiff = true;
      } else if (isComparing(child)) {
        return null; // a file verdict is still streaming in — not resolved yet
      } else if (
        child.status === "different" ||
        child.status === "left_only" ||
        child.status === "right_only" ||
        child.status === "error"
      ) {
        hasDiff = true;
      }
    }
    return hasDiff ? "different" : "identical";
  }

  async function onExpand(entry: DiffEntry) {
    if (!originSpec || !destSpec) return;
    const key = entry.rel_path.join("/");
    if (loaded.has(key) || pendingLoads.has(key)) return;
    pendingLoads.add(key);
    const token = scanToken; // part of the current comparison — do NOT bump
    try {
      const opts = makeOpts(method, exclude);
      const result = await commands.compareLevel(originSpec, destSpec, opts, key, token);
      if (token !== scanToken) return; // a new comparison started while we were loading — drop it
      // list_level returns children already rooted at the source root, so they splice straight into
      // the tree (no re-basing). Mutating a $state node's `children` is reactive.
      entry.children = result.root.children;
      loaded.add(key);
      drainVerdicts(); // apply any of this level's verdicts that arrived before the listing
    } catch (e) {
      error = String(e);
    } finally {
      pendingLoads.delete(key);
    }
  }

  // Preload (non-lazy) mode: after the initial compare, auto-expand every pending dir in the
  // background. A bounded worker pool keeps the connection busy without a request storm (which,
  // over a slow link, froze the UI). Newly discovered pending subdirs are enqueued as levels load.
  const PRELOAD_CONCURRENCY = 4;
  async function preloadAll(top: DiffEntry[]) {
    const queue: DiffEntry[] = top.filter((e) => e.is_dir && isPending(e));
    let idx = 0;
    const worker = async () => {
      while (idx < queue.length) {
        const e = queue[idx++];
        await onExpand(e);
        for (const child of e.children) {
          if (child.is_dir && isPending(child)) queue.push(child);
        }
      }
    };
    preloading = true;
    try {
      await Promise.all(Array.from({ length: PRELOAD_CONCURRENCY }, worker));
    } finally {
      preloading = false;
    }
  }

  // ---- Flattened, virtualized view (fixed-height rows; no external dep) ----
  const ROW_H = 24;
  const OVERSCAN = 8;
  let scrollTop = $state(0);
  let viewportH = $state(480);
  let treeEl = $state<HTMLElement | undefined>(); // the scroll container (for keyboard scroll-into-view)
  // Keyboard-navigation cursor: index into the visible `rows`. -1 = no cursor yet.
  let cursor = $state(-1);
  // Folder drill-down: rel-path components of the focused subfolder ([] = comparison root). Double-clicking
  // a folder present on both sides enters it; Esc / "Up" pops one level back, up to the root.
  let focusPath = $state<string[]>([]);

  type Row = { entry: DiffEntry; depth: number; loading: boolean; resolvedStatus: DiffStatus | null };
  type StatusOf = (e: DiffEntry) => DiffStatus;
  function flattenInto(entries: DiffEntry[], depth: number, out: Row[], statusOf: StatusOf) {
    for (const e of sortEntries(entries, statusOf)) {
      if (!nodeVisible(e, statusOf)) continue;
      const key = e.rel_path.join("/");
      const pend = isPending(e);
      // Recursively resolve the dir's status — null until the full subtree is loaded.
      const resolvedStatus: DiffStatus | null = pend ? resolveDir(e) : null;
      // Spin while a dir's level is loading or a file's verdict is still streaming in.
      // Spin while: a dir's level is being fetched, a file's verdict is streaming, OR a loaded dir's
      // subtree is still resolving (children comparing) so the folder shows progress, not a static box.
      const loading =
        pendingLoads.has(key) || isComparing(e) || (pend && loaded.has(key) && resolvedStatus === null);
      out.push({ entry: e, depth, loading, resolvedStatus });
      if (e.is_dir && isOpen(key) && e.children.length > 0) {
        flattenInto(e.children, depth + 1, out, statusOf);
      }
    }
  }
  // The DiffEntry at `focusPath` (the report root when not drilled in) — drives which subtree the tree shows.
  function entryAtPath(root: DiffEntry, path: string[]): DiffEntry | null {
    let cur = root;
    for (const name of path) {
      const next = cur.children.find((c) => c.is_dir && c.name === name);
      if (!next) return null;
      cur = next;
    }
    return cur;
  }
  const focused = $derived(report ? entryAtPath(report.root, focusPath) : null);
  const rows = $derived.by<Row[]>(() => {
    const out: Row[] = [];
    // Effective status for filtering: resolved status for a loaded pending dir, raw otherwise.
    // An unresolved pending dir keeps its raw `skipped` (stays visible until its subtree is known).
    // Memoised per pass — the filter recurses into descendants, so without a cache this is O(n²).
    const cache = new Map<string, DiffStatus>();
    const statusOf: StatusOf = (e) => {
      const key = e.rel_path.join("/");
      const hit = cache.get(key);
      if (hit !== undefined) return hit;
      const s = isPending(e) ? (resolveDir(e) ?? e.status) : e.status;
      cache.set(key, s);
      return s;
    };
    if (focused) flattenInto(focused.children, 0, out, statusOf);
    return out;
  });
  const start = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const end = $derived(Math.min(rows.length, start + Math.ceil(viewportH / ROW_H) + OVERSCAN * 2));
  const visible = $derived(rows.slice(start, end).map((r, i) => ({ row: r, index: start + i })));

  // Live counters: tally the loaded tree (files + one-sided dirs) by status, recomputed as levels
  // load and streamed verdicts land. `scanning` = files still being compared in the background.
  type LiveSummary = {
    identical: number; different: number; left_only: number; right_only: number;
    skipped: number; errored: number; scanning: number;
  };
  const liveSummary = $derived.by<LiveSummary>(() => {
    const s: LiveSummary = { identical: 0, different: 0, left_only: 0, right_only: 0, skipped: 0, errored: 0, scanning: 0 };
    if (!report) return s;
    const walk = (e: DiffEntry) => {
      for (const c of e.children) {
        if (c.is_dir) {
          if (c.status === "left_only") s.left_only++;
          else if (c.status === "right_only") s.right_only++;
          else walk(c); // both-sides dir: a container — recurse into its loaded children
        } else if (isComparing(c)) {
          s.scanning++;
        } else if (c.status === "identical") s.identical++;
        else if (c.status === "different") s.different++;
        else if (c.status === "left_only") s.left_only++;
        else if (c.status === "right_only") s.right_only++;
        else if (c.status === "error") s.errored++;
        else s.skipped++; // filtered / symlink
      }
    };
    walk(report.root);
    return s;
  });

  // Keep the cursor in range as the visible list grows/shrinks (filtering, expand/collapse, new compare).
  $effect(() => {
    if (rows.length === 0) cursor = -1;
    else if (cursor < 0) cursor = 0;
    else if (cursor >= rows.length) cursor = rows.length - 1;
  });

  // Restore tree scroll when returning from the side-by-side (the tree DOM is rebuilt, losing scrollTop).
  let wasFileOpen = $state(false);
  $effect(() => {
    const fileOpen = !!openedFile;
    if (wasFileOpen && !fileOpen && treeEl) {
      treeEl.scrollTop = scrollTop;
    }
    wasFileOpen = fileOpen;
  });

  // Scroll the cursor row into view within the virtualized container (DOM scrollTop drives `scrollTop`).
  function scrollCursorIntoView() {
    if (!treeEl || cursor < 0) return;
    const top = cursor * ROW_H;
    const bottom = top + ROW_H;
    if (top < treeEl.scrollTop) treeEl.scrollTop = top;
    else if (bottom > treeEl.scrollTop + treeEl.clientHeight) {
      treeEl.scrollTop = bottom - treeEl.clientHeight;
    }
  }
  function moveCursor(delta: number) {
    if (rows.length === 0) return;
    cursor = Math.max(0, Math.min(rows.length - 1, (cursor < 0 ? 0 : cursor) + delta));
    scrollCursorIntoView();
  }

  // Arrow/Enter/Space navigation over the visible tree. See the per-key behaviour inline.
  function handleTreeKey(e: KeyboardEvent) {
    if (rows.length === 0) return;
    if (cursor < 0 || cursor >= rows.length) cursor = 0;
    const entry = rows[cursor].entry;
    const key = entry.rel_path.join("/");
    const expandable = entry.is_dir && (entry.children.length > 0 || isPending(entry));
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        moveCursor(1);
        break;
      case "ArrowUp":
        e.preventDefault();
        moveCursor(-1);
        break;
      case "PageDown":
        e.preventDefault();
        moveCursor(Math.max(1, Math.floor(viewportH / ROW_H) - 1));
        break;
      case "PageUp":
        e.preventDefault();
        moveCursor(-Math.max(1, Math.floor(viewportH / ROW_H) - 1));
        break;
      case "Home":
        e.preventDefault();
        cursor = 0;
        scrollCursorIntoView();
        break;
      case "End":
        e.preventDefault();
        cursor = rows.length - 1;
        scrollCursorIntoView();
        break;
      case "ArrowRight":
        // Collapsed folder → expand (loading it if lazy); already-open folder or file → next row.
        e.preventDefault();
        if (expandable && !isOpen(key)) {
          toggleOpen(key);
          if (isPending(entry)) onExpand(entry);
        } else {
          moveCursor(1);
        }
        break;
      case "ArrowLeft":
        // Open folder → collapse; collapsed folder or file → jump to parent (VS Code pattern);
        // at root level (no parent) → previous row.
        e.preventDefault();
        if (entry.is_dir && isOpen(key)) {
          toggleOpen(key);
        } else {
          const parentDepth = rows[cursor].depth - 1;
          if (parentDepth >= 0) {
            for (let i = cursor - 1; i >= 0; i--) {
              if (rows[i].depth === parentDepth) {
                cursor = i;
                scrollCursorIntoView();
                break;
              }
            }
          } else {
            moveCursor(-1);
          }
        }
        break;
      case "Enter":
        // Folder → drill into it; file → open the side-by-side.
        e.preventDefault();
        onOpen(entry);
        break;
      case " ":
        // Mark/unmark the cursor row (same as its checkbox).
        e.preventDefault();
        toggle(entry);
        break;
    }
  }

  // (Progress per-category counts + auto-scroll now live inside <MigrateProgressModal>.)

  // (The plan virtualizer + per-category counts now live inside <MigratePlanModal>.)
  // Still needed here: the checked-item total feeds the progress modal's "≈N" reference.
  const planCheckedTotal = $derived(checkedItemTotal(migrateActions ?? [], migrateChecked));

  let shellInstalled = $state(false);
  let shellBusy = $state(false);
  async function toggleShellIntegration() {
    if (shellBusy) return;
    shellBusy = true;
    try {
      if (shellInstalled) {
        await commands.uninstallShellIntegration();
        shellInstalled = false;
        flashShell("Removed.");
      } else {
        await commands.installShellIntegration();
        shellInstalled = true;
        flashShell("Installed. Right-click a folder → Services.");
      }
    } catch (e) {
      error = String(e);
    } finally {
      shellBusy = false;
    }
  }
  onMount(() => {
    commands.shellIntegrationStatus().then((s) => (shellInstalled = s.installed)).catch(() => {});
  });
</script>

<svelte:window onkeydown={onWindowKey} onclick={() => (modeMenuOpen = false)} />

<!-- Shared comparison controls — rendered identically in the entry view and the comparison compact bar.
     Order: recursive → method → exclude. `refresh` is a no-op until a comparison exists (entry view). -->
{#snippet compareControls()}
  <label class="ctl"><input type="checkbox" bind:checked={lazy} onchange={onLazyToggle} /> Lazy</label>
  <label class="ctl">Method
    <select bind:value={method} onchange={onMethodChange}>
      <option value="quick">quick</option>
      <option value="full">full</option>
      <option value="size">size</option>
      <option value="mtime">mtime</option>
      <option value="size-mtime">size-mtime</option>
    </select>
  </label>
  <label class="ctl">Exclude
    <input class="ex" bind:value={exclude} onchange={onExcludeChange} placeholder="*.tmp, node_modules" spellcheck="false" />
  </label>
  <label class="ctl">Date tol. <input class="tol" type="number" bind:value={dateTol} min="0" step="500" /> ms</label>
{/snippet}

<main>
  {#if !comparing && !openedFile}
  <div class="entry">
    <header>
      <img class="brand-logo" src="/confold-mark.png" alt="" />
      <div class="titlewrap">
        <h1>Confold</h1>
        {#if appVersion}<span class="appver">{appVersion}</span>{/if}
      </div>
    </header>
    <div class="entry-mid">
  <section class="sources">
    <div class="side">
      {#if originSpec}
        <button class="chosen" class:stale={originStale} type="button" onclick={() => { recentPrefill = originSpec; picking = "origin"; }}>
          <span class="ico">{slugOf(originSpec).icon}</span>
          <span class="slug">{slugOf(originSpec).label}</span>
          <span class="change">change</span>
        </button>
      {:else}
        <button class="add" type="button" onclick={() => (picking = "origin")}>
          <span class="plus">+</span> Select source
        </button>
      {/if}
      {#if originRecents.some((r) => !originSpec || slugOf(r.spec).label !== slugOf(originSpec).label)}
        <div class="recents">
          <span class="rlabel">Recent sources:</span>
          {#each originRecents.filter((r) => !originSpec || slugOf(r.spec).label !== slugOf(originSpec).label) as r (slugOf(r.spec).label)}
            <button class="recent" class:stale={r.stale} type="button" onclick={() => pickRecent(r, "origin")} title={slugOf(r.spec).label}>
              <span class="ico">{slugOf(r.spec).icon}</span><span class="rslug">{slugOf(r.spec).label}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="side">
      {#if destSpec}
        <button class="chosen" class:stale={destStale} type="button" onclick={() => { recentPrefill = destSpec; picking = "dest"; }}>
          <span class="ico">{slugOf(destSpec).icon}</span>
          <span class="slug">{slugOf(destSpec).label}</span>
          <span class="change">change</span>
        </button>
      {:else}
        <button class="add" type="button" onclick={() => (picking = "dest")}>
          <span class="plus">+</span> Select destination
        </button>
      {/if}
      {#if destRecents.some((r) => !destSpec || slugOf(r.spec).label !== slugOf(destSpec).label)}
        <div class="recents">
          <span class="rlabel">Recent destinations:</span>
          {#each destRecents.filter((r) => !destSpec || slugOf(r.spec).label !== slugOf(destSpec).label) as r (slugOf(r.spec).label)}
            <button class="recent" class:stale={r.stale} type="button" onclick={() => pickRecent(r, "dest")} title={slugOf(r.spec).label}>
              <span class="ico">{slugOf(r.spec).icon}</span><span class="rslug">{slugOf(r.spec).label}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </section>
    </div>
    <div class="entry-bottom">
  <div class="opts">
    {#if !bothFiles}
      {@render compareControls()}
    {/if}
  </div>

  {#if invalidReason}
    <p class="warn">{invalidReason}</p>
  {/if}

  <div class="action-bar">
    <!-- Split button: left side runs the selected mode; right side opens the mode picker. -->
    <div class="split-btn" class:disabled={!canRun}>
      <button
        class="split-main"
        onclick={primaryAction}
        disabled={!canRun}
      >
        {loading ? "Working…" : modeLabel[mode]}
      </button>
      <div class="split-sep"></div>
      <button
        class="split-arrow"
        disabled={loading}
        onclick={(e) => { e.stopPropagation(); modeMenuOpen = !modeMenuOpen; }}
        aria-label="Select mode"
        aria-expanded={modeMenuOpen}
      >▾</button>
      {#if modeMenuOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="mode-menu" onmouseleave={() => (modeMenuOpen = false)}>
          {#each (["compare", "sync", "migrate"] as AppMode[]) as m (m)}
            <button
              class="mode-item"
              class:active={mode === m}
              onclick={() => { mode = m; modeMenuOpen = false; }}
            >
              <span class="mode-name">{modeLabel[m]}</span>
              <span class="mode-desc">{modeDesc[m]}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
  <div class="entry-shell-toggle">
    <label class="ctl">
      <input type="checkbox" checked={shellInstalled} disabled={shellBusy} onchange={toggleShellIntegration} />
      Folder right-click integration
    </label>
    {#if shellMsg}<span class="shell-msg">{shellMsg}</span>{/if}
  </div>
  </div>
  </div>
  {:else}
  {#if !openedFile}
  <div class="compact-bar">
    <img class="compact-logo" src="/confold-mark.png" alt="Confold" />
    <button class="mini-src" type="button" onclick={tryLeaveToSources} title="Back to source selection">
      {#if originSpec}<span class="ico">{slugOf(originSpec).icon}</span><span class="mini-slug">{slugOf(originSpec).label}</span>{/if}
    </button>
    <span class="arrow">→</span>
    <button class="mini-src" type="button" onclick={tryLeaveToSources} title="Back to source selection">
      {#if destSpec}<span class="ico">{slugOf(destSpec).icon}</span><span class="mini-slug">{slugOf(destSpec).label}</span>{/if}
    </button>
    {#if report}
      <button class="mini-refresh" type="button" onclick={refresh} title="Re-scan comparison">↺</button>
      <span class="mini-opts">{@render compareControls()}</span>
      {#if preloading}<span class="mini-preloading"><span class="spinner"></span> loading folders…</span>{/if}
    {/if}
  </div>
  {/if}
  {/if}

  {#if error}<p class="error">{error}</p>{/if}
  {#if resultMsg}<p class="result">{resultMsg}</p>{/if}
  {#if loading && !report && !diffResult}<p class="loading">Comparing…</p>{/if}

  {#if openedFile}
    {#if openedFile.loading}
      <SideBySideSkeleton name={openedFile.name} onback={closeOpenedFile} />
    {:else if 'hunks' in openedFile}
      <!-- Large-file hunks-only mode: read-only, paginated. Status bar is ABOVE the diff. -->
      <div class="large-file-bar">
        {#if openedFile.hunks.is_complete}
          <span>{openedFile.hunks.total_hunks} hunk{openedFile.hunks.total_hunks === 1 ? "" : "s"} · full file</span>
        {:else}
          <span>Showing {openedFile.hunks.hunks.length} hunks</span>
          <button onclick={loadMoreHunks}>Load more</button>
        {/if}
        <span class="lf-notice">⚠ Read-only — to copy this file use Copy → / ← Copy in the file list</span>
      </div>
      {#key refKey(openedFile.left) + " " + refKey(openedFile.right) + "|h" + openedFile.hunks.hunks.length}
        <SideBySide
          initial={hunksToFileDiff(openedFile.hunks)}
          left={openedFile.left}
          right={openedFile.right}
          leftMeta={openedFile.leftMeta}
          rightMeta={openedFile.rightMeta}
          name={openedFile.name}
          leftDate={openedFile.leftDate}
          rightDate={openedFile.rightDate}
          leftCreated={openedFile.leftCreated}
          rightCreated={openedFile.rightCreated}
          readOnly={true}
          onkeep={afterFileBack === "plan" ? keepFromPlan : undefined}
          onskip={afterFileBack === "plan" ? skipFromPlan : undefined}
          onback={closeOpenedFile}
          onregisterexit={(fn) => (sbsExitHandler = fn)}
        />
      {/key}
    {:else if openedFile.result.kind === "text"}
      {#key refKey(openedFile.left) + " " + refKey(openedFile.right)}
        <SideBySide
          initial={openedFile.result.diff}
          left={openedFile.left}
          right={openedFile.right}
          leftMeta={openedFile.leftMeta}
          rightMeta={openedFile.rightMeta}
          name={openedFile.name}
          leftDate={openedFile.leftDate}
          rightDate={openedFile.rightDate}
          leftCreated={openedFile.leftCreated}
          rightCreated={openedFile.rightCreated}
          readOnly={afterFileBack === "plan"}
          onkeep={afterFileBack === "plan" ? keepFromPlan : undefined}
          onskip={afterFileBack === "plan" ? skipFromPlan : undefined}
          onsaved={onFileSaved}
          onback={closeOpenedFile}
          onregisterexit={(fn) => (sbsExitHandler = fn)}
        />
      {/key}
    {:else if openedFile.result.kind === "binary"}
      {@const planReview = afterFileBack === "plan"}
      {#key refKey(openedFile.left) + " " + refKey(openedFile.right)}
        {#if isImagePath(openedFile.left.rel) && isImagePath(openedFile.right.rel)}
          <ImageView
            left={openedFile.left} right={openedFile.right}
            name={openedFile.name} onback={closeOpenedFile}
            readOnly={planReview}
            onkeep={planReview ? keepFromPlan : undefined}
            onskip={planReview ? skipFromPlan : undefined}
          />
        {:else}
          <HexView
            left={openedFile.left} right={openedFile.right} identical={openedFile.result.identical}
            name={openedFile.name} onback={closeOpenedFile}
            readOnly={planReview}
            onkeep={planReview ? keepFromPlan : undefined}
            onskip={planReview ? skipFromPlan : undefined}
          />
        {/if}
      {/key}
    {/if}
  {/if}

  {#if report && !openedFile}
    {@const s = liveSummary}
    <!-- Live counts + colored-square legend + status filter, unified: click a pill to show/hide that
         status. Counts grow as levels load and streamed file verdicts land. -->
    <div class="summary">
      <button class="leg" class:off={hidden.has("identical")} type="button" onclick={() => toggleStatus("identical")}><span class="tick">{hidden.has("identical") ? "" : "✓"}</span><span class="sq eq"></span> <span class="num">{s.identical}</span> identical</button>
      <button class="leg" class:off={hidden.has("different")} type="button" onclick={() => toggleStatus("different")}><span class="tick">{hidden.has("different") ? "" : "✓"}</span><span class="sq diff"></span> <span class="num">{s.different}</span> different</button>
      <button class="leg" class:off={hidden.has("left_only")} type="button" onclick={() => toggleStatus("left_only")}><span class="tick">{hidden.has("left_only") ? "" : "✓"}</span><span class="sq src"></span> <span class="num">{s.left_only}</span> source only</button>
      <button class="leg" class:off={hidden.has("right_only")} type="button" onclick={() => toggleStatus("right_only")}><span class="tick">{hidden.has("right_only") ? "" : "✓"}</span><span class="sq dst"></span> <span class="num">{s.right_only}</span> destination only</button>
      <button class="leg" class:off={hidden.has("skipped")} type="button" onclick={() => toggleStatus("skipped")}><span class="tick">{hidden.has("skipped") ? "" : "✓"}</span><span class="dotleg">·</span> <span class="num">{s.skipped}</span> skipped</button>
      <button class="leg" class:off={hidden.has("error")} type="button" onclick={() => toggleStatus("error")}><span class="tick">{hidden.has("error") ? "" : "✓"}</span><span class="sq err"></span> <span class="num">{s.errored}</span> error</button>
      {#if s.scanning > 0}
        <span class="scanning"><span class="spinner"></span> <span class="num">{s.scanning}</span> comparing…</span>
      {/if}
    </div>
    {#if focusPath.length}
      <div class="crumbs">
        <button class="crumb" type="button" onclick={leaveFolder} title="Up one level">↑ Up</button>
        <button class="crumb" type="button" onclick={() => (focusPath = [])}>root</button>
        {#each focusPath as seg, i (i)}
          <span class="csep">/</span>
          <button class="crumb" type="button" onclick={() => (focusPath = focusPath.slice(0, i + 1))}>{seg}</button>
        {/each}
      </div>
    {/if}
    <div class="actions">
      <label class="sort">sort:
        <select bind:value={sortState.key}>
          <option value="name">name</option>
          <option value="status">status</option>
          <option value="size">size</option>
          <option value="mtime">mtime</option>
        </select>
      </label>
      <span class="seln">{selectedCount}/{rows.length} selected</span>
      <button disabled={!canCopyLR} onclick={() => startAction("copy_left_to_right", "Copy left → right")}>Copy →</button>
      <button disabled={!canCopyRL} onclick={() => startAction("copy_right_to_left", "Copy right → left")}>← Copy</button>
      <button class="danger" disabled={!canDelL} onclick={() => startAction("delete_left", "Delete on left")}>Delete left</button>
      <button class="danger" disabled={!canDelR} onclick={() => startAction("delete_right", "Delete on right")}>Delete right</button>
      <button class="ghost" disabled={selectedCount === 0} onclick={clearSelection}>Clear</button>
    </div>

    <div class="tree-wrap">
    <div class="thead">
      <span class="th-name">Name</span>
      <span class="th-sz">Source</span>
      <span class="th-sz">Dest.</span>
      <span class="th-dt">Source mod.</span>
      <span class="th-dt">Dest. mod.</span>
      <span class="th-dt">Source created</span>
      <span class="th-dt">Dest. created</span>
    </div>
    <div
      class="tree"
      bind:this={treeEl}
      onscroll={(e) => (scrollTop = (e.currentTarget as HTMLElement).scrollTop)}
      bind:clientHeight={viewportH}
    >
      {#if rows.length === 0}
        <p class="empty">Nothing to show (empty, fully filtered, or all hidden).</p>
      {:else}
        <div class="spacer" style="height: {rows.length * ROW_H}px">
          {#each visible as v (v.row.entry.rel_path.join("/"))}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="vrow" class:cursor={v.index === cursor} style="top: {v.index * ROW_H}px" onclick={() => (cursor = v.index)}>
              <DiffNode entry={v.row.entry} depth={v.row.depth} {dateTol} onopen={onOpen} loading={v.row.loading} resolvedStatus={v.row.resolvedStatus} onexpand={onExpand} />
            </div>
          {/each}
        </div>
      {/if}
    </div>
    </div>
  {/if}

  {#if diffResult && originSpec && destSpec}
    {#if diffResult.kind === "text" && filesMeta}
      {#key slugOf(originSpec).label + "|" + slugOf(destSpec).label}
        <SideBySide initial={diffResult.diff} left={fileRefOf(originSpec)} right={fileRefOf(destSpec)} leftMeta={filesMeta.left} rightMeta={filesMeta.right} name={fileRefOf(originSpec).rel} leftDate={filesMeta.left.mtime} rightDate={filesMeta.right.mtime} leftCreated={filesMeta.left.created} rightCreated={filesMeta.right.created} />
      {/key}
    {:else if diffResult.kind === "binary"}
      {#key slugOf(originSpec).label + "|" + slugOf(destSpec).label}
        {#if isImagePath(originSpec.fields.root ?? "") && isImagePath(destSpec.fields.root ?? "")}
          <ImageView left={fileRefOf(originSpec)} right={fileRefOf(destSpec)} />
        {:else}
          <HexView left={fileRefOf(originSpec)} right={fileRefOf(destSpec)} identical={diffResult.identical} />
        {/if}
      {/key}
    {/if}
  {/if}

  {#if picking}
    <SourcePicker
      title={picking === "origin" ? "Source" : "Destination"}
      types={sourceTypes}
      initial={recentPrefill}
      onconfirm={onPicked}
      oncancel={() => { picking = null; recentPrefill = null; }}
    />
  {/if}

  {#if cancelConfirm}
    <div
      class="overlay"
      role="presentation"
      onclick={(e) => {
        if (e.target === e.currentTarget) cancelConfirm = false;
      }}
    >
      <div class="modal">
        <h2>Back to source selection?</h2>
        <p>The current comparison will be discarded. Your selected sources are kept.</p>
        <div class="modal-actions">
          <button class="ghost" type="button" onclick={() => (cancelConfirm = false)}>Stay here</button>
          <button type="button" onclick={leaveToSources}>Back to sources</button>
        </div>
      </div>
    </div>
  {/if}

  {#if plan}
    <div class="overlay">
      <div class="modal">
        <h2>{pendingLabel}</h2>
        <p>{plan.length} operation{plan.length === 1 ? "" : "s"} will run:</p>
        <ul class="planlist">
          {#each plan.slice(0, 200) as o (k(o.rel_path) + o.op)}
            <li>{k(o.rel_path)}</li>
          {/each}
          {#if plan.length > 200}<li>… and {plan.length - 200} more</li>{/if}
        </ul>
        <div class="modal-actions">
          <button class="ghost" onclick={cancelAction} disabled={applying}>Cancel</button>
          <button class="danger" onclick={confirmApply} disabled={applying}>{applying ? "Applying…" : "Confirm"}</button>
        </div>
      </div>
    </div>
  {/if}

  {#if migratePlanOpen && migrateActions !== null}
    <MigratePlanModal
      actions={migrateActions}
      checked={migrateChecked}
      {originSpec}
      {destSpec}
      destructive={migrateDestructive}
      moveRequested={migrateFlags.delete_origin}
      bind:scrollTop={migratePlanScroll}
      onback={() => { migratePlanOpen = false; migratePanel = true; }}
      onclose={() => (migratePlanOpen = false)}
      onapply={startMigrateApply}
      onview={openFileFromPlan}
    />
  {/if}

  {#if migrateProgressOpen}
    <MigrateProgressModal
      outcomes={migrateOutcomes}
      applying={migrateApplying}
      doneSummary={migrateDoneSummary}
      moveSummary={migrateMoveSummary}
      phase={migratePhase}
      {originSpec}
      {destSpec}
      checkedTotal={planCheckedTotal}
      oncancel={cancelMigrate}
      oncontinue={continueMigrate}
    />
  {/if}

  {#if migrateBusy}
    <div class="prep-overlay">
      <div class="prep-card">
        <span class="spinner prep-spinner"></span>
        <p class="prep-title">Preparing plan…</p>
        <p class="prep-sub">
          {planProgress > 0
            ? `${planProgress.toLocaleString()} items examined…`
            : "Comparing the selected sources — this can take a moment for large trees."}
        </p>
      </div>
    </div>
  {/if}

  {#if migratePanel}
    <MigrateConfigPanel
      bind:method={migrateMethod}
      bind:exclude={migrateExclude}
      bind:flags={migrateFlags}
      bind:backupAck={migrateBackupAck}
      busy={migrateBusy}
      {originSpec}
      {destSpec}
      onchange={() => (migrateActions = null)}
      onpreview={previewMigrate}
      onclose={() => (migratePanel = false)}
    />
  {/if}

  {#if syncPanel}
    <SyncConfigPanel
      bind:method={migrateMethod}
      bind:exclude={migrateExclude}
      bind:flags={syncFlags}
      bind:backupAck={syncBackupAck}
      busy={migrateBusy}
      {originSpec}
      {destSpec}
      onchange={() => (migrateActions = null)}
      onpreview={previewSync}
      onclose={() => (syncPanel = false)}
    />
  {/if}

  {#if syncPlanOpen && migrateActions !== null}
    <SyncPlanModal
      actions={migrateActions}
      checked={migrateChecked}
      {originSpec}
      {destSpec}
      bind:scrollTop={syncPlanScroll}
      onback={() => { syncPlanOpen = false; syncPanel = true; }}
      onclose={() => (syncPlanOpen = false)}
      onapply={startMigrateApply}
      onview={openFileFromPlan}
    />
  {/if}

  {#if largeFileConfirm}
    <div class="overlay">
      <div class="modal">
        <h2>Large file</h2>
        <p><strong>{largeFileConfirm.name}</strong></p>
        <p>
          This file is too large for a full diff
          {largeFileConfirm.leftSize > 0 ? `(${(Math.max(largeFileConfirm.leftSize, largeFileConfirm.rightSize) / 1_000_000).toFixed(1)} MB)` : "(> 2 MB)"}.
          For text files, Confold will show the first differences found (up to 10 MB read per side).
          For binary files, it will show the first 256 KB in hex view.
        </p>
        <p class="large-file-note">The view will be <strong>read-only</strong>. To copy this file entirely, use Copy → / ← Copy in the file list.</p>
        <div class="modal-actions">
          <button class="ghost" onclick={cancelLargeFile}>Cancel</button>
          <button onclick={openLargeFile}>Show differences</button>
        </div>
      </div>
    </div>
  {/if}
</main>

<style>
  :root {
    font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
    color: #1a1a1a;
    background: #f6f6f6;
  }
  /* Full window height+width. `height:100vh; overflow:hidden` anchors main to the viewport so that
     `.tree` / `.sbs` (flex:1; min-height:0) are bounded — virtualization stays active.
     The entry screen re-caps itself to a centered column (see `.entry`). */
  :global(body) { margin: 0; padding: 0; }
  main {
    padding: 1.2rem;
    box-sizing: border-box;
    height: 100vh;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  header {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 1.5rem;
    margin-bottom: 0.4rem;
  }
  .brand-logo {
    height: 2.6rem;
    width: auto;
    flex: none;
  }
  h1 {
    margin: 0;
    font-size: 1.6rem;
  }
  .titlewrap {
    display: flex;
    align-items: baseline;
    gap: 0.45rem;
  }
  .appver {
    font-size: 0.9rem;
    font-weight: 400;
    opacity: 0.5;
  }
  .sources {
    margin: 1rem 0 0.6rem;
    display: flex;
    gap: 1.5rem;
    align-items: flex-start;
    justify-content: center;
  }
  .side {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.6rem;
  }
  .add {
    width: 100%;
    max-width: 300px;
    min-height: 96px;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(57, 108, 216, 0.08);
    color: inherit;
    border: 1.5px dashed #396cd8;
    border-radius: 10px;
    padding: 0.8rem;
    font-weight: 600;
    font-size: 0.95rem;
  }
  .add .plus {
    font-size: 1.2em;
    font-weight: 700;
    color: #396cd8;
  }
  .chosen {
    width: 100%;
    max-width: 300px;
    min-height: 96px;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: rgba(46, 160, 67, 0.14);
    color: inherit;
    border: 1.5px solid #2ea043;
    border-radius: 10px;
    padding: 0.8rem;
    text-align: left;
    overflow: hidden;
  }
  .chosen.stale {
    background: rgba(192, 57, 43, 0.1);
    border-color: #c0392b;
  }
  .chosen .ico {
    font-size: 1.2em;
    flex: none;
  }
  .chosen .slug {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, monospace;
    font-size: 0.85rem;
  }
  .chosen .change {
    flex: none;
    font-size: 0.72rem;
    opacity: 0.7;
    text-transform: uppercase;
    color: #2ea043;
  }
  .recents {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.3rem;
    max-width: 100%;
  }
  .rlabel {
    font-size: 0.66rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.45;
    margin-bottom: 0.1rem;
  }
  .recent {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    max-width: 100%;
    background: rgba(128, 128, 128, 0.12);
    color: inherit;
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 999px;
    padding: 0.2rem 0.6rem;
    font-size: 0.76rem;
    overflow: hidden;
  }
  .recent.stale {
    color: #c0392b;
    border-color: rgba(192, 57, 43, 0.4);
  }
  .recent .ico {
    flex: none;
  }
  .recent .rslug {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, monospace;
  }
  .opts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: 0.8rem;
  }
  .action-bar {
    display: flex;
    justify-content: center;
    margin: 0.4rem 0 0.8rem;
  }
  /* Split button */
  .split-btn {
    position: relative;
    display: inline-flex;
    border-radius: 8px;
    overflow: visible;
    box-shadow: 0 1px 3px rgba(0,0,0,0.15);
  }
  .split-main {
    border: 1px solid #396cd8;
    border-right: none;
    border-radius: 8px 0 0 8px;
    background: #396cd8;
    color: #fff;
    font-weight: 600;
    font-size: 0.95rem;
    padding: 0.45em 1.2em;
    cursor: pointer;
    min-width: 7rem;
  }
  .split-main:disabled { opacity: 0.45; cursor: not-allowed; }
  .split-sep {
    width: 1px;
    background: rgba(255,255,255,0.35);
    flex: none;
  }
  .split-arrow {
    border: 1px solid #396cd8;
    border-left: none;
    border-radius: 0 8px 8px 0;
    background: #396cd8;
    color: #fff;
    font-size: 0.9rem;
    padding: 0.45em 0.65em;
    cursor: pointer;
    line-height: 1;
  }
  .split-arrow:disabled { opacity: 0.45; cursor: not-allowed; }
  .split-main:not(:disabled):hover,
  .split-arrow:not(:disabled):hover { filter: brightness(1.1); }
  /* Dropdown menu */
  .mode-menu {
    position: absolute;
    bottom: calc(100% + 6px);
    top: auto;
    left: 0;
    min-width: 18rem;
    background: #fff;
    border: 1px solid #d0ccc5;
    border-radius: 8px;
    box-shadow: 0 4px 16px rgba(0,0,0,0.14);
    z-index: 50;
    overflow: hidden;
  }
  .mode-item {
    display: flex;
    flex-direction: column;
    gap: 0.15em;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    padding: 0.6em 0.9em;
    cursor: pointer;
    border-bottom: 1px solid #eee;
  }
  .mode-item:last-child { border-bottom: none; }
  .mode-item:hover { background: #f4f2ee; }
  .mode-item.active { background: #eef2fc; }
  .mode-name {
    font-weight: 600;
    font-size: 0.9rem;
    display: flex;
    align-items: center;
    gap: 0.4em;
  }
  .mode-desc {
    font-size: 0.78rem;
    opacity: 0.65;
    line-height: 1.3;
  }
  @media (prefers-color-scheme: dark) {
    .mode-menu { background: #1e1c18; border-color: #3a3630; }
    .mode-item { border-bottom-color: #2e2c28; }
    .mode-item:hover { background: #2a2824; }
    .mode-item.active { background: #1e2a40; }
  }
  .entry {
    display: flex;
    flex-direction: column;
    min-height: 79vh;
    max-width: 1000px;
    margin: 0 auto;
  }
  .entry header {
    margin-top: 3vh;
  }
  .entry-mid {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }
  .entry-bottom {
    padding-bottom: 0;
  }
  .entry-shell-toggle {
    position: fixed;
    bottom: 0.5rem;
    left: 0.75rem;
    display: flex;
    align-items: center;
    gap: 0.6rem;
    opacity: 0.5;
    font-size: 0.8rem;
  }
  .entry-shell-toggle:hover {
    opacity: 1;
  }
  .shell-msg {
    color: #2a8c3f;
  }
  .compact-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.6rem;
    padding: 0.5rem 0;
    margin-bottom: 0.4rem;
    border-bottom: 1px solid rgba(128, 128, 128, 0.3);
    font-size: 0.85rem;
  }
  .compact-logo {
    height: 1.5rem;
    width: auto;
    flex: none;
  }
  .mini-src {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    max-width: 38%;
    background: rgba(46, 160, 67, 0.14);
    color: inherit;
    border: 1px solid #2ea043;
    border-radius: 8px;
    padding: 0.3rem 0.6rem;
    overflow: hidden;
    font-weight: 600;
  }
  .mini-slug {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, monospace;
    font-size: 0.82rem;
  }
  .arrow {
    opacity: 0.6;
    font-weight: 700;
  }
  .mini-opts {
    display: inline-flex;
    align-items: center;
    gap: 0.6rem;
    margin-left: auto;
  }
  .ctl {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-weight: 600;
    font-size: 0.85rem;
  }
  .ctl .ex {
    width: 12rem;
  }
  .ctl .tol {
    width: 5rem;
  }
  .mini-refresh {
    border-radius: 7px;
    border: 1px solid #aaa;
    background: transparent;
    color: inherit;
    font-weight: 700;
    font-size: 1rem;
    padding: 0.2em 0.55em;
    cursor: pointer;
    line-height: 1;
  }
  .mini-refresh:hover { background: rgba(128,128,128,0.12); }
  .loading {
    padding: 1.2rem 0;
    opacity: 0.6;
    font-size: 0.9rem;
  }
  .warn {
    text-align: center;
    color: #e5484d;
    font-size: 0.85rem;
    margin: 0.2rem 0;
  }
  .crumbs {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.3rem;
    padding: 0.4rem 0 0.2rem;
  }
  .crumb {
    background: rgba(128, 128, 128, 0.12);
    color: inherit;
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 6px;
    padding: 0.15rem 0.55rem;
    font-size: 0.8rem;
    font-family: ui-monospace, monospace;
    font-weight: 600;
  }
  .csep {
    opacity: 0.4;
  }
  input,
  select,
  button {
    border-radius: 7px;
    border: 1px solid #ccc;
    padding: 0.45em 0.7em;
    font-size: 0.9rem;
    font-family: inherit;
    background: #fff;
    color: inherit;
  }
  button {
    cursor: pointer;
    font-weight: 600;
    border-color: #396cd8;
    background: #396cd8;
    color: #fff;
  }
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  button.danger {
    border-color: #e5484d;
    background: #e5484d;
  }
  button.ghost {
    background: transparent;
    color: inherit;
    border-color: #aaa;
  }
  .error {
    color: #e5484d;
    background: #fdecec;
    padding: 0.6rem 0.8rem;
    border-radius: 7px;
    font-family: ui-monospace, monospace;
    font-size: 0.85rem;
  }
  .result {
    background: #e9f6ec;
    padding: 0.5rem 0.8rem;
    border-radius: 7px;
    font-size: 0.85rem;
  }
  .summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem 0.6rem;
    padding: 0.5rem 0;
    border-bottom: 1px solid rgba(128, 128, 128, 0.25);
  }
  .num {
    font-weight: 700;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 0;
  }
  .actions .seln {
    font-size: 0.82rem;
    opacity: 0.7;
    margin-right: auto;
  }
  .actions button {
    padding: 0.3em 0.6em;
    font-size: 0.82rem;
  }
  .leg {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    background: rgba(128, 128, 128, 0.08);
    border: 1px solid rgba(128, 128, 128, 0.35);
    border-radius: 999px;
    padding: 0.2rem 0.7rem;
    margin: 0;
    color: inherit;
    font: inherit;
    font-size: 0.72rem;
    cursor: pointer;
  }
  .leg.off {
    opacity: 0.5;
    text-decoration: line-through;
  }
  .tick {
    width: 0.85em;
    text-align: center;
    color: #2ea043;
    font-weight: 700;
  }
  .sq {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 1px solid rgba(128, 128, 128, 0.6);
    border-radius: 2px;
  }
  .sq.src {
    background: linear-gradient(to right, #4c8bf5 50%, transparent 50%);
  }
  .sq.dst {
    background: linear-gradient(to right, transparent 50%, #2bb3a3 50%);
  }
  .sq.eq {
    background: #fff;
  }
  .sq.diff {
    background: #e8a33d;
  }
  .sq.err {
    background: #e5484d;
    border-color: #e5484d;
  }
  .dotleg {
    font-weight: 700;
    opacity: 0.6;
  }
  /* Live "still comparing N files" indicator in the summary bar. */
  .scanning {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.72rem;
    opacity: 0.75;
    margin-left: 0.2rem;
  }
  .scanning .spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 2px solid rgba(128, 128, 128, 0.25);
    border-top-color: #888;
    border-radius: 50%;
    animation: spin 0.65s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  /* Preload-in-progress indicator (compact bar, non-lazy mode). */
  .mini-preloading {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.78rem;
    opacity: 0.7;
    margin-left: 0.3rem;
  }
  .mini-preloading .spinner {
    display: inline-block;
    width: 11px;
    height: 11px;
    border: 2px solid rgba(128, 128, 128, 0.25);
    border-top-color: #888;
    border-radius: 50%;
    animation: spin 0.65s linear infinite;
  }
  /* "Preparing plan…" overlay (shown while the migrate/sync action list is being computed). */
  .prep-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
  }
  .prep-card {
    background: #2a2a2a;
    color: #f0f0f0;
    border-radius: 10px;
    padding: 1.6rem 2rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.6rem;
    text-align: center;
    max-width: 380px;
  }
  .prep-spinner {
    display: inline-block;
    width: 28px;
    height: 28px;
    border: 3px solid rgba(128, 128, 128, 0.3);
    border-top-color: #4c8bf5;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  .prep-title {
    margin: 0;
    font-weight: 700;
  }
  .prep-sub {
    margin: 0;
    font-size: 0.85rem;
    opacity: 0.7;
  }
  .tree-wrap {
    container-type: inline-size;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .thead {
    display: flex;
    align-items: center;
    font-size: 0.72rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    opacity: 0.55;
    border-bottom: 1px solid #ccc;
  }
  .th-name {
    flex: 1;
    padding-left: 0.5em;
  }
  .th-sz {
    flex: none;
    width: 6em;
    text-align: right;
    padding: 0.25rem 0.6em;
    border-left: 1px solid rgba(128, 128, 128, 0.18);
  }
  .th-dt {
    flex: none;
    width: 9.5em;
    text-align: right;
    padding: 0.25rem 0.6em;
    border-left: 1px solid rgba(128, 128, 128, 0.18);
  }
  @container (max-width: 850px) {
    .th-dt { display: none; }
  }
  @container (max-width: 550px) {
    .th-sz { display: none; }
  }
  .tree {
    border: 1px solid #ddd;
    border-radius: 8px;
    overflow: auto;
    background: #fff;
    flex: 1 1 auto;
    min-height: 0;
    position: relative;
    scrollbar-gutter: stable;
  }
  .spacer {
    position: relative;
  }
  .vrow {
    position: absolute;
    left: 0;
    right: 0;
    height: 24px;
  }
  /* Keyboard-navigation cursor highlight (distinct from hover and from checkbox selection). */
  .vrow.cursor {
    background: rgba(57, 108, 216, 0.16);
    box-shadow: inset 2px 0 0 #396cd8;
  }
  @media (prefers-color-scheme: dark) {
    .vrow.cursor {
      background: rgba(86, 140, 255, 0.22);
    }
  }
  /* Large-file hunks mode status bar */
  .large-file-bar {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.3rem 0.5rem;
    font-size: 0.8rem;
    border-top: 1px solid rgba(128, 128, 128, 0.2);
    background: rgba(232, 163, 61, 0.06);
    flex-shrink: 0;
  }
  .large-file-bar button {
    font-size: 0.78rem;
    padding: 0.2em 0.7em;
  }
  .lf-notice {
    margin-left: auto;
    opacity: 0.65;
    font-style: italic;
  }
  .large-file-note {
    font-size: 0.85rem;
    background: rgba(232, 163, 61, 0.1);
    border: 1px solid rgba(232, 163, 61, 0.4);
    border-radius: 6px;
    padding: 0.4rem 0.6rem;
    margin: 0;
  }
  .empty {
    padding: 1rem;
    opacity: 0.6;
  }
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .modal {
    background: #fff;
    color: #1a1a1a;
    border-radius: 10px;
    padding: 1.2rem;
    width: min(560px, 90vw);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
  }
  .modal h2 {
    margin: 0 0 0.4rem;
    font-size: 1.1rem;
  }
  .planlist {
    overflow: auto;
    background: #f3f3f3;
    border-radius: 6px;
    padding: 0.5rem 1.2rem;
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
    margin: 0.5rem 0;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
    margin-top: 0.6rem;
  }

  /* (Migrate config/plan/progress modal styles moved to their Migrate*.svelte components.) */

  @media (prefers-color-scheme: dark) {
    :root {
      color: #f0f0f0;
      background: #1e1e1e;
    }
    input,
    select {
      background: #2a2a2a;
      border-color: #444;
      color: #f0f0f0;
    }
    .tree {
      background: #232323;
      border-color: #3a3a3a;
    }
    .summary {
      border-color: #3a3a3a;
    }
    .error {
      background: #3a1f20;
    }
    .result {
      background: #1f3a26;
    }
    .shell-msg {
      color: #6abf7a;
    }
    .recent.stale {
      color: #e74c3c;
    }
    .chosen.stale {
      background: rgba(231, 76, 60, 0.12);
      border-color: #e74c3c;
    }
    .modal {
      background: #2a2a2a;
      color: #f0f0f0;
    }
    .planlist {
      background: #1e1e1e;
    }
  }
</style>
