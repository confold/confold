// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const { invokeMock, saveDialogMock, runtime } = vi.hoisted(() => {
  const runtime = {
    verifiedModel: null as string | null,
    semanticReviewPromise: null as Promise<unknown> | null,
  };
  return {
    runtime,
    saveDialogMock: vi.fn(() => Promise.resolve<string | null>("/tmp/docs/guide-merged.md")),
    invokeMock: vi.fn((command: string) => {
      if (command === "load_provider_settings") {
        return Promise.resolve({
          profile: {
            kind: "ollama",
            baseUrl: "http://127.0.0.1:11434/v1",
            model: "qwen3:8b",
            useApiKey: false,
          },
          hasApiKey: false,
          hasCustomHeaders: false,
          verifiedModel: runtime.verifiedModel,
        });
      }
      if (command === "semantic_review") {
        if (runtime.semanticReviewPromise) return runtime.semanticReviewPromise;
        return Promise.resolve({
          kind: "review",
          model: "qwen3:8b",
          configuration: {
            provider: "ollama",
            endpoint: "http://127.0.0.1:11434/v1",
            requestedModel: "qwen3:4b-instruct",
            responseModel: "qwen3:8b",
            language: "es",
          },
          origin_path: "/tmp/docs/guide.md",
          destination_path: "/tmp/other/guide.md",
          proposal: {
            schema_version: 1,
            operation_id: "operation-1",
            verdict: "merged",
            summary: "Both documents contribute.",
            contributions: [
              { source: "left", intent: "Keep the introduction.", disposition: "preserved" },
              { source: "right", intent: "Keep the examples.", disposition: "preserved" },
            ],
            warnings: [],
            result: "# Guide\nIntroduction.\nExamples.\n",
          },
          report: {
            schema_version: 1,
            operation_id: "operation-1",
            verdict: "merged",
            applicable: true,
            summary: "Both documents contribute.",
            contributions: [
              { source: "left", intent: "Keep the introduction.", disposition: "preserved" },
              { source: "right", intent: "Keep the examples.", disposition: "preserved" },
            ],
            warnings: [],
            result_sha256: "fixture",
            diffs: [{ source: "left", unified_diff: "--- left\n+++ result\n" }],
          },
        });
      }
      if (command === "semantic_apply_to") {
        return Promise.resolve({ output: "/tmp/other/guide.md", sha256: "saved", byte_len: 32 });
      }
      if (command === "semantic_apply_new") {
        return Promise.resolve({ output: "/tmp/docs/guide-merged.md", sha256: "saved", byte_len: 32 });
      }
      return Promise.resolve(undefined);
    }),
  };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: saveDialogMock }));

import SemanticReviewAction from "./SemanticReviewAction.svelte";

const local = {
  source: { kind: "fs", fields: { root: "/tmp/docs" } },
  rel: "guide.md",
};

beforeEach(() => {
  invokeMock.mockClear();
  runtime.verifiedModel = null;
  runtime.semanticReviewPromise = null;
  saveDialogMock.mockClear();
});

afterEach(cleanup);

it("explains why provider setup is required before opening Settings", async () => {
  const onconfigure = vi.fn();
  render(SemanticReviewAction, { left: local, right: local, onconfigure });

  const button = await screen.findByRole<HTMLButtonElement>("button", {
    name: "Set up AI review",
  });
  await waitFor(() => expect(button.disabled).toBe(false));
  await fireEvent.click(button);

  const dialog = screen.getByRole("dialog", { name: "AI provider required" });
  expect(within(dialog).getByText(/configure and verify an AI provider/i)).not.toBeNull();
  expect(onconfigure).not.toHaveBeenCalled();

  await fireEvent.click(within(dialog).getByRole("button", { name: "Open Settings" }));
  expect(onconfigure).toHaveBeenCalledWith(button);
  expect(invokeMock).not.toHaveBeenCalledWith("semantic_review", expect.anything());
});

it("refreshes verification before deciding to reopen Settings", async () => {
  const onconfigure = vi.fn();
  render(SemanticReviewAction, { left: local, right: local, onconfigure });
  const button = await screen.findByRole<HTMLButtonElement>("button", {
    name: "Set up AI review",
  });
  await waitFor(() => expect(button.disabled).toBe(false));

  runtime.verifiedModel = "qwen3:8b";
  await fireEvent.click(button);

  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("semantic_review", { left: local, right: local });
  });
  expect(onconfigure).not.toHaveBeenCalled();
});

it("starts semantic review when requested from the comparison tree", async () => {
  runtime.verifiedModel = "qwen3:8b";
  render(SemanticReviewAction, { left: local, right: local, autostart: true });

  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("semantic_review", { left: local, right: local });
  });
});

it("offers explicit origin, destination, and save-as outcomes for an applicable review", async () => {
  runtime.verifiedModel = "qwen3:8b";
  render(SemanticReviewAction, { left: local, right: local });

  const button = await screen.findByRole<HTMLButtonElement>("button", {
    name: "Run semantic review",
  });
  await waitFor(() => expect(button.disabled).toBe(false));
  await fireEvent.click(button);

  expect(invokeMock).toHaveBeenCalledWith("semantic_review", { left: local, right: local });
  const dialog = await screen.findByRole("dialog", { name: "Semantic review" });
  expect(within(dialog).getByText("Both documents contribute.")).not.toBeNull();
  const configuration = within(dialog).getByRole("region", { name: "AI configuration used" });
  expect(within(configuration).getByText("Ollama")).not.toBeNull();
  expect(within(configuration).getByText("qwen3:4b-instruct")).not.toBeNull();
  expect(within(configuration).getByText("qwen3:8b")).not.toBeNull();
  expect(within(configuration).getByText("Español")).not.toBeNull();
  expect(within(dialog).getByText(/no files have changed yet/i)).not.toBeNull();
  expect(within(dialog).getByRole("button", { name: "Save to origin" })).not.toBeNull();
  expect(within(dialog).getByRole("button", { name: "Save to destination" })).not.toBeNull();
  expect(within(dialog).getByRole("button", { name: "Save as…" })).not.toBeNull();
});

it("confirms the exact destination path, applies there, and reports refresh", async () => {
  runtime.verifiedModel = "qwen3:8b";
  const onsaved = vi.fn();
  render(SemanticReviewAction, { left: local, right: local, onsaved });
  await fireEvent.click(await screen.findByRole("button", { name: "Run semantic review" }));
  const dialog = await screen.findByRole("dialog", { name: "Semantic review" });

  await fireEvent.click(within(dialog).getByRole("button", { name: "Save to destination" }));
  expect(within(dialog).getByText("/tmp/other/guide.md")).not.toBeNull();
  await fireEvent.click(within(dialog).getByRole("button", { name: "Replace destination" }));

  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("semantic_apply_to", {
      left: local,
      right: local,
      proposal: expect.objectContaining({ operation_id: "operation-1" }),
      target: "destination",
    });
  });
  expect(onsaved).toHaveBeenCalledWith("right");
  expect(await within(dialog).findByText(/saved to \/tmp\/other\/guide.md/i)).not.toBeNull();
});

it("uses the native save dialog and the no-clobber apply command for save as", async () => {
  runtime.verifiedModel = "qwen3:8b";
  render(SemanticReviewAction, { left: local, right: local });
  await fireEvent.click(await screen.findByRole("button", { name: "Run semantic review" }));
  const dialog = await screen.findByRole("dialog", { name: "Semantic review" });

  await fireEvent.click(within(dialog).getByRole("button", { name: "Save as…" }));

  await waitFor(() => {
    expect(saveDialogMock).toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledWith("semantic_apply_new", {
      left: local,
      right: local,
      proposal: expect.objectContaining({ operation_id: "operation-1" }),
      output: "/tmp/docs/guide-merged.md",
    });
  });
});

it("opens a visible progress dialog while the provider is reviewing", async () => {
  runtime.verifiedModel = "qwen3:8b";
  runtime.semanticReviewPromise = new Promise(() => {});
  render(SemanticReviewAction, { left: local, right: local });

  const button = await screen.findByRole<HTMLButtonElement>("button", {
    name: "Run semantic review",
  });
  await waitFor(() => expect(button.disabled).toBe(false));
  await fireEvent.click(button);

  const dialog = await screen.findByRole("dialog", { name: "Semantic review" });
  expect(within(dialog).getByRole("status").textContent).toContain("Reviewing with qwen3:8b");
  expect(within(dialog).getByRole<HTMLButtonElement>("button", { name: "Close Semantic review" }).disabled).toBe(true);
  expect(button.disabled).toBe(true);
});

it("does not expose the action for remote file pairs", () => {
  const remote = { source: { kind: "sftp", fields: {} }, rel: "guide.md" };
  render(SemanticReviewAction, { left: remote, right: remote });
  expect(screen.queryByRole("button", { name: "Run semantic review" })).toBeNull();
});

it("does not expose the action for unsupported local text formats", () => {
  const structured = { ...local, rel: "config.json" };
  render(SemanticReviewAction, { left: structured, right: structured });
  expect(screen.queryByRole("button")).toBeNull();
});
