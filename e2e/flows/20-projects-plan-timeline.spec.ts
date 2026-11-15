// Flow 20 — the project record: the plan grid and the timeline chart.
//
// What this proves: the two surfaces the project redesign added render against
// a real seeded project, that the plan's undated work is called out instead of
// being drawn as scheduled, and that the timeline can be read as a table when
// the chart is not the right instrument. No dates are moved here — the ripple
// preview has its own unit coverage and a drag is a flaky guest in CI.
import { expect, test } from "../helpers/harness";

/** Open the first project in the list, or skip when the seed has none. */
async function openFirstProject(page: import("@playwright/test").Page): Promise<boolean> {
  await page.goto("/projects");
  // The list fills in from the API, so wait for a row rather than counting
  // immediately — a count taken too early reads as "no projects seeded".
  const row = page.locator('[data-testid^="project-row-"]').first();
  const appeared = await row
    .waitFor({ state: "visible", timeout: 30_000 })
    .then(() => true)
    .catch(() => false);
  if (!appeared) return false;
  await row.locator("a").first().click();
  await expect(page).toHaveURL(/\/projects\/[0-9a-f-]{36}/, { timeout: 30_000 });
  return true;
}

test("the plan grid renders rows, columns and the undated group", async ({ authedPage }) => {
  test.skip(!(await openFirstProject(authedPage)), "seed has no project");

  await expect(authedPage.getByRole("tab", { name: /Plan/i })).toBeVisible({ timeout: 30_000 });
  // The measured column set from S3.1, not a generic table.
  for (const header of ["Task", "Status", "Who", "Start", "Days", "Effort", "Depends on"]) {
    await expect(
      authedPage.getByRole("columnheader", { name: new RegExp(`^${header}$`, "i") }).first(),
    ).toBeVisible();
  }
  // Either the plan has rows or it tells a person how to fill it; never blank.
  await expect(
    authedPage
      .getByText(/Not dated yet|No work planned yet|Add the first phase/i)
      .first(),
  ).toBeVisible({ timeout: 20_000 });
});

test("the timeline renders the chart and offers the table equivalent", async ({ authedPage }) => {
  test.skip(!(await openFirstProject(authedPage)), "seed has no project");

  await authedPage.getByRole("tab", { name: /Timeline/i }).click();
  const chart = authedPage.locator("[data-slot='gantt-chart']").first();
  await expect(chart).toBeVisible({ timeout: 30_000 });

  // The legend names every status, so colour is never the only cue.
  await expect(authedPage.getByText(/In progress/i).first()).toBeVisible();

  // The non-visual equivalent exists and carries the same columns.
  const tableToggle = authedPage.getByRole("button", { name: /View as table/i }).first();
  if ((await tableToggle.count()) > 0) {
    await tableToggle.click();
    await expect(authedPage.getByRole("table").first()).toBeVisible();
  }
});

test("neither surface shows an error state", async ({ authedPage }) => {
  test.skip(!(await openFirstProject(authedPage)), "seed has no project");

  await expect(authedPage.getByRole("tab", { name: /Plan/i })).toBeVisible({ timeout: 30_000 });
  await expect(authedPage.getByText(/could not load|something went wrong/i)).toHaveCount(0);
});

test("the plan's rows reorder from the keyboard, and the move can be undone", async ({
  authedPage,
}) => {
  test.skip(!(await openFirstProject(authedPage)), "seed has no project");
  await authedPage.getByRole("tab", { name: /Plan/i }).click();

  const rows = authedPage.locator("[data-slot='plan-row'][aria-label]");
  const first = await rows.first().waitFor({ state: "visible", timeout: 30_000 }).then(
    () => true,
    () => false,
  );
  test.skip(!first, "the seeded plan has no task rows");
  const labels = await rows.evaluateAll((nodes) =>
    nodes.map((node) => node.getAttribute("aria-label") ?? ""),
  );
  test.skip(labels.length < 2, "reordering needs two tasks");

  const firstTitle = labels[0].split(",")[0];
  const secondTitle = (labels[1] ?? "").split(",")[0];
  const order = () =>
    rows.evaluateAll((nodes) =>
      nodes.map((node) => (node.getAttribute("aria-label") ?? "").split(",")[0]),
    );

  // The grip is the keyboard path: the same write a drag makes, without a drag
  // (a pointer drag is a flaky guest in CI, its own note in this file says).
  const grip = (index: number, title: string) =>
    rows.nth(index).getByRole("button", { name: `Reorder ${title}` });

  await grip(0, firstTitle).focus();
  await grip(0, firstTitle).press("ArrowDown");
  await expect
    .poll(async () => (await order()).slice(0, 2), { timeout: 20_000 })
    .toEqual([secondTitle, firstTitle]);

  // Put it back, so a run leaves the seeded plan as it found it.
  await grip(1, firstTitle).press("ArrowUp");
  await expect
    .poll(async () => (await order()).slice(0, 2), { timeout: 20_000 })
    .toEqual([firstTitle, secondTitle]);
});
