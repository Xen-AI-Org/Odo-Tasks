import { expect, test, type Page } from "@playwright/test";

const storageKey = "odo-notes-workspace-v2";

async function openProjects(page: Page) {
  await page.goto("/");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.locator('[data-view="projects"]').click();
}

async function storedProjectStatus(page: Page, projectId: string) {
  return page.evaluate(({ key, id }) => {
    const workspace = JSON.parse(localStorage.getItem(key) ?? "null");
    return workspace?.projects.find((project: { id: string }) => project.id === id)?.status;
  }, { key: storageKey, id: projectId });
}

test("opens the Projects board between Tasks and Journal", async ({ page }) => {
  await page.goto("/");

  const tasksTab = page.locator('[data-view="tasks"]');
  const journalTab = page.locator('[data-view="journal"]');
  const projectsTab = page.locator('[data-view="projects"]');
  await expect(tasksTab).toBeVisible();
  await expect(journalTab).toBeVisible();
  await expect(projectsTab).toBeVisible();
  await expect(tasksTab.locator("xpath=following-sibling::*[1]")).toHaveAttribute("data-view", "projects");
  await expect(projectsTab.locator("xpath=following-sibling::*[1]")).toHaveAttribute("data-view", "journal");
  await projectsTab.click();

  await expect(projectsTab).toHaveClass(/is-selected/);
  await expect(page.locator('main.projects-view[aria-label="Projects"]')).toBeVisible();
  await expect(page.getByRole("heading", { name: "Projects", level: 1 })).toBeVisible();
  await expect(page.locator(".project-column")).toHaveCount(4);

  await page.locator('[data-project-id="summer-launch"]').click();
  await expect(page.locator('main.project-detail-view[aria-label="Summer launch project"]')).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Project name" })).toContainText("Summer launch");
});

test("drags a project to another stage with mouse input and persists the status", async ({ page }) => {
  await openProjects(page);

  const project = page.locator('[data-project-id="summer-launch"]');
  const completed = page.locator('[data-project-status="completed"]');
  const sourceBox = await project.boundingBox();
  const targetBox = await completed.locator(".project-column-list").boundingBox();
  if (!sourceBox || !targetBox) throw new Error("Expected project and destination stage to be visible");
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(targetBox.x + targetBox.width / 2, targetBox.y + Math.min(targetBox.height / 2, 90), { steps: 12 });
  await expect(completed).toHaveClass(/is-drop-target/);
  await page.mouse.up();

  await expect(page.locator('main.projects-view[aria-label="Projects"]')).toBeVisible();
  await expect(completed.locator('[data-project-id="summer-launch"]')).toBeVisible();
  await expect.poll(() => storedProjectStatus(page, "summer-launch")).toBe("completed");

  await page.reload();
  await page.locator('[data-view="projects"]').click();
  await expect(page.locator('[data-project-status="completed"] [data-project-id="summer-launch"]')).toBeVisible();
});

test("moves a project between stages with touch pointer input", async ({ page }) => {
  await openProjects(page);

  const project = page.locator('[data-project-id="mobile-capture"]');
  const planned = page.locator('[data-project-status="planned"]');
  const sourceBox = await project.boundingBox();
  const targetBox = await planned.locator(".project-column-list").boundingBox();
  if (!sourceBox || !targetBox) throw new Error("Expected project and destination stage to be visible");

  const pointerId = 23;
  await project.dispatchEvent("pointerdown", {
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
  }, { id: pointerId, x: targetBox.x + targetBox.width / 2, y: targetBox.y + Math.min(targetBox.height / 2, 90) });

  await expect(planned).toHaveClass(/is-drop-target/);

  await page.evaluate(({ id, x, y }) => {
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, cancelable: true, pointerId: id, pointerType: "touch", isPrimary: true, button: 0, clientX: x, clientY: y }));
  }, { id: pointerId, x: targetBox.x + targetBox.width / 2, y: targetBox.y + Math.min(targetBox.height / 2, 90) });

  await expect(page.locator('[data-project-status="planned"] [data-project-id="mobile-capture"]')).toBeVisible();
  await expect.poll(() => storedProjectStatus(page, "mobile-capture")).toBe("planned");
});
