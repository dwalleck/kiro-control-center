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
