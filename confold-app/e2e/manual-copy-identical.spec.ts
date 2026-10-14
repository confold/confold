import { test, expect } from "@playwright/test";

// The compare-view copy guard: files the comparison already shows as identical are offered for
// skipping (default ON) instead of being silently re-copied. Repro of the owner report: identical
// files were re-copied with no prompt.
const ORIGIN = "/tmp/cdemo/left";
const DEST = "/tmp/cdemo/right";

test("copying identical files asks first and skips them by default", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript(
    ([origin, dest]) => {
      localStorage.setItem(
        "confold-origin-recents",
        JSON.stringify([{ spec: { kind: "fs", fields: { root: origin } }, isDir: true }]),
      );
      localStorage.setItem(
        "confold-dest-recents",
        JSON.stringify([{ spec: { kind: "fs", fields: { root: dest } }, isDir: true }]),
      );
    },
    [ORIGIN, DEST],
  );
  await page.goto("/");
  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();
  await expect(page.getByText("readme.md")).toBeVisible();

  // One identical file + one different file.
  await page.getByLabel("select same.txt").check();
  await page.getByLabel("select readme.md").check();

  await page.getByRole("button", { name: "Copy →" }).click();

  // The guard asks, defaulting to skip; the plan shows only the non-identical copy.
  await expect(
    page.getByText("Skip the 1 file already identical (won't be re-copied)"),
  ).toBeVisible();
  await expect(page.locator(".planlist li")).toHaveCount(1);

  // Confirm: the backend batch must NOT contain the identical file.
  await page.getByRole("button", { name: "Confirm" }).click();
  const applied = (await page.evaluate(
    () => (globalThis as { __confoldApplied?: unknown[] }).__confoldApplied ?? [],
  )) as { rel_path: string[]; op: string }[][];
  const flat = applied.flat().map((a) => a.rel_path.join("/"));
  expect(flat).toEqual(["readme.md"]);
});
