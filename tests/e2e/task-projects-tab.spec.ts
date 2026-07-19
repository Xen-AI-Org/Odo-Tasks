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

  const projectSections = page.getByRole("navigation", { name: "Project sections" });
  await expect(projectSections.getByRole("button", { name: /^Milestones/ })).toBeVisible();
  await expect(projectSections.getByRole("button", { name: /^Tasks/ })).toHaveCount(0);
  await projectSections.getByRole("button", { name: /^Milestones/ }).click();

  await page.getByRole("button", { name: "Add milestone" }).click();
  const milestoneDialog = page.getByRole("dialog", { name: "New milestone" });
  await milestoneDialog.getByRole("textbox", { name: "Name" }).fill("Release candidate");
  await milestoneDialog.getByRole("button", { name: "Save" }).click();

  const milestone = page.locator("[data-milestone-id]").filter({ has: page.locator('input[value="Release candidate"]') });
  await expect(milestone).toBeVisible();
  await milestone.getByRole("button", { name: "Add task" }).click();

  const taskDialog = page.getByRole("dialog", { name: "Add a task" });
  await taskDialog.getByRole("textbox", { name: "Name" }).fill("Prepare release notes");
  await taskDialog.getByRole("button", { name: "Save" }).click();

  const taskDetail = page.getByRole("dialog", { name: "Task details" });
  await expect(taskDetail.getByLabel("Project")).toHaveValue("summer-launch");
  await expect(taskDetail.getByLabel("Milestone")).toHaveValue(/^milestone-/);
  await taskDetail.getByLabel("Milestone").selectOption({ label: "No milestone" });
  await expect(taskDetail.getByLabel("Milestone")).toHaveValue("");
  await taskDetail.getByRole("button", { name: "Close task detail" }).click();

  await expect(page.locator(".unassigned-tasks")).toContainText("Prepare release notes");
  for (const width of [320, 375, 414, 768]) {
    await page.setViewportSize({ width, height: 720 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
  }
});
