import { expect, test } from "@playwright/test";

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
