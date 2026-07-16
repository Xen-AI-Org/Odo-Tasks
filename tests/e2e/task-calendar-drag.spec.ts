import { expect, type Locator, type Page, test } from "@playwright/test";

async function pointerDrag(page: Page, source: Locator, target: Locator, hold = false) {
  await source.scrollIntoViewIfNeeded();
  await target.scrollIntoViewIfNeeded();
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  if (!sourceBox || !targetBox) throw new Error("Drag source or target is not visible");
  const start = { x: sourceBox.x + sourceBox.width / 2, y: sourceBox.y + sourceBox.height / 2 };
  const end = { x: targetBox.x + targetBox.width / 2, y: targetBox.y + targetBox.height / 2 };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(end.x, end.y, { steps: 12 });
  if (!hold) await page.mouse.up();
  return { sourceBox, end };
}

test("task cards follow the pointer without selecting text when scheduled and moved", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.locator('[data-view="tasks"]').click();

  await page.locator("#quick-task-input").fill("Drag preview regression");
  await page.locator("#quick-task-input").press("Enter");

  const inboxTask = page.locator(".planner-task-card", { hasText: "Drag preview regression" }).first();
  const firstSlot = page.locator(".calendar-column").first().locator('[data-slot-minute="600"]');
  const firstDrag = await pointerDrag(page, inboxTask, firstSlot, true);
  const inboxPreview = page.locator(".planner-drag-preview.planner-task-card");

  await expect(inboxPreview).toBeVisible();
  await expect(inboxPreview).toContainText("Drag preview regression");
  await expect.poll(() => page.evaluate(() => window.getSelection()?.toString() ?? "")).toBe("");
  await expect(page.locator("html")).toHaveClass(/is-planner-dragging/);
  const previewBox = await inboxPreview.boundingBox();
  expect(previewBox?.width).toBeCloseTo(firstDrag.sourceBox.width, 0);
  expect(previewBox?.x).toBeLessThan(firstDrag.end.x);
  expect(previewBox?.x).toBeGreaterThan(firstDrag.end.x - firstDrag.sourceBox.width);
  await page.mouse.up();

  const calendarTask = page.locator(".calendar-task", { hasText: "Drag preview regression" });
  await expect(calendarTask).toBeVisible();
  await expect(page.locator("html")).not.toHaveClass(/is-planner-dragging/);
  await expect.poll(() => page.evaluate(() => {
    const workspace = JSON.parse(localStorage.getItem("odo-notes-workspace-v2") ?? "null");
    return workspace?.todos.find((item: { text: string }) => item.text === "Drag preview regression")?.scheduledStart ?? null;
  })).not.toBeNull();

  const secondSlot = page.locator(".calendar-column").first().locator('[data-slot-minute="720"]');
  const calendarDrag = await pointerDrag(page, calendarTask, secondSlot, true);
  const calendarPreview = page.locator(".planner-drag-preview.calendar-task");
  await expect(calendarPreview).toBeVisible();
  await expect(calendarPreview).toContainText("Drag preview regression");
  const calendarPreviewBox = await calendarPreview.boundingBox();
  expect(calendarPreviewBox?.width).toBeCloseTo(calendarDrag.sourceBox.width, 0);
  expect(calendarPreviewBox?.height).toBeCloseTo(calendarDrag.sourceBox.height, 0);
  await expect.poll(() => page.evaluate(() => window.getSelection()?.toString() ?? "")).toBe("");
  await page.mouse.up();

  await expect.poll(() => page.evaluate(() => {
    const workspace = JSON.parse(localStorage.getItem("odo-notes-workspace-v2") ?? "null");
    const scheduled = workspace?.todos.find((item: { text: string }) => item.text === "Drag preview regression")?.scheduledStart;
    return scheduled ? new Date(scheduled).getHours() * 60 + new Date(scheduled).getMinutes() : null;
  })).toBe(720);
});
