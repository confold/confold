import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const { testSource } = vi.hoisted(() => ({
  testSource: vi.fn(),
}));

vi.mock("$lib/commands", () => ({
  commands: {
    testSource,
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
}));

import SourcePicker from "./SourcePicker.svelte";
import type { SourceTypeInfo } from "$lib/types";

const NUXEO_TYPE: SourceTypeInfo = {
  id: "nuxeo",
  name: "Nuxeo",
  icon: "nuxeo",
  fields: [
    { key: "url", label: "Server URL", kind: "text", required: true },
    { key: "username", label: "Username", kind: "text", required: true },
    { key: "password", label: "Password", kind: "password", required: true, secret: true },
    { key: "root", label: "Document path", kind: "path", required: false },
  ],
} as unknown as SourceTypeInfo;

const TYPES = [NUXEO_TYPE];

function fillCompleteForm() {
  return (async () => {
    await fireEvent.click(screen.getByRole("button", { name: "Nuxeo" }));
    await fireEvent.input(screen.getByLabelText(/Server URL/), {
      target: { value: "https://nuxeo.example/nuxeo" },
    });
    await fireEvent.input(screen.getByLabelText(/Username/), { target: { value: "reader" } });
    await fireEvent.input(screen.getByLabelText(/Password/), { target: { value: "secret" } });
  })();
}

beforeEach(() => {
  testSource.mockReset();
});

describe("SourcePicker", () => {
  it("never probes the connection on its own — a complete form is selectable unverified", async () => {
    const onconfirm = vi.fn();
    render(SourcePicker, {
      title: "source",
      types: TYPES,
      onconfirm,
      oncancel: vi.fn(),
    });
    await fillCompleteForm();
    const select = await screen.findByRole("button", { name: /Select source/ });
    await waitFor(() => expect((select as HTMLButtonElement).disabled).toBe(false));
    expect(testSource).not.toHaveBeenCalled();
  });

  it("tests only when the user presses Verify connection, and shows the result", async () => {
    testSource.mockResolvedValue({ ok: true, is_dir: true, message: "Connected" });
    const onconfirm = vi.fn();
    render(SourcePicker, {
      title: "source",
      types: TYPES,
      onconfirm,
      oncancel: vi.fn(),
    });
    await fillCompleteForm();
    await fireEvent.click(screen.getByRole("button", { name: "Verify connection" }));
    await waitFor(() => expect(screen.getByText(/Connected/)).toBeTruthy());
    expect(testSource).toHaveBeenCalledTimes(1);
  });

  it("clears a previous verification when any field is edited", async () => {
    testSource.mockResolvedValue({ ok: true, is_dir: true, message: "Connected" });
    render(SourcePicker, {
      title: "source",
      types: TYPES,
      onconfirm: vi.fn(),
      oncancel: vi.fn(),
    });
    await fillCompleteForm();
    await fireEvent.click(screen.getByRole("button", { name: "Verify connection" }));
    await waitFor(() => expect(screen.getByText(/Connected/)).toBeTruthy());
    await fireEvent.input(screen.getByLabelText(/Username/), { target: { value: "reader2" } });
    expect(screen.queryByText(/Connected/)).toBeNull();
  });

  it("toggles a password field between hidden and visible", async () => {
    render(SourcePicker, {
      title: "source",
      types: TYPES,
      onconfirm: vi.fn(),
      oncancel: vi.fn(),
    });
    await fireEvent.click(screen.getByRole("button", { name: "Nuxeo" }));
    const secret = screen.getByLabelText(/Password/) as HTMLInputElement;
    expect(secret.type).toBe("password");
    await fireEvent.click(screen.getByRole("button", { name: "Show password" }));
    expect((screen.getByLabelText(/Password/) as HTMLInputElement).type).toBe("text");
    await fireEvent.click(screen.getByRole("button", { name: "Hide password" }));
    expect((screen.getByLabelText(/Password/) as HTMLInputElement).type).toBe("password");
  });
});
