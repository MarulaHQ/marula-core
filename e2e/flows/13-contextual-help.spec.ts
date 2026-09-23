// Flow 13 — Contextual help (Wave 26 lane 2): the ? help panel opens on
// representative surfaces with route-keyed content, tours run once per
// surface, suppression persists across reload, and the session cap holds.
//
// ORDER-INDEPENDENCE (p37-env-orchestration, see e2e/README.md): the tour
// state (tours.disabled / tours.completed) is per-user persisted settings —
// an earlier run (or the light project before the dark one) may carry real
// suppression/completion rows. Every test that ASSERTS on tour behavior
// resets that state at its start and restores what it armed at its end, so
// the flow makes no assumption about pristine demo rows.
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";

/** Reset THIS account's tours state to the seeded defaults (auto tours
 * may start again) via the public settings API. */
async function resetTourState(token: string) {
  const put = authedFetch(token);
  await put("/settings/user", { method: "PUT", body: { key: "tours.disabled", value: false } });
  await put("/settings/user", { method: "PUT", body: { key: "tours.completed", value: "" } });
}

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
  ownerTokens,
}) => {
  // Order-independence: a prior run may have armed tours suppression or
  // marked tours completed — clear both so an auto tour MAY start.
  await resetTourState(ownerTokens.access_token);
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
  ownerTokens,
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
  // RESTORE: suppression was armed FOR this test — clear it so a later run
  // (dark project, next suite) starts from the seeded default again.
  await resetTourState(ownerTokens.access_token);
});
