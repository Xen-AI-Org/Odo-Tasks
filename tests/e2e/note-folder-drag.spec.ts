import { expect, test, type Page } from "@playwright/test";

const storageKey = "odo-notes-workspace-v2";

async function resetWorkspace(page: Page) {
  await page.goto("/");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
}

async function storedFolderId(page: Page, noteId: string) {
  return page.evaluate(({ key, id }) => {
    const workspace = JSON.parse(localStorage.getItem(key) ?? "null");
    return workspace?.notes.find((item: { id: string }) => item.id === id)?.folderId;
  }, { key: storageKey, id: noteId });
}

test("moves a note to a root folder and persists the destination", async ({ page }) => {
  await resetWorkspace(page);

  const note = page.locator('[data-note-id="welcome"]');
  const workFolder = page.locator('[data-folder-id="work"]');

  await expect(note).toBeVisible();
  await expect(note.locator(".note-drag-handle")).toBeVisible();
  await note.dragTo(workFolder);

  await expect(page.locator("#note-list-title")).toHaveText("Work");
  await expect(workFolder.locator(".folder-count")).toHaveText("1");
  await expect.poll(() => storedFolderId(page, "welcome")).toBe("work");

  await page.reload();

  await expect(page.locator("#note-list-title")).toHaveText("Work");
  await expect(page.locator('[data-folder-id="work"] .folder-count')).toHaveText("1");
  await expect(page.locator('[data-note-id="welcome"]')).toBeVisible();
});

test("moves a note to a nested folder with touch pointer input", async ({ page }) => {
  await resetWorkspace(page);

  const note = page.locator('[data-note-id="welcome"]');
  const summerFolder = page.locator('[data-folder-id="summer"]');
  const sourceBox = await note.boundingBox();
  const targetBox = await summerFolder.boundingBox();
  if (!sourceBox || !targetBox) throw new Error("Expected note and nested folder to be visible");

  const pointerId = 17;
  await note.dispatchEvent("pointerdown", {
    pointerId,
    pointerType: "touch",
    isPrimary: true,
    button: 0,
    buttons: 1,
    clientX: sourceBox.x + sourceBox.width / 2,
    clientY: sourceBox.y + sourceBox.height / 2,
  });
  await page.evaluate(({ id, x, y }) => {
    window.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, cancelable: true, pointerId: id, pointerType: "touch", isPrimary: true, buttons: 1, clientX: x, clientY: y }));
  }, { id: pointerId, x: targetBox.x + targetBox.width / 2, y: targetBox.y + targetBox.height / 2 });

  await expect(summerFolder).toHaveClass(/is-drop-target/);

  await page.evaluate(({ id, x, y }) => {
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, cancelable: true, pointerId: id, pointerType: "touch", isPrimary: true, button: 0, clientX: x, clientY: y }));
  }, { id: pointerId, x: targetBox.x + targetBox.width / 2, y: targetBox.y + targetBox.height / 2 });

  await expect(page.locator("#note-list-title")).toHaveText("Summer launch");
  await expect.poll(() => storedFolderId(page, "welcome")).toBe("summer");

  await page.reload();

  await expect(page.locator("#note-list-title")).toHaveText("Summer launch");
  await expect(page.locator('[data-folder-id="projects"]')).toBeVisible();
  await expect(page.locator('[data-folder-id="summer"]')).toBeVisible();
  await expect(page.locator('[data-note-id="welcome"]')).toBeVisible();
});

test("ignores external text dragged over a folder", async ({ page }) => {
  await resetWorkspace(page);

  const workFolder = page.locator('[data-folder-id="work"]');
  const storageBefore = await page.evaluate((key) => localStorage.getItem(key), storageKey);
  await workFolder.evaluate((target) => {
    const dataTransfer = new DataTransfer();
    dataTransfer.setData("text/plain", "welcome");
    for (const type of ["dragenter", "dragover", "drop"]) {
      target.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer }));
    }
  });

  await expect(workFolder).not.toHaveClass(/is-drop-target/);
  await expect(page.locator("#note-list-title")).toHaveText("Inbox");
  await expect(page.locator('[data-note-id="welcome"]')).toBeVisible();
  await expect(page.locator('[data-folder-id="work"] .folder-count')).toHaveText("0");
  await expect.poll(() => page.evaluate((key) => localStorage.getItem(key), storageKey)).toBe(storageBefore);
});
