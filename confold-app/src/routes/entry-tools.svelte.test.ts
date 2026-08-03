// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { beforeEach, expect, it, vi } from "vitest";

const { closeMock, invokeMock, runtime } = vi.hoisted(() => {
  const runtime = {
    shellSupported: true,
    shellInstalled: false,
    providerProfile: {
      kind: "ollama",
      baseUrl: "http://127.0.0.1:11434/v1",
      model: "",
      useApiKey: false,
    },
    capabilityPromise: null as Promise<unknown> | null,
    semanticLanguage: "en" as "en" | "es",
  };
  return {
    closeMock: vi.fn(),
    runtime,
    invokeMock: vi.fn(
      (
        command: string,
        args?: {
          request?: { profile?: typeof runtime.providerProfile };
          preferences?: { language: "en" | "es" };
        },
      ) => {
      switch (command) {
        case "source_types":
          return Promise.resolve([]);
        case "load_recents":
          return Promise.resolve({ origins: [], destinations: [] });
        case "shell_integration_status":
          return Promise.resolve({
            supported: runtime.shellSupported,
            installed: runtime.shellInstalled,
          });
        case "load_provider_settings":
          return Promise.resolve({
            profile: runtime.providerProfile,
            hasApiKey: false,
            hasCustomHeaders: false,
            verifiedModel: null,
          });
        case "save_provider_settings":
          if (args?.request?.profile) runtime.providerProfile = args.request.profile;
          return Promise.resolve({
            profile: runtime.providerProfile,
            hasApiKey: false,
            hasCustomHeaders: false,
            verifiedModel: null,
          });
        case "load_semantic_preferences":
          return Promise.resolve({ language: runtime.semanticLanguage });
        case "save_semantic_preferences":
          if (args?.preferences) runtime.semanticLanguage = args.preferences.language;
          return Promise.resolve({ language: runtime.semanticLanguage });
        case "list_provider_models":
          return Promise.resolve(["qwen3:8b", "gpt-oss:20b"]);
        case "test_provider_capability":
          if (runtime.capabilityPromise) return runtime.capabilityPromise;
          return Promise.resolve({ model: "qwen3:8b" });
        case "install_shell_integration":
        case "uninstall_shell_integration":
          return Promise.resolve(undefined);
        default:
          return Promise.resolve(undefined);
      }
      },
    ),
  };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: () => Promise.resolve("1.2.3") }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ close: closeMock }),
}));
vi.mock("@tauri-apps/plugin-deep-link", () => ({
  getCurrent: () => Promise.resolve([]),
  onOpenUrl: () => Promise.resolve(() => {}),
}));

import Page from "./+page.svelte";
import EntryTools from "./EntryTools.svelte";

beforeEach(() => {
  closeMock.mockClear();
  invokeMock.mockClear();
  runtime.shellSupported = true;
  runtime.shellInstalled = false;
  runtime.capabilityPromise = null;
  runtime.semanticLanguage = "en";
  localStorage.clear();
});

it("moves folder integration into the Settings dialog and keeps it operational", async () => {
  render(Page);

  expect(screen.queryByLabelText("Folder right-click integration")).toBeNull();
  await fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));

  const dialog = screen.getByRole("dialog", { name: "Settings" });
  expect(dialog).not.toBeNull();

  const integration = await screen.findByRole<HTMLInputElement>("checkbox", {
    name: /Folder right-click integration/,
  });
  expect(integration.checked).toBe(false);

  await fireEvent.click(integration);
  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("install_shell_integration");
  });
  expect(integration.checked).toBe(true);
});

it("explains when folder integration is unavailable instead of offering a broken toggle", async () => {
  runtime.shellSupported = false;
  render(Page);

  await fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));

  expect(await screen.findByText("Not available on this platform")).not.toBeNull();
  expect(screen.queryByLabelText("Folder right-click integration")).toBeNull();
});

it("opens Help and About, closes it with Escape, and returns focus without quitting", async () => {
  render(Page);

  const helpButton = await screen.findByRole("button", { name: "Open help and about" });
  await fireEvent.click(helpButton);

  const dialog = screen.getByRole("dialog", { name: "Help and About" });
  expect(dialog).not.toBeNull();
  expect(await within(dialog).findByText("v1.2.3")).not.toBeNull();
  expect(
    screen.getByRole<HTMLAnchorElement>("link", { name: /Project website/ }).href,
  ).toBe("https://confold.com/");

  await fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });

  await waitFor(() => {
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(helpButton);
  });
  expect(closeMock).not.toHaveBeenCalled();
});

it("discovers models and performs a structured-output capability test", async () => {
  render(Page);
  await fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));

  const provider = await screen.findByRole<HTMLSelectElement>("combobox", {
    name: "Provider profile",
  });
  expect(provider.value).toBe("ollama");
  expect(
    screen.getByRole<HTMLInputElement>("textbox", { name: "Base URL" }).value,
  ).toBe("http://127.0.0.1:11434/v1");
  expect(screen.queryByLabelText("API key")).toBeNull();

  await fireEvent.click(screen.getByRole("button", { name: "Discover models" }));
  const model = await screen.findByRole<HTMLSelectElement>("combobox", { name: "Model" });
  expect(within(model).getAllByRole("option").map((option) => option.textContent)).toEqual([
    "qwen3:8b",
    "gpt-oss:20b",
    "Enter another identifier…",
  ]);
  await fireEvent.change(model, { target: { value: "qwen3:8b" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save and test" }));

  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("test_provider_capability");
  });
  expect(await screen.findByText(/Structured output verified/)).not.toBeNull();

  await fireEvent.change(model, { target: { value: "__manual__" } });
  await fireEvent.input(screen.getByRole("textbox", { name: "Model" }), {
    target: { value: "another-model" },
  });
  expect(screen.queryByText(/Structured output verified/)).toBeNull();
});

it("shows an explicit progress state while provider verification is running", async () => {
  runtime.capabilityPromise = new Promise(() => {});
  render(Page);
  await fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));
  await fireEvent.click(screen.getByRole("button", { name: "Discover models" }));
  await fireEvent.change(await screen.findByRole("combobox", { name: "Model" }), {
    target: { value: "qwen3:8b" },
  });

  await fireEvent.click(screen.getByRole("button", { name: "Save and test" }));

  expect((await screen.findByRole("status")).textContent).toContain("Testing qwen3:8b");
  expect(screen.getByRole<HTMLButtonElement>("button", { name: "Testing..." }).disabled).toBe(true);
});

it("saves the semantic communication language without invalidating the provider", async () => {
  render(Page);
  await fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));
  const language = await screen.findByRole<HTMLSelectElement>("combobox", {
    name: /AI communication language/,
  });
  expect(language.value).toBe("en");
  invokeMock.mockClear();

  await fireEvent.change(language, { target: { value: "es" } });

  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("save_semantic_preferences", {
      preferences: { language: "es" },
    });
  });
  expect(invokeMock).not.toHaveBeenCalledWith("save_provider_settings", expect.anything());
  expect(await screen.findByText("Communication language saved.")).not.toBeNull();
});

it("allows authentication for a secured custom OpenAI-compatible endpoint", async () => {
  render(Page);
  await fireEvent.click(await screen.findByRole("button", { name: "Open settings" }));
  await fireEvent.change(
    await screen.findByRole("combobox", { name: "Provider profile" }),
    { target: { value: "open_ai_compatible" } },
  );

  const requiresKey = screen.getByRole<HTMLInputElement>("checkbox", {
    name: "Endpoint requires an API key",
  });
  await fireEvent.click(requiresKey);
  expect(screen.getByLabelText("API key")).not.toBeNull();
});

it("hides entry-only tools while supporting contextual programmatic opening", async () => {
  const registration: { open: ((trigger: HTMLButtonElement) => void) | null } = { open: null };
  const trigger = document.createElement("button");
  document.body.append(trigger);
  render(EntryTools, {
    visible: false,
    appVersion: "1.2.3",
    shellSupported: true,
    shellInstalled: false,
    shellBusy: false,
    shellMessage: "",
    onToggleShell: vi.fn(),
    onregistersettings: (open) => {
      registration.open = open;
    },
  });

  await waitFor(() => expect(registration.open).not.toBeNull());
  expect(screen.queryByRole("button", { name: "Open settings" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Open help and about" })).toBeNull();
  registration.open?.(trigger);

  expect(await screen.findByRole("dialog", { name: "Settings" })).not.toBeNull();
  trigger.remove();
});
