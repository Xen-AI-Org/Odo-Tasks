import { expect, test } from "@playwright/test";

test("opens an empty top-level Projects page from below Journal", async ({ page }) => {
  await page.goto("/");

  const journalTab = page.locator('[data-view="journal"]');
  const projectsTab = page.locator('[data-view="projects"]');
  await expect(journalTab).toBeVisible();
  await expect(projectsTab).toBeVisible();
  await expect(journalTab.locator("xpath=following-sibling::*[1]")).toHaveAttribute("data-view", "projects");
  await projectsTab.click();

  await expect(projectsTab).toHaveClass(/is-selected/);
  await expect(page.locator('main.projects-view[aria-label="Projects"]')).toBeVisible();
  await expect(page.locator("main.projects-view")).toBeEmpty();
});
