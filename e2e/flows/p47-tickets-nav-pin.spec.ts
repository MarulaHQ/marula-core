// Flow — the /tickets client-navigation pin (Wave 47 lane A21).
//
// A15 measured, on the 2026-10-17 run, that every client-side transition FROM
// /tickets was a no-op: Alt+2, Alt+1, mod+k, a rail-link click and a direct
// `window.next.router.push("/clients")` all left the URL at /tickets, with no
// history.pushState and no in-flight request, while every other route navigated
// and the same chords worked from /assets. The cause was this surface
// re-rendering forever whenever no surface document was saved: the memo chain
// the bulk-status effect depends on (`clauses` -> `filtered` -> `shown`) lost
// its identity every render, so the effect re-ran and set a fresh array each
// time. A loop that never settles starves the App Router's transitions, so
// `router.push` never commits and never reaches `history.pushState`.
//
// This spec is the regression, and it owns its precondition rather than
// assuming the seed: it reads the current `appearance.data_surfaces` document,
// clears it (the shape a fresh account and the demo both have), requires both
// navigation paths to land, and puts the previous value back. The /assets leg
// is the control — the same chord, a different starting route.
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";

const SURFACE_KEY = "appearance.data_surfaces";

test("client-side navigation lands from /tickets with no saved surface document", async ({
  authedPage,
  ownerTokens,
}) => {
  const fetcher = authedFetch(ownerTokens.access_token);
  const rows = await fetcher<{ key: string; value: string }[]>("/settings/user");
  const previous = rows.find((row) => row.key === SURFACE_KEY)?.value ?? "";
  // "No document" is the shape under test, and `parseDocument("")` is how the
  // app spells it. A saved document hides the defect, which is exactly why the
  // failure looked route-specific and intermittent.
  await fetcher("/settings/user", { method: "PUT", body: { key: SURFACE_KEY, value: "" } });

  try {
    // A rail-link click is a `<Link>`, and a chord goes through the command
    // registry's `navigate`. A15 measured both as no-ops; both must land.
    await authedPage.goto("/tickets");
    await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
    await authedPage.locator('nav[aria-label="Main"] a[href="/clients"]').first().click();
    await expect(authedPage).toHaveURL(/\/clients(\?.*)?$/);

    await authedPage.goto("/tickets");
    await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
    await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
    await authedPage.keyboard.press("Alt+2");
    await expect(authedPage).toHaveURL(/\/clients(\?.*)?$/);

    // The control: the same chord from another route, which A15 measured
    // working. Without this leg a green run could hide a chord regression.
    await authedPage.goto("/assets");
    await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
    await authedPage.keyboard.press("Alt+2");
    await expect(authedPage).toHaveURL(/\/clients(\?.*)?$/);
  } finally {
    await fetcher("/settings/user", { method: "PUT", body: { key: SURFACE_KEY, value: previous } });
  }
});
