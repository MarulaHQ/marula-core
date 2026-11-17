// Flow 13 — Contextual help (Wave 26 lane 2): the ? help panel opens on
// representative surfaces with route-keyed content, tours run once per
// surface, suppression persists across reload, and the session cap holds.
// Wave 71 adds the new exits and the replay: Skip tour ends one tour and
// leaves the global switch alone, Escape ends the current tour, and the
// panel's "Take the tour" row starts a tour on demand even while tours
// are disabled (the switches govern automatic starts only).
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
    // Walk it: Done appears on the last step, so press Next until it does
    // (the workbar tour runs three steps). The exit animates for 200ms
    // and completes AFTER the click, so wait for the overlay to be gone
    // before navigating — that is also when the completion write lands.
    const done = authedPage.getByTestId("tour-done");
    if (!(await done.isVisible().catch(() => false))) {
      const next = authedPage.getByTestId("tour-next");
      for (let i = 0; i < 6 && !(await done.isVisible().catch(() => false)); i++) {
        await next.click();
        await expect(done).toBeVisible({ timeout: 5_000 }).catch(() => {});
      }
    }
    await done.click();
    await expect(authedPage.getByTestId("tour-overlay")).toHaveCount(0);
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
  // The control is disabled until the user's preference row loads; a click
  // before that lands on a disabled input and silently does nothing (it
  // flaked exactly that way on a loaded box). Wait for the load, then act.
  await expect(toggle).toBeEnabled({ timeout: 20_000 });
  await toggle.check();
  await expect(toggle).toBeChecked();
  await authedPage.reload();
  await expect(toggle).toBeVisible({ timeout: 20_000 });
  await expect(toggle).toBeEnabled({ timeout: 20_000 });
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

test("skip tour ends the tour and leaves the global switch alone", async ({
  authedPage,
  ownerTokens,
}) => {
  await resetTourState(ownerTokens.access_token);
  await authedPage.goto("/today");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  const overlay = authedPage.getByTestId("tour-overlay");
  await expect(overlay).toBeVisible({ timeout: 8_000 });
  // markCompleted is a settings PUT fired without await by the scheduler:
  // overlay removal is not a persistence barrier, so arm the response
  // before acting and wait for it before any reset.
  const persisted = authedPage.waitForResponse(
    (r) => r.url().includes("/settings/user") && r.request().method() === "PUT",
  );
  await authedPage.getByTestId("tour-skip").click();
  await expect(authedPage.getByTestId("tour-overlay")).toHaveCount(0);
  await persisted;
  // Skip is THIS tour only: the global switch must still be off.
  await authedPage.goto("/settings/preferences");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  const toggle = authedPage.getByTestId("pref-tours-disabled");
  await expect(toggle).toBeVisible({ timeout: 20_000 });
  await expect(toggle).toBeEnabled({ timeout: 20_000 });
  await expect(toggle).not.toBeChecked();
  // RESTORE: the skip marked the workbar tour completed.
  await resetTourState(ownerTokens.access_token);
});

test("escape ends the current tour", async ({ authedPage, ownerTokens }) => {
  await resetTourState(ownerTokens.access_token);
  await authedPage.goto("/today");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await expect(authedPage.getByTestId("tour-overlay")).toBeVisible({ timeout: 8_000 });
  const persisted = authedPage.waitForResponse(
    (r) => r.url().includes("/settings/user") && r.request().method() === "PUT",
  );
  await authedPage.keyboard.press("Escape");
  await expect(authedPage.getByTestId("tour-overlay")).toHaveCount(0);
  await persisted;
  await resetTourState(ownerTokens.access_token);
});

test("take the tour replays on demand even while tours are disabled", async ({
  authedPage,
  ownerTokens,
}) => {
  await resetTourState(ownerTokens.access_token);
  // Arm the account switch: from here on, no auto-start may fire.
  await authedFetch(ownerTokens.access_token)("/settings/user", {
    method: "PUT",
    body: { key: "tours.disabled", value: true },
  });
  await authedPage.goto("/tickets");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await authedPage.waitForTimeout(2_000);
  await expect(authedPage.getByTestId("tour-overlay")).toHaveCount(0);
  // Explicit replay: the panel row starts the tour anyway (user ruling
  // 2026-11-17 — the switches govern automatic starts only, never a
  // deliberate ask).
  await authedPage.getByTestId("help-button").click();
  await expect(authedPage.getByTestId("help-take-tour")).toBeVisible();
  await authedPage.getByTestId("help-take-tour").click();
  await expect(authedPage.getByTestId("tour-overlay")).toBeVisible({ timeout: 8_000 });
  const persisted = authedPage.waitForResponse(
    (r) => r.url().includes("/settings/user") && r.request().method() === "PUT",
  );
  await authedPage.getByTestId("tour-skip").click();
  await expect(authedPage.getByTestId("tour-overlay")).toHaveCount(0);
  await persisted;
  // RESTORE: disabled was armed FOR this test.
  await resetTourState(ownerTokens.access_token);
});

test("the help panel offers the tour only where one can run", async ({
  authedPage,
}) => {
  await authedPage.goto("/tickets");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await authedPage.getByTestId("help-button").click();
  await expect(authedPage.getByTestId("help-panel")).toBeVisible();
  await expect(authedPage.getByTestId("help-take-tour")).toBeVisible();
  await authedPage.goto("/settings/fx");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await authedPage.getByTestId("help-button").click();
  await expect(authedPage.getByTestId("help-panel")).toBeVisible();
  await expect(authedPage.getByTestId("help-take-tour")).toHaveCount(0);
});
