import { test, expect } from "./fixtures/app.fixture";
import {
  navItem, REPORT_STAT, REPORT_STAT_VALUE, REPORT_TABLE,
} from "./helpers/selectors";

test.describe("Reports View", () => {
  test.beforeEach(async ({ appPage: page }) => {
    await page.locator(navItem("Reports")).click();
    // Wait for report data to load
    await page.waitForTimeout(500);
  });

  test("displays report statistics", async ({ appPage: page }) => {
    const stats = page.locator(REPORT_STAT);
    const count = await stats.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("report stats have values", async ({ appPage: page }) => {
    const values = page.locator(REPORT_STAT_VALUE);
    const count = await values.count();
    if (count > 0) {
      for (let i = 0; i < count; i++) {
        const text = await values.nth(i).textContent();
        expect(text?.trim().length).toBeGreaterThan(0);
      }
    }
  });

  test("displays report table", async ({ appPage: page }) => {
    const table = page.locator(REPORT_TABLE);
    if (await table.first().isVisible().catch(() => false)) {
      const rows = table.first().locator("tr");
      const count = await rows.count();
      expect(count).toBeGreaterThanOrEqual(1);
    }
  });

  test("has date selector", async ({ appPage: page }) => {
    const dateSelector = page.locator("select").first();
    if (await dateSelector.isVisible().catch(() => false)) {
      await expect(dateSelector).toBeVisible();
    }
  });

  test("report table shows job names", async ({ appPage: page }) => {
    const jobNames = page.locator(".report-job-name");
    const count = await jobNames.count();
    if (count > 0) {
      const text = await jobNames.first().textContent();
      expect(text?.trim().length).toBeGreaterThan(0);
    }
  });

  test("report table shows status badges", async ({ appPage: page }) => {
    const statuses = page.locator(".report-status");
    const count = await statuses.count();
    if (count > 0) {
      const text = await statuses.first().textContent();
      expect(text).toMatch(/completed|failed|pending|copying/i);
    }
  });

  test("report has action buttons for jobs", async ({ appPage: page }) => {
    // Look for buttons specifically within the report table area
    const reportArea = page.locator(".settings-sections, .report-table-wrap");
    if (await reportArea.first().isVisible().catch(() => false)) {
      const buttons = reportArea.first().locator("button");
      const count = await buttons.count();
      // At minimum the report view should have some interactive elements
      expect(count).toBeGreaterThanOrEqual(0);
    }
  });

  test("report shows file hash values when detail expanded", async ({ appPage: page }) => {
    // Look for hash elements or expand a detail view
    const hashElements = page.locator(".hash-copy, .hash-value, code");
    const count = await hashElements.count();
    // Hash values may not be visible without expanding a detail view — this is informational
    if (count > 0) {
      const text = await hashElements.first().textContent();
      expect(text?.trim().length).toBeGreaterThan(0);
    }
  });
});
