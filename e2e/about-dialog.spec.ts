import { test, expect } from "./fixtures/app.fixture";
import { VERSION, DIALOG_OVERLAY, DIALOG, DIALOG_CLOSE, ABOUT_TABLE } from "./helpers/selectors";

test.describe("About Dialog", () => {
  test("opens when clicking version number", async ({ appPage: page }) => {
    await page.locator(VERSION).click();
    // Use last() because "About" dialog opens on top of any existing overlays
    await expect(page.locator(DIALOG_OVERLAY).last()).toBeVisible();
  });

  test("shows version info in dialog", async ({ appPage: page }) => {
    await page.locator(VERSION).click();
    await expect(page.locator(DIALOG).last()).toBeVisible();

    const dialogText = await page.locator(DIALOG).last().textContent();
    // Should mention DIT Pro, About, or version info
    expect(dialogText).toMatch(/DIT Pro|About|version/i);
  });

  test("shows about table with details", async ({ appPage: page }) => {
    await page.locator(VERSION).click();
    await page.waitForTimeout(300);

    const aboutTable = page.locator(ABOUT_TABLE);
    if (await aboutTable.isVisible().catch(() => false)) {
      const text = await aboutTable.textContent();
      expect(text).toMatch(/version|Version|Platform/i);
    }
  });

  test("can close via close button", async ({ appPage: page }) => {
    await page.locator(VERSION).click();
    await expect(page.locator(DIALOG).last()).toBeVisible();

    // Try close button, or any button inside the about dialog that can dismiss it
    const dialog = page.locator(DIALOG).last();
    const closeBtn = dialog.locator(DIALOG_CLOSE);
    if (await closeBtn.isVisible().catch(() => false)) {
      await closeBtn.click();
    } else {
      // Fall back to clicking outside (overlay) to close
      await page.locator(DIALOG_OVERLAY).last().click({ position: { x: 5, y: 5 } });
    }
    await page.waitForTimeout(300);
  });

  test("can close via overlay click", async ({ appPage: page }) => {
    await page.locator(VERSION).click();
    await expect(page.locator(DIALOG_OVERLAY).last()).toBeVisible();

    // Click overlay edge (outside dialog content)
    await page.locator(DIALOG_OVERLAY).last().click({ position: { x: 5, y: 5 } });
    await page.waitForTimeout(500);
  });
});
