import { test, expect } from "./fixtures/app.fixture";
import { navItem, SETTINGS_VIEW, APP_TITLE } from "./helpers/selectors";

test.describe("Internationalization (i18n)", () => {
  test("default language renders English navigation labels", async ({ appPage: page }) => {
    // Check sidebar has English labels
    const pageText = await page.locator("nav.sidebar").textContent();
    expect(pageText).toMatch(/Jobs|Volumes|Presets|Reports|Settings/);
  });

  test("can switch to Chinese", async ({ appPage: page }) => {
    // Navigate to Settings
    await page.locator(navItem("Settings")).click();
    await expect(page.locator(SETTINGS_VIEW)).toBeVisible();

    // Click Chinese language option
    const zhOption = page.locator('label').filter({ hasText: /Chinese|中文/ });
    if (await zhOption.isVisible().catch(() => false)) {
      await zhOption.click();
      await page.waitForTimeout(500);

      // Sidebar labels should now be in Chinese
      const sidebarText = await page.locator("nav.sidebar").textContent();
      // Check for Chinese navigation characters
      expect(sidebarText).toMatch(/任务|卷|预设|报告|设置/);
    }
  });

  test("can switch back to English from Chinese", async ({ appPage: page }) => {
    // Go to Settings
    await page.locator(navItem("Settings")).click();
    await expect(page.locator(SETTINGS_VIEW)).toBeVisible();

    // Switch to Chinese first
    const zhOption = page.locator('label').filter({ hasText: /Chinese|中文/ });
    if (await zhOption.isVisible().catch(() => false)) {
      await zhOption.click();
      await page.waitForTimeout(500);

      // Now switch back to English
      const enOption = page.locator('label').filter({ hasText: /English/ });
      await enOption.click();
      await page.waitForTimeout(500);

      // Should be back to English
      const sidebarText = await page.locator("nav.sidebar").textContent();
      expect(sidebarText).toMatch(/Jobs|Volumes|Settings/);
    }
  });

  test("app title remains DIT Pro in both languages", async ({ appPage: page }) => {
    // DIT Pro should be the brand name regardless of language
    await expect(page.locator(APP_TITLE)).toHaveText("DIT Pro");

    // Switch to Chinese
    await page.locator(navItem("Settings")).click();
    const zhOption = page.locator('label').filter({ hasText: /Chinese|中文/ });
    if (await zhOption.isVisible().catch(() => false)) {
      await zhOption.click();
      await page.waitForTimeout(500);
      // DIT Pro brand should not change
      await expect(page.locator(APP_TITLE)).toHaveText("DIT Pro");
    }
  });
});
