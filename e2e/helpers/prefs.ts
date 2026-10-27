// Per-user surface preferences, for flow authors.
//
// WHY THIS EXISTS (Wave 62 lane `p62-gate-truth`, closing Wave 47 A26's
// follow-up). A default tab, filter or density is not spec state: it is a
// SAVED user preference (`appearance.data_surfaces`,
// apps/web/lib/data-surface-prefs.ts) that the page restores on mount. A case
// that ends on "Approved" therefore changes what the NEXT case sees, and the
// case order differs between the light and dark projects. `05-approvals` went
// red on exactly that. Resetting per case is the fix; doing it in the fixture
// means a new flow cannot reintroduce the flake by forgetting.
import { authedFetch } from "./api";

/** The key the saved surface document lives under (`/settings/user`). */
export const SURFACE_PREFS_KEY = "appearance.data_surfaces";

/**
 * Put the saved surface views back to their fresh-account state.
 *
 * An empty document is what a fresh account has: `parseDocument` drops entries
 * without `columns`, so every surface falls back to its own default. Await it
 * before `goto`, so the page's preferences read cannot return the value this
 * call is about to replace.
 */
export async function resetSurfacePrefs(token: string): Promise<void> {
  await authedFetch(token)("/settings/user", {
    method: "PUT",
    body: { key: SURFACE_PREFS_KEY, value: "{}" },
  });
}
