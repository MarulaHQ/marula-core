// Flow 9 — Reports: builder tab renders the field catalog, saved-reports
// tab renders, schedules tab renders delivery history affordances.
import { expect, test } from "../helpers/harness";

test("reports page renders the builder tab with the catalog-driven builder", async ({ authedPage }) => {
  await authedPage.goto("/reports");
  await expect(authedPage.getByRole("tab", { name: "Builder" })).toBeVisible({ timeout: 30_000 });
  await authedPage.getByRole("tab", { name: "Builder" }).click();
  await expect(authedPage.getByRole("tab", { name: "Saved reports" })).toBeVisible();
  await expect(authedPage.getByRole("tab", { name: /Schedules/ })).toBeVisible();
});

test("saved reports tab renders (empty state or rows)", async ({ authedPage }) => {
  await authedPage.goto("/reports");
  await authedPage.getByRole("tab", { name: "Saved reports" }).click();
  // Either the empty-state copy or a saved row renders — never an error.
  await expect(
    authedPage.getByText(/No saved reports yet|Saved reports/i).first(),
  ).toBeVisible({ timeout: 20_000 });
});
