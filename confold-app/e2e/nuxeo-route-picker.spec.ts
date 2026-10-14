import { test, expect } from "@playwright/test";

test("Nuxeo source picker browses paginated folders and searches repository paths", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.emulateMedia({ reducedMotion: "reduce", colorScheme: "dark" });
  await page.goto("/");

  await page.getByRole("button", { name: "Select source" }).click();
  await page.getByRole("button", { name: "Nuxeo" }).click();
  await page.getByLabel(/Server URL/).fill("https://nuxeo.example/nuxeo");
  await page.getByLabel("Username").fill("reader");
  await page.getByRole("textbox", { name: /Password/ }).fill("test-password");
  // Connection checks are user-invoked: nothing is probed until "Verify connection" is pressed.
  await expect(page.getByText("Testing connection…")).toHaveCount(0);
  await page.getByRole("button", { name: "Verify connection" }).click();
  await expect(page.getByText(/Connected/)).toBeVisible();
  await expect(page.locator(".type.on img")).toHaveAttribute("src", "/nuxeo-logo.svg");
  await page.screenshot({ path: "test-results/nuxeo-logo-picker-dark.png", fullPage: true, animations: "disabled" });

  await page.getByRole("button", { name: "Browse routes" }).click();
  await expect(page.getByRole("region", { name: "Choose a Nuxeo document path" })).toBeVisible();
  await page.screenshot({ path: "test-results/nuxeo-route-browser-wide.png", fullPage: true, animations: "disabled" });

  await page.getByRole("button", { name: /Default domain/ }).click();
  await expect(page.getByRole("button", { name: /Workspaces/ })).toBeVisible();
  await page.getByRole("button", { name: /Workspaces/ }).click();
  await expect(page.getByText("Contracts", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Load more folders" }).click();
  await expect(page.getByText("Archive", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Use this folder" }).last().click();
  await expect(page.getByLabel("Document path")).toHaveValue("/default-domain/workspaces");
  await expect(page.getByRole("button", { name: "Browse routes" })).toBeFocused();

  await page.getByRole("button", { name: "Browse routes" }).click();
  await page.getByRole("searchbox", { name: "Search folder names or absolute path prefixes" }).fill("signed");
  const searchResult = page.getByRole("button", { name: /Signed contracts/ });
  await expect(searchResult).toBeVisible();
  await page.screenshot({ path: "test-results/nuxeo-route-search-results.png", fullPage: true, animations: "disabled" });
  await searchResult.click();
  await expect(page.getByLabel("Document path")).toHaveValue("/default-domain/workspaces/signed-contracts");

  await page.setViewportSize({ width: 375, height: 720 });
  await page.getByRole("button", { name: "Browse routes" }).click();
  await expect(page.getByRole("region", { name: "Choose a Nuxeo document path" })).toBeVisible();
  await expect(page.getByRole("searchbox", { name: "Search folder names or absolute path prefixes" })).toBeVisible();
  const hasHorizontalOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  expect(hasHorizontalOverflow).toBe(false);
  await page.screenshot({ path: "test-results/nuxeo-route-browser-narrow.png", fullPage: true, animations: "disabled" });
  await page.getByRole("button", { name: "Back to source settings" }).click();
  await expect(page.getByRole("button", { name: "Browse routes" })).toBeFocused();
  const selectSource = page.getByRole("button", { name: "Select source", exact: true });
  await expect(selectSource).toBeEnabled();
  await selectSource.click();
  await expect(page.locator(".chosen img[src='/nuxeo-logo.svg']")).toBeVisible();
  await expect(page.locator(".chosen")).toHaveAttribute(
    "title",
    "nuxeo.example/nuxeo:/default-domain/workspaces/signed-contracts (reader)",
  );
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: "test-results/nuxeo-selected-source-narrow.png", fullPage: true, animations: "disabled" });
});
