import { expect, test } from "@playwright/test";

test("moves a note to another folder and persists the destination", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  const note = page.locator('[data-note-id="welcome"]');
  const workFolder = page.locator('[data-folder-id="work"]');

  await expect(note).toBeVisible();
  await note.dragTo(workFolder);

  await expect(page.locator("#note-list-title")).toHaveText("Work");
  await expect(workFolder.locator(".folder-count")).toHaveText("1");
  await expect
    .poll(() => page.evaluate(() => {
      const workspace = JSON.parse(localStorage.getItem("odo-notes-workspace-v2") ?? "null");
      return workspace?.notes.find((item: { id: string }) => item.id === "welcome")?.folderId;
    }))
    .toBe("work");

  await page.reload();

  await expect(page.locator("#note-list-title")).toHaveText("Work");
  await expect(page.locator('[data-folder-id="work"] .folder-count')).toHaveText("1");
  await expect(page.locator('[data-note-id="welcome"]')).toBeVisible();
});
