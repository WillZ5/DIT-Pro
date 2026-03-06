import { test, expect } from "./fixtures/app.fixture";
import {
  navItem, SETTINGS_SECTION, IO_TABLE_ROW,
} from "./helpers/selectors";

test.describe("Settings View", () => {
  test.beforeEach(async ({ appPage: page }) => {
    await page.locator(navItem("Settings")).click();
    // Wait for settings sections to load (Settings uses .settings-view like other views)
    await page.locator(SETTINGS_SECTION).first().waitFor({ state: "visible", timeout: 5_000 });
  });

  test("displays settings sections", async ({ appPage: page }) => {
    const sections = page.locator(SETTINGS_SECTION);
    const count = await sections.count();
    expect(count).toBeGreaterThanOrEqual(3);
  });

  test("has a Save Settings button", async ({ appPage: page }) => {
    const saveBtn = page.getByRole("button", { name: /Save Settings/i });
    await expect(saveBtn).toBeVisible();
  });

  test("shows language selector", async ({ appPage: page }) => {
    const langSection = page.locator(SETTINGS_SECTION).first();
    const langText = await langSection.textContent();
    expect(langText).toMatch(/English|Chinese|language/i);
  });

  test("shows IO scheduling table", async ({ appPage: page }) => {
    const ioRows = page.locator(IO_TABLE_ROW);
    const count = await ioRows.count();
    expect(count).toBeGreaterThanOrEqual(3);
  });

  test("IO table shows device types", async ({ appPage: page }) => {
    const ioSection = page.locator(".io-table");
    if (await ioSection.isVisible().catch(() => false)) {
      const text = await ioSection.textContent();
      expect(text).toMatch(/HDD|SSD|RAID|Network/i);
    }
  });

  test("can click Save Settings button", async ({ appPage: page }) => {
    const saveBtn = page.getByRole("button", { name: /Save Settings/i });
    await saveBtn.click();
    await page.waitForTimeout(500);
  });
});
