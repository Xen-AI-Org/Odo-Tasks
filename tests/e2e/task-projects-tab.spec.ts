import { expect, test } from "@playwright/test";

test("opens an empty Projects page from below Tasks", async ({ page }) => {
  await page.goto("/");

  const tasksTab = page.locator('[data-view="tasks"]');
  const projectsTab = page.locator('[data-view="projects"]');
  await expect(tasksTab).toBeVisible();
  await expect(projectsTab).toBeVisible();
  await projectsTab.click();

  await expect(projectsTab).toHaveClass(/is-selected/);
  await expect(page.locator('main.projects-view[aria-label="Projects"]')).toBeVisible();
  await expect(page.locator("main.projects-view")).toBeEmpty();
});
