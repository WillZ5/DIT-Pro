import { test, expect } from "./fixtures/app.fixture";
import { navItem, navItemActive } from "./helpers/selectors";

test.describe("MHL Verify View", () => {
  test("can run a demo MHL verification", async ({ appPage: page }) => {
    await page.locator(navItem("MHL Verify")).click();
    await expect(page.locator(navItemActive("MHL Verify"))).toBeVisible();
    await expect(page.getByRole("heading", { name: "MHL Verify" })).toBeVisible();

    await page.getByRole("button", { name: "Select Folder" }).click();
    await expect(page.getByPlaceholder(/Select a folder/)).toHaveValue(/RAID_SHUTTLE_01/);

    await page.getByRole("button", { name: "Verify", exact: true }).click();
    await expect(page.getByText("PASS").first()).toBeVisible();
    await expect(page.getByText("No issues detected")).toBeVisible();
  });
});
