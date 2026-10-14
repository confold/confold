import { test, expect } from "@playwright/test";

// The launcher must fit the default 800×600 window even with worst-case recents: long SFTP/Nuxeo
// slugs must ellipsize inside their chips instead of widening the columns off-screen (owner report:
// fresh boot clipped "Select destination" past the window edge).
const LONG_ORIGIN_RECENTS = [
  {
    spec: {
      kind: "nuxeo",
      fields: {
        url: "https://nuxeo.example.com/nuxeo",
        root: "/Acme/workspaces/Presupuestos",
        username: "demo.user",
        password: "x",
      },
    },
    isDir: true,
  },
  {
    spec: {
      kind: "fs",
      fields: { root: "/srv/demo/workspace/DESKTOPS/NOTES/long-vault-name-for-overflow" },
    },
    isDir: true,
  },
  { spec: { kind: "fs", fields: { root: "/tmp/origin" } }, isDir: true },
];
const LONG_DEST_RECENTS = [
  {
    spec: {
      kind: "fs",
      fields: { root: "/srv/demo/Downloads/Very Long Folder Name - EXAMPLE 2024" },
    },
    isDir: true,
  },
  { spec: { kind: "fs", fields: { root: "/tmp/destination" } }, isDir: true },
];

test("launcher fits the 800×600 default window with long recents", async ({ page }) => {
  await page.addInitScript(
    ([origin, dest]) => {
      localStorage.setItem("confold-origin-recents", JSON.stringify(origin));
      localStorage.setItem("confold-dest-recents", JSON.stringify(dest));
    },
    [LONG_ORIGIN_RECENTS, LONG_DEST_RECENTS],
  );
  await page.setViewportSize({ width: 800, height: 600 });
  await page.goto("/");
  await page.waitForLoadState("networkidle");
  await expect(page.getByRole("button", { name: "+ Select source" })).toBeVisible();
  await expect(page.getByRole("button", { name: "+ Select destination" })).toBeVisible();

  const overflow = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    innerWidth: window.innerWidth,
  }));
  expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.innerWidth);

  for (const name of ["+ Select source", "+ Select destination"]) {
    const box = await page.getByRole("button", { name }).boundingBox();
    expect(box).toBeTruthy();
    expect((box?.x ?? 0)).toBeGreaterThanOrEqual(0);
    expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(800);
  }
  await page.screenshot({ path: "test-results/launcher-800x600.png", fullPage: true });
});
