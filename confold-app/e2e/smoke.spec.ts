import { test, expect } from "@playwright/test";

// Visual smoke through the real frontend in Chromium (Tauri shimmed). Proves the app boots, runs a
// compare, renders the tree, and opens a file into the side-by-side frame — and captures screenshots
// for the developer to eyeball (layout / overlap / z-index, which jsdom can't verify).
const ORIGIN = "/tmp/cdemo/left";
const DEST = "/tmp/cdemo/right";

test.beforeEach(async ({ page }) => {
  // Seed the source recents so we can pick both sides with one click each (no picker form needed).
  await page.addInitScript(
    ([origin, dest]) => {
      localStorage.setItem("confold-origin-recents", JSON.stringify([{ spec: { type: "fs", root: origin }, isDir: true }]));
      localStorage.setItem("confold-dest-recents", JSON.stringify([{ spec: { type: "fs", root: dest }, isDir: true }]));
    },
    [ORIGIN, DEST],
  );
});

test("boot → compare → tree → open file", async ({ page }) => {
  await page.goto("/");
  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();

  // The compare tree renders with our mixed fixture.
  await expect(page.getByText("readme.md")).toBeVisible();
  await page.screenshot({ path: "test-results/compare-tree.png", fullPage: true });

  // Clicking a differing file opens the side-by-side frame (loading skeleton, with its back control).
  await page.getByText("readme.md").click();
  await expect(page.getByLabel("back")).toBeVisible();
  await page.screenshot({ path: "test-results/file-open.png", fullPage: true });
});

test("large text file → warning → hunks view renders content", async ({ page }) => {
  await page.goto("/");
  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();
  await page.getByText("big.log").click();

  // Warning dialog first.
  await expect(page.getByText("Large file")).toBeVisible();
  await page.screenshot({ path: "test-results/large-warning.png", fullPage: true });

  // Confirm → the hunks view must actually render content (not a black/empty window).
  await page.getByText("Show differences").click();
  await expect(page.getByText("new 1", { exact: true }).first()).toBeVisible();
  await page.screenshot({ path: "test-results/large-hunks.png", fullPage: true });
});

test("file-view header is consistent across text and binary", async ({ page }) => {
  await page.goto("/");
  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();

  // Text view header.
  await page.getByText("readme.md").click();
  await expect(page.getByLabel("back")).toBeVisible();
  await page.locator(".fvh").waitFor();
  await page.locator(".fvh").screenshot({ path: "test-results/header-text.png" });
  await page.getByLabel("back").click();

  // Binary view header — same shared component (.fvh).
  await page.getByText("blob.bin").click();
  await page.locator(".fvh").waitFor();
  await page.locator(".fvh").screenshot({ path: "test-results/header-binary.png" });
});

test("binary file → hex view with difference navigation", async ({ page }) => {
  await page.goto("/");
  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();
  await page.getByText("blob.bin").click();

  // The hex view renders with the difference navigation — 3 distinct byte regions (one per row), so
  // an all-rows-differ file is NOT collapsed to a single region.
  await expect(page.getByText("Binary — they differ")).toBeVisible();
  await expect(page.getByText("1 / 3 diffs")).toBeVisible();
  await expect(page.getByLabel("next difference")).toBeVisible();
  await page.screenshot({ path: "test-results/hex-view.png", fullPage: true });

  // Step through the regions.
  await page.getByLabel("next difference").click();
  await expect(page.getByText("2 / 3 diffs")).toBeVisible();
  await page.getByLabel("next difference").click();
  await expect(page.getByText("3 / 3 diffs")).toBeVisible();
  await page.screenshot({ path: "test-results/hex-view-nav.png", fullPage: true });
});
