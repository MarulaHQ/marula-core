// Flow 7 — Scorecards (Wave 14 lane 3): per-tech scorecards render over the
// aggregated read model with the seeded demo data.
import { expect, test } from "../helpers/harness";

test("scorecards page renders with demo aggregation data", async ({ authedPage }) => {
  await authedPage.goto("/scorecards");
  await expect(authedPage.getByRole("heading", { name: "Scorecards" })).toBeVisible({
    timeout: 30_000,
  });
  // The aggregation renders (own metrics or the honest empty state).
  await expect(
    authedPage.getByRole("region", { name: "Your scorecard" }),
  ).toBeVisible({ timeout: 30_000 });
});
