import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const { browse, search } = vi.hoisted(() => ({
  browse: vi.fn(),
  search: vi.fn(),
}));

vi.mock("$lib/commands", () => ({
  commands: {
    nuxeoBrowseRoutes: browse,
    nuxeoSearchRoutes: search,
  },
}));

import NuxeoPathPicker from "./NuxeoPathPicker.svelte";

const SPEC = { kind: "nuxeo", fields: { url: "https://nuxeo.example/nuxeo", username: "reader", password: "secret" } };
const route = (name: string, path: string) => ({ name, title: name, path });

beforeEach(() => {
  browse.mockReset();
  search.mockReset();
});

describe("NuxeoPathPicker", () => {
  it("navigates one folder at a time and selects the current path", async () => {
    browse.mockImplementation(async (_spec, path: string, pageIndex: number) => ({
      entries: path === "/"
        ? [route("Default domain", "/default-domain")]
        : [route("Workspaces", "/default-domain/workspaces")],
      has_more: false,
    }));
    const onselect = vi.fn();
    render(NuxeoPathPicker, { spec: SPEC, startPath: "/", onselect, oncancel: vi.fn() });

    await fireEvent.click(await screen.findByRole("button", { name: /Default domain/ }));
    await waitFor(() => expect(browse).toHaveBeenLastCalledWith(SPEC, "/default-domain", 0));
    expect(await screen.findByText("Workspaces")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Use this folder" }));
    expect(onselect).toHaveBeenCalledWith("/default-domain");
  });

  it("searches the repository globally and selects the matching route", async () => {
    browse.mockResolvedValue({ entries: [], has_more: false });
    search.mockResolvedValue({
      entries: [route("Contracts archive", "/default-domain/workspaces/contracts")],
      has_more: false,
    });
    const onselect = vi.fn();
    render(NuxeoPathPicker, { spec: SPEC, startPath: "/", onselect, oncancel: vi.fn() });

    const input = screen.getByRole("searchbox", { name: "Search folder names or absolute path prefixes" });
    await fireEvent.input(input, { target: { value: "contracts" } });
    const result = await screen.findByRole("button", { name: /Contracts archive/ });
    await fireEvent.click(result);

    expect(search).toHaveBeenCalledWith(SPEC, "contracts", 0);
    expect(onselect).toHaveBeenCalledWith("/default-domain/workspaces/contracts");
  });

  it("loads the next route page only when requested", async () => {
    browse.mockImplementation(async (_spec, _path: string, pageIndex: number) => ({
      entries: [route(pageIndex === 0 ? "First folder" : "Next folder", `/folder-${pageIndex}`)],
      has_more: pageIndex === 0,
    }));
    render(NuxeoPathPicker, { spec: SPEC, startPath: "/", onselect: vi.fn(), oncancel: vi.fn() });

    expect(await screen.findByText("First folder")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Load more folders" }));
    expect(await screen.findByText("Next folder")).toBeTruthy();
    expect(browse).toHaveBeenLastCalledWith(SPEC, "/", 1);
  });

  it("shows route listing errors and retries the same path", async () => {
    browse
      .mockRejectedValueOnce(new Error("Nuxeo listing /: HTTP 403"))
      .mockResolvedValueOnce({ entries: [route("Default domain", "/default-domain")], has_more: false });
    render(NuxeoPathPicker, { spec: SPEC, startPath: "/", onselect: vi.fn(), oncancel: vi.fn() });

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Nuxeo listing /: HTTP 403");
    await fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Default domain", { exact: true })).toBeTruthy();
    expect(browse).toHaveBeenCalledTimes(2);
  });
});
