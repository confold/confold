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
});

test("entry tools open accessible Settings and Help dialogs", async ({ page }) => {
  test.setTimeout(60_000);
  const browserErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });

  await page.setViewportSize({ width: 1280, height: 800 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
  await expect(page.getByLabel("Folder right-click integration")).toBeVisible();
  await expect(page.getByLabel("Provider profile")).toHaveValue("ollama");
  await expect(page.getByLabel("AI communication language")).toHaveValue("en");
  await page.getByLabel("AI communication language").selectOption("es");
  await expect(page.getByText("Communication language saved.")).toBeVisible();
  await page.getByRole("button", { name: "Discover models" }).click();
  const models = page.getByLabel("Model");
  await expect(models.locator("option")).toHaveCount(6);
  await models.selectOption("qwen2.5-coder:7b");
  await page.getByRole("button", { name: "Save and test" }).click();
  await expect(page.getByRole("status")).toContainText("Testing qwen2.5-coder:7b");
  await page.screenshot({
    path: "test-results/provider-testing-progress-wide.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 375, height: 720 });
  await expect(page.getByRole("status")).toContainText("Testing qwen2.5-coder:7b");
  await page.screenshot({
    path: "test-results/provider-testing-progress-narrow.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(page.getByText("Structured output verified with qwen2.5-coder:7b.")).toBeVisible();
  await page.screenshot({
    path: "test-results/final-provider-wide.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.getByLabel("Provider profile").selectOption("open_ai_compatible");
  await page.getByLabel("Endpoint requires an API key").check();
  await page.getByLabel("API key", { exact: true }).fill("visual-test-key");
  await page.getByText("Advanced headers").click();
  await page.getByLabel("Custom headers").fill("X-Provider-Route: local");
  await page.screenshot({
    path: "test-results/final-provider-custom-wide.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.locator(".modal-body").evaluate((body) => {
    body.scrollTop = body.scrollHeight;
  });
  await page.screenshot({
    path: "test-results/final-provider-custom-wide-bottom.png",
    fullPage: true,
    animations: "disabled",
  });

  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(page.getByRole("button", { name: "Open settings" })).toBeFocused();

  await page.setViewportSize({ width: 800, height: 600 });
  await page.getByRole("button", { name: "Open help and about" }).click();
  await expect(page.getByRole("dialog", { name: "Help and About" })).toBeVisible();
  await page.screenshot({
    path: "test-results/entry-help-default.png",
    fullPage: true,
    animations: "disabled",
  });

  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 640, height: 480 });
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
  const compactTitleY = (await page.getByRole("heading", { name: "Settings" }).boundingBox())?.y;
  await page.screenshot({
    path: "test-results/final-provider-compact.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.locator(".modal-body").evaluate((body) => {
    body.scrollTop = body.scrollHeight;
  });
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close Settings" })).toBeVisible();
  expect((await page.getByRole("heading", { name: "Settings" }).boundingBox())?.y).toBeCloseTo(
    compactTitleY ?? 0,
    0,
  );
  await page.screenshot({
    path: "test-results/final-provider-compact-bottom.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.locator(".modal-panel").screenshot({
    path: "test-results/final-provider-compact-panel-bottom.png",
    animations: "disabled",
  });

  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 390, height: 780 });
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
  const narrowTitleY = (await page.getByRole("heading", { name: "Settings" }).boundingBox())?.y;
  await page.screenshot({
    path: "test-results/final-provider-narrow.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.locator(".modal-body").evaluate((body) => {
    body.scrollTop = body.scrollHeight;
  });
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close Settings" })).toBeVisible();
  expect((await page.getByRole("heading", { name: "Settings" }).boundingBox())?.y).toBeCloseTo(
    narrowTitleY ?? 0,
    0,
  );
  await page.screenshot({
    path: "test-results/final-provider-narrow-bottom.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.locator(".modal-panel").screenshot({
    path: "test-results/final-provider-narrow-panel-bottom.png",
    animations: "disabled",
  });
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Open help and about" }).click();
  await expect(page.getByRole("dialog", { name: "Help and About" })).toBeVisible();
  await page.screenshot({
    path: "test-results/entry-help-narrow.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.keyboard.press("Escape");
  await page.emulateMedia({ colorScheme: "dark" });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
  await page.getByRole("button", { name: "Save and test" }).click();
  await expect(page.getByText(/Structured output verified/)).toBeVisible();
  await page.screenshot({
    path: "test-results/final-provider-dark.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.getByLabel("Model").fill("reject-structured");
  await expect(page.getByText(/Structured output verified/)).toBeHidden();
  await page.getByRole("button", { name: "Save and test" }).click();
  await expect(page.getByRole("alert")).toContainText("required JSON schema");
  await page.screenshot({
    path: "test-results/final-provider-dark-error.png",
    fullPage: true,
    animations: "disabled",
  });

  expect(browserErrors).toEqual([]);
});

test("boot → compare → tree → open file", async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto("/");
  await page.waitForLoadState("networkidle");
  await expect(page.getByRole("button", { name: "Open settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open help and about" })).toBeVisible();
  await page.screenshot({
    path: "test-results/entry-tools-initial.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();
  const folderToolbar = page.locator(".compact-bar");
  await expect(folderToolbar.getByRole("button", { name: "Open settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open help and about" })).toBeHidden();
  const folderBack = folderToolbar.getByRole("button", { name: "Back to source selection" });
  await expect(folderBack).toBeVisible();
  await folderBack.click();
  await expect(page.getByRole("heading", { name: "Back to source selection?" })).toBeVisible();
  await page.getByRole("button", { name: "Stay here" }).click();

  // The compare tree renders with our mixed fixture.
  await expect(page.getByText("readme.md")).toBeVisible();
  await page.screenshot({ path: "test-results/compare-tree.png", fullPage: true, animations: "disabled" });

  await page.setViewportSize({ width: 1039, height: 648 });
  const compactSources = folderToolbar.locator(".mini-src");
  const compactOrigin = compactSources.nth(0);
  const compactDestination = compactSources.nth(1);
  const compactLogo = folderToolbar.locator(".compact-logo");
  const compactBack = folderToolbar.locator(".mini-back");
  const compactArrow = folderToolbar.locator(".arrow");
  const compactRefresh = folderToolbar.getByTitle("Re-scan comparison");
  const compactSettings = folderToolbar.getByRole("button", { name: "Open settings" });
  const compactOptions = folderToolbar.locator(".mini-opts");
  await compactOrigin.click();
  await expect(page.getByRole("heading", { name: "Back to source selection?" })).toBeVisible();
  await page.getByRole("button", { name: "Stay here" }).click();
  await compactDestination.click();
  await expect(page.getByRole("heading", { name: "Back to source selection?" })).toBeVisible();
  await page.getByRole("button", { name: "Stay here" }).click();
  const [toolbarBox, logoBox, backBox, originBox, arrowBox, destinationBox, refreshBox, compactSettingsBox, optionsBox] = await Promise.all([
    folderToolbar.boundingBox(),
    compactLogo.boundingBox(),
    compactBack.boundingBox(),
    compactOrigin.boundingBox(),
    compactArrow.boundingBox(),
    compactDestination.boundingBox(),
    compactRefresh.boundingBox(),
    compactSettings.boundingBox(),
    compactOptions.boundingBox(),
  ]);
  const horizontalGap = (
    left: { x: number; width: number } | null,
    right: { x: number } | null,
  ) => (right?.x ?? 0) - ((left?.x ?? 0) + (left?.width ?? 0));
  for (const gap of [
    horizontalGap(logoBox, backBox),
    horizontalGap(backBox, originBox),
    horizontalGap(originBox, arrowBox),
    horizontalGap(arrowBox, destinationBox),
    horizontalGap(destinationBox, refreshBox),
  ]) {
    expect(gap).toBeGreaterThanOrEqual(0);
    expect(gap).toBeLessThan(24);
  }
  const destinationCenterY = (destinationBox?.y ?? 0) + (destinationBox?.height ?? 0) / 2;
  for (const box of [logoBox, backBox, originBox, arrowBox, refreshBox, compactSettingsBox]) {
    expect((box?.y ?? 0) + (box?.height ?? 0) / 2).toBeCloseTo(destinationCenterY, 0);
  }
  expect(optionsBox?.y ?? 0).toBeGreaterThan((compactSettingsBox?.y ?? 0) + (compactSettingsBox?.height ?? 0));
  expect(
    Math.abs(
      ((toolbarBox?.x ?? 0) + (toolbarBox?.width ?? 0)) -
        ((compactSettingsBox?.x ?? 0) + (compactSettingsBox?.width ?? 0)),
    ),
  ).toBeLessThan(5);
  expect(
    Math.abs(
      ((toolbarBox?.x ?? 0) + (toolbarBox?.width ?? 0)) -
        ((optionsBox?.x ?? 0) + (optionsBox?.width ?? 0)),
    ),
  ).toBeLessThan(5);
  await page.screenshot({
    path: "test-results/compare-tree-compact.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({
    path: "test-results/compare-tree-compact-dark.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.emulateMedia({ colorScheme: "light" });
  await page.setViewportSize({ width: 1280, height: 720 });

  // Clicking a differing file opens the side-by-side frame (loading skeleton, with its back control).
  await page.getByText("readme.md").click();
  await expect(page.getByLabel("back")).toBeVisible();
  const settingsButton = page.getByRole("button", { name: "Open settings" });
  const semanticButton = page.getByRole("button", { name: "Run semantic review" });
  const fileHeader = page.locator(".fvh");
  const fileLogo = fileHeader.locator(".fvh-logo");
  const fileBack = fileHeader.getByRole("button", { name: "back" });
  await expect(fileLogo).toBeVisible();
  await expect(fileBack).toBeVisible();
  const [fileLogoBox, fileBackBox] = await Promise.all([fileLogo.boundingBox(), fileBack.boundingBox()]);
  expect(horizontalGap(fileLogoBox, fileBackBox)).toBeGreaterThanOrEqual(0);
  expect(horizontalGap(fileLogoBox, fileBackBox)).toBeLessThan(24);
  await expect(fileHeader.getByRole("button", { name: "Open settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open help and about" })).toBeHidden();
  await expect(settingsButton).toBeVisible();
  await expect(semanticButton).toBeVisible();
  const settingsBox = await settingsButton.boundingBox();
  const semanticBox = await semanticButton.boundingBox();
  expect(settingsBox?.x ?? 0).toBeGreaterThanOrEqual(
    (semanticBox?.x ?? 0) + (semanticBox?.width ?? 0),
  );
  await page.screenshot({ path: "test-results/file-open.png", fullPage: true, animations: "disabled" });

  await settingsButton.click();
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
  await page.screenshot({ path: "test-results/settings-from-file-view.png", fullPage: true, animations: "disabled" });
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeHidden();

  await page.setViewportSize({ width: 390, height: 780 });
  await expect(settingsButton).toBeVisible();
  await expect(semanticButton).toBeVisible();
  const narrowSettingsBox = await settingsButton.boundingBox();
  const narrowSemanticBox = await semanticButton.boundingBox();
  expect(narrowSettingsBox?.x ?? 0).toBeGreaterThanOrEqual(
    (narrowSemanticBox?.x ?? 0) + (narrowSemanticBox?.width ?? 0),
  );
  expect((narrowSettingsBox?.x ?? 0) + (narrowSettingsBox?.width ?? 0)).toBeLessThanOrEqual(390);
  await page.screenshot({ path: "test-results/file-open-narrow.png", fullPage: true, animations: "disabled" });
});

test("local text diff → validated semantic review can save safely and remains responsive", async ({
  page,
}) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByLabel("AI communication language").selectOption("es");
  await expect(page.getByText("Communication language saved.")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();

  const treeReviewButton = page.getByRole("button", { name: "Review readme.md with AI" });
  await expect(treeReviewButton).toBeVisible();
  await page.screenshot({
    path: "test-results/semantic-tree-action-wide.png",
    fullPage: true,
    animations: "disabled",
  });
  await treeReviewButton.click();

  const dialog = page.getByRole("dialog", { name: "Semantic review" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("status")).toContainText("Reviewing with qwen3:4b-instruct");
  await page.screenshot({
    path: "test-results/semantic-review-progress-wide.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 768, height: 720 });
  await expect(dialog.getByRole("status")).toBeVisible();
  await page.screenshot({
    path: "test-results/semantic-review-progress-medium.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 375, height: 720 });
  await expect(dialog.getByRole("status")).toBeVisible();
  await page.screenshot({
    path: "test-results/semantic-review-progress-narrow.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(dialog.getByText("Both documents contribute useful, non-conflicting details.")).toBeVisible();
  const configuration = dialog.getByRole("region", { name: "AI configuration used" });
  await expect(configuration.getByText("Ollama")).toBeVisible();
  await expect(configuration.getByText("qwen3:4b-instruct", { exact: true })).toHaveCount(2);
  await expect(configuration.getByText("Español")).toBeVisible();
  await expect(configuration.getByText("http://127.0.0.1:11434/v1")).toBeVisible();
  await expect(dialog.locator(".safety-note")).toContainText("no files have changed yet");
  await expect(dialog.getByRole("button", { name: "Save to origin" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Save to destination" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Save as…" })).toBeVisible();
  await page.screenshot({
    path: "test-results/semantic-tree-auto-wide.png",
    fullPage: true,
    animations: "disabled",
  });

  await page.setViewportSize({ width: 768, height: 720 });
  await expect(dialog).toBeVisible();
  await page.screenshot({
    path: "test-results/semantic-tree-auto-medium.png",
    fullPage: true,
    animations: "disabled",
  });

  await page.setViewportSize({ width: 375, height: 720 });
  const closeButton = dialog.getByRole("button", { name: "Close Semantic review" });
  const heading = dialog.getByRole("heading", { name: "Semantic review" });
  await expect(closeButton).toBeVisible();
  const headerY = (await heading.boundingBox())?.y;
  await page.screenshot({
    path: "test-results/semantic-tree-auto-narrow.png",
    fullPage: true,
    animations: "disabled",
  });
  await dialog.locator(".modal-body").evaluate((body) => {
    body.scrollTop = body.scrollHeight;
  });
  await expect(dialog.getByText("Deterministic diffs")).toBeVisible();
  await expect(heading).toBeVisible();
  await expect(closeButton).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Save as…" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Save to origin" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Save to destination" })).toBeVisible();
  for (const control of [
    heading,
    closeButton,
    dialog.getByRole("button", { name: "Save as…" }),
    dialog.getByRole("button", { name: "Save to origin" }),
    dialog.getByRole("button", { name: "Save to destination" }),
  ]) {
    const box = await control.boundingBox();
    expect(box?.y ?? -1).toBeGreaterThanOrEqual(0);
    expect((box?.y ?? 720) + (box?.height ?? 1)).toBeLessThanOrEqual(720);
  }
  expect((await heading.boundingBox())?.y).toBeCloseTo(headerY ?? 0, 0);
  await dialog.locator(".modal-panel").screenshot({
    path: "test-results/semantic-tree-auto-narrow-bottom-panel-r4.png",
    animations: "disabled",
  });
  await dialog.locator("header").screenshot({
    path: "test-results/semantic-tree-auto-header-retained-proof.png",
    animations: "disabled",
  });

  await dialog.getByText("left compared with the proposal").click();
  await expect(dialog.locator(".diff-line.addition").first()).toHaveCSS(
    "color",
    "rgb(23, 109, 99)",
  );
  await dialog.getByRole("button", { name: "Save to destination" }).click();
  await expect(dialog.getByText("/tmp/cdemo/right/readme.md")).toBeVisible();
  const replaceDestination = dialog.getByRole("button", { name: "Replace destination" });
  await replaceDestination.scrollIntoViewIfNeeded();
  await expect(replaceDestination).toBeVisible();
  await page.screenshot({
    path: "test-results/semantic-save-confirm-narrow.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 768, height: 720 });
  await page.screenshot({
    path: "test-results/semantic-save-confirm-medium.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.screenshot({
    path: "test-results/semantic-save-confirm-wide.png",
    fullPage: true,
    animations: "disabled",
  });
  await replaceDestination.click();
  await expect(dialog.getByRole("status")).toContainText(
    "Saved to /tmp/cdemo/right/readme.md",
  );
  await page.screenshot({
    path: "test-results/semantic-save-success-wide.png",
    fullPage: true,
    animations: "disabled",
  });
});

test("semantic tree action opens provider setup when AI review is unverified", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");

  await page.getByRole("button", { name: "Open settings" }).click();
  const settings = page.getByRole("dialog", { name: "Settings" });
  await settings.getByRole("button", { name: "Save", exact: true }).click();
  await expect(settings.getByText("Provider settings saved.")).toBeVisible();
  await page.keyboard.press("Escape");

  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();
  await page.getByRole("button", { name: "Review readme.md with AI" }).click();

  await expect(page.getByLabel("back")).toBeVisible();
  const setup = page.getByRole("dialog", { name: "AI provider required" });
  await expect(setup).toBeVisible();
  await expect(setup).toContainText("Configure and verify an AI provider");
  await page.screenshot({
    path: "test-results/semantic-provider-required-wide.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 375, height: 720 });
  await expect(setup.getByRole("button", { name: "Open Settings" })).toBeVisible();
  await page.screenshot({
    path: "test-results/semantic-provider-required-narrow.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  await setup.getByRole("button", { name: "Open Settings" }).click();
  await expect(settings).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Semantic review" })).toHaveCount(0);
  await page.screenshot({
    path: "test-results/semantic-tree-configure-wide.png",
    fullPage: true,
    animations: "disabled",
  });

  await page.setViewportSize({ width: 375, height: 720 });
  const settingsHeading = settings.getByRole("heading", { name: "Settings" });
  const settingsClose = settings.getByRole("button", { name: "Close Settings" });
  const settingsHeaderY = (await settingsHeading.boundingBox())?.y;
  await expect(settingsClose).toBeVisible();
  await page.screenshot({
    path: "test-results/semantic-tree-configure-narrow.png",
    fullPage: true,
    animations: "disabled",
  });
  await settings.locator(".modal-body").evaluate((body) => {
    body.scrollTop = body.scrollHeight;
  });
  await expect(settings.getByRole("button", { name: "Save", exact: true })).toBeVisible();
  await expect(settings.getByRole("button", { name: "Save and test" })).toBeVisible();
  await expect(settingsHeading).toBeVisible();
  await expect(settingsClose).toBeVisible();
  expect((await settingsHeading.boundingBox())?.y).toBeCloseTo(settingsHeaderY ?? 0, 0);
  await page.screenshot({
    path: "test-results/semantic-tree-configure-narrow-bottom.png",
    fullPage: true,
    animations: "disabled",
  });
  const settingsPanelBox = await settings.locator(".modal-panel").boundingBox();
  if (!settingsPanelBox) throw new Error("Settings panel is not visible");
  await page.screenshot({
    path: "test-results/semantic-tree-configure-header-retained-proof.png",
    animations: "disabled",
    clip: {
      x: settingsPanelBox.x,
      y: settingsPanelBox.y,
      width: settingsPanelBox.width,
      height: 70,
    },
  });
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

test("file-view header is consistent across text, binary, image, and loading views", async ({ page }) => {
  await page.goto("/");
  await page.getByTitle(ORIGIN).first().click();
  await page.getByTitle(DEST).first().click();
  await page.locator(".split-main").click();

  // Text view header.
  await page.getByText("readme.md").click();
  await expect(page.getByLabel("back")).toBeVisible();
  const textHeader = page.locator(".fvh");
  await textHeader.waitFor();
  await expect(textHeader.getByRole("img", { name: "Confold" })).toBeVisible();
  await textHeader.screenshot({ path: "test-results/header-text.png" });
  await page.getByLabel("back").click();

  // Binary view header — same shared component (.fvh).
  await page.getByText("blob.bin").click();
  const binaryHeader = page.locator(".fvh");
  await binaryHeader.waitFor();
  await expect(binaryHeader.getByRole("img", { name: "Confold" })).toBeVisible();
  await binaryHeader.screenshot({ path: "test-results/header-binary.png" });
  await binaryHeader.getByRole("button", { name: "back" }).click();

  await page.getByText("image.png").click();
  const imageHeader = page.locator(".fvh");
  await imageHeader.waitFor();
  await expect(imageHeader.getByRole("img", { name: "Confold" })).toBeVisible();
  await imageHeader.screenshot({ path: "test-results/header-image.png" });
  await imageHeader.getByRole("button", { name: "back" }).click();

  await page.getByText("pending.dat").click();
  const loadingHeader = page.locator(".fvh");
  await loadingHeader.waitFor();
  await expect(loadingHeader.getByRole("img", { name: "Confold" })).toBeVisible();
  await expect(page.locator(".sbs-panels .spinner")).toHaveCount(2);
  await loadingHeader.screenshot({ path: "test-results/header-loading.png" });
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
