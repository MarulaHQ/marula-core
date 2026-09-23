// Route-key derivation for contextual help (Wave 26 lane 2).
//
// Purpose: one ordered prefix map turning the current pathname into a
// ROUTE KEY — the same key vocabulary the docs pipeline's help manifest
// (docs/site/help-manifest.json, copied to ./help-manifest.json) uses to
// point a surface at its canonical doc pages. Ordered longest-prefix
// matching: settings subsections before the /settings fallback.

export const ROUTE_KEY_RULES: readonly (readonly [RegExp, string])[] = [
  [/^\/today/, "today"],
  [/^\/tickets/, "tickets"],
  [/^\/boards/, "boards"],
  [/^\/dispatch/, "dispatch"],
  [/^\/clients/, "clients"],
  [/^\/inventory/, "inventory"],
  [/^\/billing/, "billing"],
  [/^\/invoices/, "billing"],
  [/^\/credit-notes/, "billing"],
  [/^\/contracts/, "billing"],
  [/^\/sales/, "sales"],
  [/^\/pipeline/, "sales"],
  [/^\/quotes/, "sales"],
  [/^\/catalog/, "sales"],
  [/^\/approvals/, "approvals"],
  [/^\/calendar/, "calendar"],
  [/^\/projects/, "projects"],
  [/^\/dashboards/, "dashboards"],
  [/^\/reports/, "reports"],
  [/^\/scorecards/, "scorecards"],
  [/^\/search/, "search"],
  [/^\/assets/, "assets"],
  [/^\/documents/, "documents"],
  [/^\/provenance/, "provenance"],
  [/^\/audit/, "audit"],
  [/^\/kb/, "kb"],
  [/^\/alerts/, "automations"],
  [/^\/automations/, "automations"],
  [/^\/imports/, "imports"],
  [/^\/ai/, "ai"],
  [/^\/agent-platform/, "agent-platform"],
  [/^\/portal/, "portal"],
  [/^\/settings\/onboarding/, "settings-onboarding"],
  [/^\/settings\/preferences/, "settings-preferences"],
  [/^\/settings\/team/, "settings-team"],
  [/^\/settings\/api-keys/, "settings-api-keys"],
  [/^\/settings\/integrations/, "settings-integrations"],
  [/^\/settings\/plugins/, "settings-integrations"],
  [/^\/settings\/fx/, "settings-fx"],
  [/^\/settings\/portal/, "settings-portal"],
  [/^\/settings\/security/, "settings-security"],
  [/^\/settings\/sla/, "settings-sla"],
  [/^\/settings\/workflow/, "settings-workflow"],
  [/^\/settings\/boards/, "settings-workflow"],
  [/^\/settings\/taxonomy/, "settings-workflow"],
  [/^\/settings\/inventory/, "inventory"],
  [/^\/settings/, "settings-general"],
];

/** The fallback key — every surface without a specific mapping lands here. */
export const DEFAULT_ROUTE_KEY = "default";

/** Longest-applicable rule wins; unmatched paths fall to `default`. */
export function routeKeyForPath(pathname: string): string {
  for (const [pattern, key] of ROUTE_KEY_RULES) {
    if (pattern.test(pathname)) return key;
  }
  return DEFAULT_ROUTE_KEY;
}
