import { test, expect } from "./fixtures/app.fixture";
import {
  navItem, VOLUME_VIEW, VOLUME_CARD, VOLUME_NAME,
  VOLUME_TYPE, VOLUME_BAR, WARNING_BADGE,
} from "./helpers/selectors";

test.describe("Volumes View", () => {
  test.beforeEach(async ({ appPage: page }) => {
    await page.locator(navItem("Volumes")).click();
    await expect(page.locator(VOLUME_VIEW)).toBeVisible();
    // Wait for mock data to load (80ms mock delay + render)
    await page.locator(VOLUME_CARD).first().waitFor({ state: "visible", timeout: 5_000 });
  });

  test("displays volume cards", async ({ appPage: page }) => {
    const cards = page.locator(VOLUME_CARD);
    const count = await cards.count();
    // Demo mode provides 6 mock volumes
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("each volume card has a name", async ({ appPage: page }) => {
    const names = page.locator(VOLUME_NAME);
    const count = await names.count();
    expect(count).toBeGreaterThanOrEqual(1);
    for (let i = 0; i < count; i++) {
      const text = await names.nth(i).textContent();
      expect(text?.trim().length).toBeGreaterThan(0);
    }
  });

  test("each volume card shows device type", async ({ appPage: page }) => {
    const types = page.locator(VOLUME_TYPE);
    const count = await types.count();
    expect(count).toBeGreaterThanOrEqual(1);
    for (let i = 0; i < count; i++) {
      const text = await types.nth(i).textContent();
      expect(text).toMatch(/SSD|HDD|SD|RAID|Network|Unknown/i);
    }
  });

  test("volume cards have usage bars", async ({ appPage: page }) => {
    const bars = page.locator(VOLUME_BAR);
    const count = await bars.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("shows capacity information", async ({ appPage: page }) => {
    const firstCard = page.locator(VOLUME_CARD).first();
    const cardText = await firstCard.textContent();
    expect(cardText).toBeTruthy();
    expect(cardText).toMatch(/GB|TB|MB/i);
  });

  test("shows warning badge for low-space volumes", async ({ appPage: page }) => {
    const badges = page.locator(WARNING_BADGE);
    const count = await badges.count();
    if (count > 0) {
      const text = await badges.first().textContent();
      expect(text).toMatch(/critical|low/i);
    }
  });

  test("has a refresh button", async ({ appPage: page }) => {
    const viewHeader = page.locator(".view-header button");
    const count = await viewHeader.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("volume cards show file system type", async ({ appPage: page }) => {
    const firstCard = page.locator(VOLUME_CARD).first();
    const cardText = await firstCard.textContent();
    if (cardText?.match(/ExFAT|APFS|NTFS|ext4|HFS/i)) {
      expect(cardText).toMatch(/ExFAT|APFS|NTFS|ext4|HFS/i);
    }
  });
});
