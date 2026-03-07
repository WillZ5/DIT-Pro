import { test as base, expect, type Page } from "@playwright/test";
import { APP_ROOT, SIDEBAR, DEMO_BADGE } from "../helpers/selectors";

/**
 * Shared fixture that navigates to the app and waits for initial render.
 * Dismisses the "Testing Notice" dialog that appears on first load.
 */
export const test = base.extend<{ appPage: Page }>({
  appPage: async ({ page }, use) => {
    await page.goto("/");

    // Wait for the app shell to render
    await page.waitForSelector(APP_ROOT, { timeout: 10_000 });
    await page.waitForSelector(SIDEBAR, { timeout: 5_000 });

    // Dismiss the "Testing Notice" dialog — click "I Understand" button
    // This dialog appears on app start in non-stable channels
    const understandBtn = page.getByRole("button", { name: /I Understand/i });
    try {
      await understandBtn.waitFor({ state: "visible", timeout: 3_000 });
      await understandBtn.click();
      // Wait for overlay to close
      await page.waitForTimeout(500);
    } catch {
      // Dialog might not appear (e.g., stable channel) — that's fine
    }

    // Double-check: if any dialog overlay still visible, dismiss it
    const overlay = page.locator(".dialog-overlay");
    if (await overlay.first().isVisible().catch(() => false)) {
      // Try clicking the button inside the dialog
      const dialogBtn = page.locator(".dialog button.btn-primary").first();
      if (await dialogBtn.isVisible().catch(() => false)) {
        await dialogBtn.click();
        await page.waitForTimeout(300);
      }
    }

    // Verify demo badge is present (confirms browser/demo mode)
    await expect(page.locator(DEMO_BADGE)).toBeVisible({ timeout: 3_000 });

    await use(page); // eslint-disable-line react-hooks/rules-of-hooks
  },
});

export { expect };
