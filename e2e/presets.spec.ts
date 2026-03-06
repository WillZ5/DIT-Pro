import { test, expect } from "./fixtures/app.fixture";
import {
  navItem, PRESETS_VIEW, PRESET_CARD, PRESET_CARD_NAME,
  PRESET_EDITOR, ALGO_CHIP, ALGO_CHIP_ACTIVE, BTN_PRIMARY,
} from "./helpers/selectors";

test.describe("Presets View", () => {
  test.beforeEach(async ({ appPage: page }) => {
    await page.locator(navItem("Presets")).click();
    await expect(page.locator(PRESETS_VIEW)).toBeVisible({ timeout: 5_000 });
    // Wait for preset cards to render (mock data loads async)
    await page.locator(PRESET_CARD).first().waitFor({ state: "visible", timeout: 5_000 });
  });

  test("displays preset cards", async ({ appPage: page }) => {
    const cards = page.locator(PRESET_CARD);
    const count = await cards.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("each preset has a name", async ({ appPage: page }) => {
    const names = page.locator(PRESET_CARD_NAME);
    const count = await names.count();
    expect(count).toBeGreaterThanOrEqual(1);
    for (let i = 0; i < count; i++) {
      const text = await names.nth(i).textContent();
      expect(text?.trim().length).toBeGreaterThan(0);
    }
  });

  test("has New Preset button", async ({ appPage: page }) => {
    const newBtn = page.locator("button").filter({ hasText: /New Preset/i });
    await expect(newBtn).toBeVisible();
  });

  test("clicking New Preset shows the editor", async ({ appPage: page }) => {
    const newBtn = page.locator("button").filter({ hasText: /New Preset/i });
    await newBtn.click();
    await expect(page.locator(PRESET_EDITOR)).toBeVisible({ timeout: 3_000 });
  });

  test("preset editor has algorithm selection", async ({ appPage: page }) => {
    const newBtn = page.locator("button").filter({ hasText: /New Preset/i });
    await newBtn.click();
    await expect(page.locator(PRESET_EDITOR)).toBeVisible({ timeout: 3_000 });

    const chips = page.locator(`${PRESET_EDITOR} ${ALGO_CHIP}`);
    const count = await chips.count();
    expect(count).toBeGreaterThanOrEqual(2);
  });

  test("can toggle algorithm chips", async ({ appPage: page }) => {
    const newBtn = page.locator("button").filter({ hasText: /New Preset/i });
    await newBtn.click();
    await expect(page.locator(PRESET_EDITOR)).toBeVisible({ timeout: 3_000 });

    const activeBefore = await page.locator(`${PRESET_EDITOR} ${ALGO_CHIP_ACTIVE}`).count();
    const firstChip = page.locator(`${PRESET_EDITOR} ${ALGO_CHIP}`).first();
    await firstChip.click();

    const activeAfter = await page.locator(`${PRESET_EDITOR} ${ALGO_CHIP_ACTIVE}`).count();
    expect(activeAfter).not.toBe(activeBefore);
  });

  test("clicking a preset card shows its details", async ({ appPage: page }) => {
    const firstCard = page.locator(PRESET_CARD).first();
    const editBtn = firstCard.locator("button").filter({ hasText: /edit/i });
    if (await editBtn.isVisible().catch(() => false)) {
      await editBtn.click();
      await expect(page.locator(PRESET_EDITOR)).toBeVisible({ timeout: 3_000 });
    }
  });

  test("preset cards show hash algorithm info", async ({ appPage: page }) => {
    const firstCard = page.locator(PRESET_CARD).first();
    const cardText = await firstCard.textContent();
    expect(cardText).toMatch(/XXH64|SHA256|MD5|XXH3|XXH128/i);
  });

  test("preset cards show feature badges", async ({ appPage: page }) => {
    const firstCard = page.locator(PRESET_CARD).first();
    const cardText = await firstCard.textContent();
    expect(cardText).toMatch(/MHL|CASCADE|Verify/i);
  });
});
