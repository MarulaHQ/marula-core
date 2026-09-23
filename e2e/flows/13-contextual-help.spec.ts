// Flow 13 — Contextual help (Wave 26 lane 2): the ? help panel opens on
// representative surfaces with route-keyed content, tours run once per
// surface, suppression persists across reload, and the session cap holds.
import { expect, test } from "../helpers/harness";

test("help panel opens on a representative surface with route-keyed content", async ({
  authedPage,
}) => {
  await authedPage.goto("/tickets");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await authedPage.getByTestId("help-button").click();
  await expect(authedPage.getByTestId("help-panel")).toBeVisible();
  await expect(authedPage.getByTestId("help-surface-label")).toContainText("tickets");
  await expect(authedPage.getByTestId("help-doc-entry").first()).toBeVisible();
  // The staff ticket guide is the canonical doc for this route key.
  await expect(authedPage.getByText("How to work a ticket").first()).toBeVisible();
});

test("help panel content changes with the surface (settings/fx)", async ({
  authedPage,
}) => {
  await authedPage.goto("/settings/fx");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await authedPage.getByTestId("help-button").click();
  await expect(authedPage.getByTestId("help-panel")).toBeVisible();
  await expect(authedPage.getByTestId("help-surface-label")).toContainText("settings fx");
  await expect(authedPage.getByTestId("help-doc-entry").first()).toBeVisible();
});

test("tour cap: one auto-started tour per session, then suppression", async ({
  authedPage,
}) => {
  // Fresh session: the first auto tour (Today surface) may start.
  await authedPage.goto("/today");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  const overlay = authedPage.getByTestId("tour-overlay");
  const appeared = await overlay
    .waitFor({ state: "visible", timeout: 8_000 })
    .then(() => true)
    .catch(() => false);
  if (appeared) {
    // Walk or skip it — completion persists per account.
    const done = authedPage.getByTestId("tour-done");
    if (await done.isVisible().catch(() => false)) {
      await done.click();
    } else {
      await authedPage.getByTestId("tour-next").click();
      await authedPage.getByTestId("tour-done").click();
    }
  }
  // Navigating onward in the SAME session must never start a second tour.
  await authedPage.goto("/tickets");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await authedPage.waitForTimeout(2_000);
  await expect(authedPage.getByTestId("tour-overlay")).toHaveCount(0);
});

test("tour suppression persists across reload via the user setting", async ({
  authedPage,
}) => {
  await authedPage.goto("/settings/preferences");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  const toggle = authedPage.getByTestId("pref-tours-disabled");
  await expect(toggle).toBeVisible({ timeout: 20_000 });
  await toggle.check();
  await expect(toggle).toBeChecked();
  await authedPage.reload();
  await expect(toggle).toBeVisible({ timeout: 20_000 });
  await expect(toggle).toBeChecked();
  // With suppression on, no tour auto-starts on any surface.
  await authedPage.goto("/today");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await authedPage.waitForTimeout(2_000);
  await expect(authedPage.getByTestId("tour-overlay")).toHaveCount(0);
});
