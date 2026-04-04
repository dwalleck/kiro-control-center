import { test, expect } from "@playwright/test";

test.describe("Kiro Control Center", () => {
  test("app loads and shows all tabs", async ({ page }) => {
    await page.goto("/");

    // Wait for the app to mount.
    await expect(page.locator("body")).toBeVisible();

    // Verify all three tabs are present.
    await expect(page.getByRole("tab", { name: /browse/i })).toBeVisible();
    await expect(page.getByRole("tab", { name: /installed/i })).toBeVisible();
    await expect(
      page.getByRole("tab", { name: /marketplace/i })
    ).toBeVisible();
  });
});

test.describe("Marketplace workflow", () => {
  test("add local marketplace and browse plugins", async ({ page }) => {
    await page.goto("/");

    // Navigate to Marketplaces tab.
    await page.getByRole("tab", { name: /marketplace/i }).click();

    // Add a marketplace using a local path.
    // NOTE: This test requires a pre-built marketplace fixture.
    // For CI, set FIXTURE_MARKETPLACE_PATH env var.
    const fixturePath = process.env.FIXTURE_MARKETPLACE_PATH;
    if (!fixturePath) {
      test.skip(true, "FIXTURE_MARKETPLACE_PATH not set");
      return;
    }

    const input = page.getByPlaceholder(/source|url|path/i);
    await input.fill(fixturePath);

    const addButton = page.getByRole("button", { name: /add/i });
    await addButton.click();

    // Wait for success feedback.
    await expect(page.getByText(/added|success/i)).toBeVisible({
      timeout: 30_000,
    });

    // Switch to Browse tab and verify plugin appears.
    await page.getByRole("tab", { name: /browse/i }).click();
    await expect(page.getByText(/test-plugin/i)).toBeVisible();
  });

  test("install skill from browse tab", async ({ page }) => {
    await page.goto("/");

    // This test assumes a marketplace is already added (from previous test
    // or fixture setup). If no marketplace exists, skip.
    await page.getByRole("tab", { name: /browse/i }).click();

    const pluginLink = page.getByText(/test-plugin/i);
    if (!(await pluginLink.isVisible({ timeout: 5_000 }).catch(() => false))) {
      test.skip(true, "No marketplace with test-plugin available");
      return;
    }

    await pluginLink.click();

    // Find and click install on a skill.
    const installButton = page.getByRole("button", { name: /install/i });
    await installButton.first().click();

    // Verify success.
    await expect(page.getByText(/installed|success/i)).toBeVisible({
      timeout: 30_000,
    });

    // Switch to Installed tab and verify skill appears.
    await page.getByRole("tab", { name: /installed/i }).click();
    await expect(page.getByText(/test-skill/i)).toBeVisible();
  });
});
