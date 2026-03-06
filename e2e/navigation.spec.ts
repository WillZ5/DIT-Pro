import { test, expect } from "./fixtures/app.fixture";
import {
  navItem, navItemActive, JOBS_VIEW, VOLUME_VIEW,
  PRESETS_VIEW, NAV_ITEM_ACTIVE, SETTINGS_SECTION,
} from "./helpers/selectors";

test.describe("Navigation", () => {
  test("Jobs is the default active view", async ({ appPage: page }) => {
    await expect(page.locator(JOBS_VIEW)).toBeVisible();
  });

  test("can navigate to Volumes view", async ({ appPage: page }) => {
    await page.locator(navItem("Volumes")).click();
    await expect(page.locator(navItemActive("Volumes"))).toBeVisible();
    await expect(page.locator(VOLUME_VIEW)).toBeVisible();
  });

  test("can navigate to Presets view", async ({ appPage: page }) => {
    await page.locator(navItem("Presets")).click();
    await expect(page.locator(navItemActive("Presets"))).toBeVisible();
    // Presets layout loads after async data fetch
    await expect(page.locator(PRESETS_VIEW)).toBeVisible({ timeout: 5_000 });
  });

  test("can navigate to Reports view", async ({ appPage: page }) => {
    await page.locator(navItem("Reports")).click();
    await expect(page.locator(navItemActive("Reports"))).toBeVisible();
  });

  test("can navigate to Settings view", async ({ appPage: page }) => {
    await page.locator(navItem("Settings")).click();
    await expect(page.locator(navItemActive("Settings"))).toBeVisible();
    // Settings sections load after async fetch
    await expect(page.locator(SETTINGS_SECTION).first()).toBeVisible({ timeout: 5_000 });
  });

  test("only one nav item is active at a time", async ({ appPage: page }) => {
    const views = ["Jobs", "Volumes", "Presets", "Reports", "Settings"];
    for (const view of views) {
      await page.locator(navItem(view)).click();
      await page.waitForTimeout(200);
      const activeCount = await page.locator(NAV_ITEM_ACTIVE).count();
      expect(activeCount).toBe(1);
      await expect(page.locator(navItemActive(view))).toBeVisible();
    }
  });

  test("can navigate back to Jobs view", async ({ appPage: page }) => {
    await page.locator(navItem("Settings")).click();
    await expect(page.locator(navItemActive("Settings"))).toBeVisible();

    await page.locator(navItem("Jobs")).click();
    await expect(page.locator(JOBS_VIEW)).toBeVisible();
    await expect(page.locator(navItemActive("Jobs"))).toBeVisible();
  });
});
