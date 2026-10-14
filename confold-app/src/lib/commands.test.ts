// Contract test for the typed Tauri bindings. The binding layer's value is compile-time types at the
// CALL sites, but the bindings themselves pass a plain string + object to `invoke`, so a typo in the
// command name or an argument key (e.g. snake_case where Tauri wants camelCase) wouldn't be caught by
// the compiler. This test locks that down: it asserts the exact (command, args) each wrapper emits.
// If you rename a command or arg, update the Rust signature, the binding, AND this test together.
import { describe, it, expect, beforeEach, vi } from "vitest";

const invokeMock = vi.hoisted(() =>
  vi.fn((_name: string, _args?: Record<string, unknown>) => Promise.resolve(undefined)),
);
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import { commands } from "./commands";

const SPEC = { kind: "fs", fields: { root: "/x" } };
const OPTS = { method: "full" as const, include: [], exclude: [] };
const FILE = { source: SPEC, rel: "a.txt" };

// The (name, args) of the most recent invoke call.
const last = () => invokeMock.mock.calls.at(-1) as [string, Record<string, unknown>?];

beforeEach(() => invokeMock.mockClear());

describe("command bindings → invoke contract", () => {
  it("compare / compareLevel", () => {
    commands.compare(SPEC, SPEC, OPTS, 7);
    expect(last()).toEqual(["compare", { left: SPEC, right: SPEC, opts: OPTS, token: 7 }]);
    commands.compareLevel(SPEC, SPEC, OPTS, "sub", 7);
    expect(last()).toEqual(["compare_level", { left: SPEC, right: SPEC, opts: OPTS, rel: "sub", token: 7 }]);
  });

  it("migrateActions / syncActions pass flags untouched", () => {
    const mflags = { copy_new: true, overwrite_different: true, delete_extra: false, delete_origin: false };
    commands.migrateActions(SPEC, SPEC, OPTS, mflags, 11);
    expect(last()).toEqual(["migrate_actions", { left: SPEC, right: SPEC, opts: OPTS, flags: mflags, token: 11 }]);
    const sflags = { trust_left: true, trust_right: false, delete_diffs: true, conflict_rule: "newer" as const };
    commands.syncActions(SPEC, SPEC, OPTS, sflags, 12);
    expect(last()).toEqual(["sync_actions", { left: SPEC, right: SPEC, opts: OPTS, flags: sflags, token: 12 }]);
  });

  it("migrateApply uses the camelCase deleteOrigin key (the bug that bit us)", () => {
    commands.migrateApply({ left: SPEC, right: SPEC, actions: [], generation: 3, deleteOrigin: true, opts: OPTS });
    const [name, args] = last();
    expect(name).toBe("migrate_apply");
    expect(args).toHaveProperty("deleteOrigin", true);
    expect(args).not.toHaveProperty("delete_origin");
    expect(args).toMatchObject({ generation: 3, opts: OPTS });
  });

  it("migrateCancel takes no args", () => {
    commands.migrateCancel();
    expect(last()).toEqual(["migrate_cancel"]);
  });

  it("plan/apply actions", () => {
    commands.planActions(SPEC, SPEC, []);
    expect(last()).toEqual(["plan_actions", { left: SPEC, right: SPEC, actions: [] }]);
    commands.applyActions(SPEC, SPEC, [], 9);
    expect(last()).toEqual(["apply_actions", { left: SPEC, right: SPEC, actions: [], generation: 9 }]);
  });

  it("diffFile / diffFileLarge (camelCase startHunk etc.)", () => {
    commands.diffFile(FILE, FILE);
    expect(last()).toEqual(["diff_file", { left: FILE, right: FILE }]);
    commands.diffFileLarge(FILE, FILE);
    expect(last()).toEqual(["diff_file_large", { left: FILE, right: FILE }]);
    commands.diffFileLarge(FILE, FILE, { startHunk: 5, maxBytes: 100 });
    const [, args] = last();
    expect(args).toHaveProperty("startHunk", 5);
    expect(args).toHaveProperty("maxBytes", 100);
    expect(args).not.toHaveProperty("start_hunk");
  });

  it("diffStrings / saveFile / hexCompare / readBytes / sourceTypes / testSource", () => {
    commands.diffStrings("a", "b");
    expect(last()).toEqual(["diff_strings", { left: "a", right: "b" }]);
    commands.saveFile(FILE, "x", "fp", false);
    expect(last()).toEqual(["save_file", { file: FILE, contents: "x", expect: "fp", force: false }]);
    commands.hexCompare(FILE, FILE);
    expect(last()).toEqual(["hex_compare", { left: FILE, right: FILE }]);
    commands.readBytes(FILE);
    expect(last()).toEqual(["read_bytes", { file: FILE }]);
    commands.sourceTypes();
    expect(last()).toEqual(["source_types"]);
    commands.testSource(SPEC);
    expect(last()).toEqual(["test_source", { spec: SPEC }]);
  });

  it("Nuxeo route browse/search use camelCase invoke arguments", () => {
    commands.nuxeoBrowseRoutes(SPEC, "/default-domain", 2);
    expect(last()).toEqual([
      "nuxeo_browse_routes",
      { spec: SPEC, path: "/default-domain", pageIndex: 2 },
    ]);
    commands.nuxeoSearchRoutes(SPEC, "contracts", 1);
    expect(last()).toEqual([
      "nuxeo_search_routes",
      { spec: SPEC, query: "contracts", pageIndex: 1 },
    ]);
  });

  it("shell integration commands use the exact Tauri command names", () => {
    commands.installShellIntegration();
    expect(last()).toEqual(["install_shell_integration"]);
    commands.uninstallShellIntegration();
    expect(last()).toEqual(["uninstall_shell_integration"]);
    const status: Promise<{ supported: boolean; installed: boolean }> =
      commands.shellIntegrationStatus();
    void status;
    expect(last()).toEqual(["shell_integration_status"]);
  });

  it("AI provider commands preserve the typed settings contract", () => {
    const request = {
      profile: {
        kind: "open_ai_compatible" as const,
        baseUrl: "https://provider.example/v1",
        model: "model-a",
        useApiKey: true,
      },
      apiKey: "secret",
      clearApiKey: false,
      customHeaders: { "x-route": "private" },
      clearCustomHeaders: false,
    };
    commands.loadProviderSettings();
    expect(last()).toEqual(["load_provider_settings"]);
    commands.saveProviderSettings(request);
    expect(last()).toEqual(["save_provider_settings", { request }]);
    commands.listProviderModels();
    expect(last()).toEqual(["list_provider_models"]);
    commands.testProviderCapability();
    expect(last()).toEqual(["test_provider_capability"]);
  });

  it("semantic commands preserve review, preference, and apply arguments", () => {
    const proposal = {
      schema_version: 1,
      operation_id: "operation-1",
      verdict: "merged" as const,
      summary: "Merged fixture",
      contributions: [],
      warnings: [],
      result: "merged",
    };
    commands.semanticReview(FILE, FILE);
    expect(last()).toEqual(["semantic_review", { left: FILE, right: FILE }]);
    commands.loadSemanticPreferences();
    expect(last()).toEqual(["load_semantic_preferences"]);
    commands.saveSemanticPreferences({ language: "es" });
    expect(last()).toEqual(["save_semantic_preferences", { preferences: { language: "es" } }]);
    commands.semanticApplyTo(FILE, FILE, proposal, "destination");
    expect(last()).toEqual([
      "semantic_apply_to",
      { left: FILE, right: FILE, proposal, target: "destination" },
    ]);
    commands.semanticApplyNew(FILE, FILE, proposal, "/tmp/merged.md");
    expect(last()).toEqual([
      "semantic_apply_new",
      { left: FILE, right: FILE, proposal, output: "/tmp/merged.md" },
    ]);
  });
});
