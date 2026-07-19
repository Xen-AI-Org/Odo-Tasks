import { expect, test } from "@playwright/test";

test("opens an empty top-level Projects page between Tasks and Journal", async ({ page }) => {
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
  await expect(page.locator("main.projects-view")).toBeEmpty();
});
