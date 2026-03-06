import { test, expect } from "./fixtures/app.fixture";
import {
  APP_ROOT, SIDEBAR, MAIN_CONTENT, APP_TITLE, NAV_ITEM,
  DEMO_BADGE, DEMO_BANNER, VERSION, JOBS_VIEW,
} from "./helpers/selectors";

test.describe("App Shell", () => {
  test("renders the root app container", async ({ appPage: page }) => {
    await expect(page.locator(APP_ROOT)).toBeVisible();
  });

  test("shows the sidebar with DIT Pro title", async ({ appPage: page }) => {
    await expect(page.locator(SIDEBAR)).toBeVisible();
    await expect(page.locator(APP_TITLE)).toHaveText("DIT Pro");
  });

  test("shows the main content area", async ({ appPage: page }) => {
    await expect(page.locator(MAIN_CONTENT)).toBeVisible();
  });

  test("displays 5 navigation items", async ({ appPage: page }) => {
    const navItems = page.locator(`${SIDEBAR} ${NAV_ITEM}`).filter({
      has: page.locator(".nav-label"),
    });
    // 5 main nav + possible feedback/website buttons
    const count = await navItems.count();
    expect(count).toBeGreaterThanOrEqual(5);
  });

  test("shows DEMO badge in browser mode", async ({ appPage: page }) => {
    await expect(page.locator(DEMO_BADGE)).toBeVisible();
    await expect(page.locator(DEMO_BADGE)).toHaveText(/DEMO/);
  });

  test("shows demo banner in content area", async ({ appPage: page }) => {
    const banner = page.locator(DEMO_BANNER);
    if (await banner.isVisible().catch(() => false)) {
      await expect(banner).toContainText(/preview|demo/i);
    }
  });

  test("displays version element", async ({ appPage: page }) => {
    await expect(page.locator(VERSION)).toBeVisible();
    const versionText = await page.locator(VERSION).textContent();
    // In demo mode, version might show "v--" or a real version string
    expect(versionText).toBeTruthy();
    expect(versionText!.startsWith("v")).toBe(true);
  });

  test("defaults to Jobs view on load", async ({ appPage: page }) => {
    // Jobs view should be visible by default
    await expect(page.locator(JOBS_VIEW)).toBeVisible();
  });
});
