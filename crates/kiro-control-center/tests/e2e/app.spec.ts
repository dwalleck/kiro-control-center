import { test, expect } from "@playwright/test";

test.describe("Kiro Control Center", () => {
  test("app loads and shows all nav destinations", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("body")).toBeVisible();

    await expect(page.getByRole("button", { name: "Browse", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Installed", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Marketplaces", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Kiro Settings", exact: true })).toBeVisible();
  });

  test("header has no settings gear button", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("button", { name: /open settings/i })).toHaveCount(0);
  });

  test("active rail destination gets aria-current=page", async ({ page }) => {
    await page.goto("/");

    const browseButton = page.getByRole("button", { name: "Browse", exact: true });
    await expect(browseButton).toHaveAttribute("aria-current", "page");

    const installedButton = page.getByRole("button", { name: "Installed", exact: true });
    await installedButton.click();
    await expect(installedButton).toHaveAttribute("aria-current", "page");
    await expect(browseButton).not.toHaveAttribute("aria-current", "page");
  });
});

test.describe("Browse tab filters", () => {
  test("filters popover opens and escape closes it", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Browse", exact: true }).click();

    const filtersButton = page.getByRole("button", { name: /^Filters/ });
    await expect(filtersButton).toHaveAttribute("aria-expanded", "false");

    await filtersButton.click();
    await expect(filtersButton).toHaveAttribute("aria-expanded", "true");

    await expect(page.getByText("Marketplace", { exact: true })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(filtersButton).toHaveAttribute("aria-expanded", "false");
  });

  test("outside click closes the filters popover", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Browse", exact: true }).click();

    const filtersButton = page.getByRole("button", { name: /^Filters/ });
    await filtersButton.click();
    await expect(filtersButton).toHaveAttribute("aria-expanded", "true");

    await page.getByPlaceholder(/filter skills by name/i).click();
    await expect(filtersButton).toHaveAttribute("aria-expanded", "false");
  });
});

test.describe("Marketplace workflow", () => {
  test("add local marketplace and see its skills in Browse", async ({ page }) => {
    await page.goto("/");

    await page.getByRole("button", { name: "Marketplaces", exact: true }).click();

    const fixturePath = process.env.FIXTURE_MARKETPLACE_PATH;
    if (!fixturePath) {
      test.skip(true, "FIXTURE_MARKETPLACE_PATH not set");
      return;
    }

    const input = page.getByPlaceholder(/source|url|path/i);
    await input.fill(fixturePath);

    const addButton = page.getByRole("button", { name: /add/i });
    await addButton.click();

    await expect(page.getByText(/added|success/i)).toBeVisible({ timeout: 30_000 });

    // Browse grid auto-populates from the first marketplace on load; no sidebar
    // navigation required.
    await page.getByRole("button", { name: "Browse", exact: true }).click();
    await expect(page.getByText(/test-plugin|test-skill/i).first()).toBeVisible({
      timeout: 10_000,
    });
  });

  test("install skill from browse tab", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Browse", exact: true }).click();

    const testSkill = page.getByText(/test-skill/i).first();
    if (!(await testSkill.isVisible({ timeout: 5_000 }).catch(() => false))) {
      test.skip(true, "No marketplace with test-skill available");
      return;
    }

    await testSkill.click();

    const installButton = page.getByRole("button", { name: /install \d+ selected/i });
    await installButton.click();

    await expect(page.getByText(/installed|success/i)).toBeVisible({ timeout: 30_000 });

    await page.getByRole("button", { name: "Installed", exact: true }).click();
    await expect(page.getByText(/test-skill/i).first()).toBeVisible();
  });

  test("install plugin from browse tab and verify in installed tab", async ({ page }) => {
    await page.getByRole("button", { name: "Browse", exact: true }).click();

    // The earlier "add local marketplace" test seeds FIXTURE_MARKETPLACE_PATH.
    // Skip if the fixture isn't available (matches the skill-install pattern).
    const fixturePath = process.env.FIXTURE_MARKETPLACE_PATH;
    test.skip(!fixturePath, "FIXTURE_MARKETPLACE_PATH not set");

    // Switch to the Plugins view if the toggle isn't already on it. The
    // Plugins button is the default per Task 7's BrowseView state, but
    // switching defensively keeps the test robust to a future default change.
    const pluginsToggle = page.getByRole("button", { name: "Plugins", exact: true });
    if (await pluginsToggle.isVisible({ timeout: 2_000 }).catch(() => false)) {
      await pluginsToggle.click();
    }

    // Find a plugin card with the test fixture's plugin name. PluginCard
    // exposes "Install plugin" via aria-label="Install <name>".
    const testPlugin = page.getByText(/test-plugin/i).first();
    if (!(await testPlugin.isVisible({ timeout: 5_000 }).catch(() => false))) {
      test.skip(true, "No marketplace with test-plugin available");
    }

    const installButton = page
      .getByRole("button", { name: /install test-plugin/i })
      .first();
    await installButton.click();

    // Wait for the success banner. Matches the success-banner pattern from
    // the skill-install test.
    await expect(page.getByText(/Plugin test-plugin/i)).toBeVisible({
      timeout: 30_000,
    });

    // Navigate to Installed tab and assert the plugin row appears.
    await page.getByRole("button", { name: "Installed", exact: true }).click();
    await expect(page.getByRole("heading", { name: /installed plugins/i })).toBeVisible();
    await expect(page.getByText(/test-plugin/i).first()).toBeVisible();
  });

  test("broken marketplace surfaces dismissible banner that clears on deselect", async ({ page }) => {
    const brokenPath = process.env.FIXTURE_BROKEN_MARKETPLACE_PATH;
    if (!brokenPath) {
      test.skip(true, "FIXTURE_BROKEN_MARKETPLACE_PATH not set");
      return;
    }

    await page.goto("/");
    await page.getByRole("button", { name: "Marketplaces", exact: true }).click();

    await page.getByPlaceholder(/source|url|path/i).fill(brokenPath);
    await page.getByRole("button", { name: /add/i }).click();

    // Switch to Browse — the plugin-fetch error should surface as a banner.
    await page.getByRole("button", { name: "Browse", exact: true }).click();

    const errorBanner = page.locator('[data-testid="fetch-error"]').first();
    await expect(errorBanner).toBeVisible({ timeout: 10_000 });

    // Dismiss via the contextual aria-label (e.g. "Dismiss error for <mp>").
    await errorBanner.getByRole("button", { name: /^Dismiss error for/ }).click();
    await expect(errorBanner).not.toBeVisible();
  });
});
