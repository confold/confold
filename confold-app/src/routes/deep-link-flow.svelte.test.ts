// @vitest-environment jsdom
import { render, waitFor } from "@testing-library/svelte";
import { beforeEach, expect, it, vi } from "vitest";

const ORIGIN = "/tmp/confold-left.txt";
const DESTINATION = "/tmp/confold-right.txt";
const DEEP_LINK =
  `confold://compare?origin=${encodeURIComponent(ORIGIN)}&destination=${encodeURIComponent(DESTINATION)}`;

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn((command: string) => {
    switch (command) {
      case "source_types":
        return Promise.resolve([]);
      case "load_recents":
        return Promise.resolve({ origins: [], destinations: [] });
      case "path_exists":
        return Promise.resolve(true);
      case "test_source":
        return Promise.resolve({ ok: true, is_dir: false, message: "Connected (file)" });
      case "diff_file":
        return Promise.resolve({
          result: {
            kind: "text",
            diff: {
              rows: [],
              summary: { equal: 0, inserted: 0, deleted: 0, replaced: 0 },
            },
          },
          left: { fp: "", eol: "\n", final_nl: false, mtime: null, created: null },
          right: { fp: "", eol: "\n", final_nl: false, mtime: null, created: null },
        });
      default:
        return Promise.resolve(undefined);
    }
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: () => Promise.resolve("1.2.3") }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ close: vi.fn() }) }));
vi.mock("@tauri-apps/plugin-deep-link", () => ({
  getCurrent: () => Promise.resolve([DEEP_LINK]),
  onOpenUrl: () => Promise.resolve(() => {}),
}));

import Page from "./+page.svelte";

beforeEach(() => {
  invokeMock.mockClear();
  localStorage.clear();
});

it("opens two file paths from a deep link with the file diff command", async () => {
  render(Page);

  await waitFor(() => {
    expect(invokeMock.mock.calls.filter(([command]) => command === "test_source")).toHaveLength(2);
    expect(invokeMock.mock.calls.some(([command]) => command === "diff_file")).toBe(true);
  });

  expect(invokeMock.mock.calls.some(([command]) => command === "compare")).toBe(false);
});
