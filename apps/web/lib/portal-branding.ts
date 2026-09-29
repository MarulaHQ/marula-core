// Portal white-label-lite branding (Wave 17 lane 10).
//
// Purpose: ONE substitution point for client-facing portal branding — org
// display name and org accent color — consumed by the /portal shell, the
// portal login cover, and the public scheduling surface. HONEST DEFAULT
// POSTURE: both values are deployment configuration; in the default build
// neither is set, every surface falls back to the Marula tokens, and no
// UI invents a brand (no fake logo uploads, no fabricated preview).
//
// Non-purpose: NOT a theming engine — the name substitutes text only and
// the accent substitutes the portal's interactive tint; everything else
// (surfaces, status colors, typography, motion) stays the governing token
// set in docs/theme.md. A per-org branding CONTRACT (org display name +
// accent as API fields) does not exist in the pinned OpenAPI — deeper
// white-label stays filed in TASKLIST for a backend lane.
//
// Hex note: source-level hexes never appear here — values arrive at runtime
// from env config, and the empty string means "no override".

const ENV_ORG_NAME = (process.env.NEXT_PUBLIC_ORG_NAME ?? "").trim();
const ENV_ORG_COLOR = (process.env.NEXT_PUBLIC_ORG_COLOR ?? "").trim();

/** The (honest) portal brand name; null ⇒ keep the Marula cover text. */
export function portalOrgName(): string | null {
  return ENV_ORG_NAME !== "" ? ENV_ORG_NAME : null;
}

/**
 * The (validated) org accent hex, or null when unset/invalid — the default
 * posture with no substitution: surfaces keep the Tangerine token accent.
 */
export function portalOrgAccent(): string | null {
  return /^#[0-9a-f]{6}$/i.test(ENV_ORG_COLOR) ? ENV_ORG_COLOR.toLowerCase() : null;
}

/**
 * CSS override map for the accent substitution: the org color replaces the
 * portal's interactive tint (--m-accent) with its soft surface kept
 * token-based at reduced opacity (color-mix, so the pairing tracks both
 * modes without a second env value).
 */
export function portalAccentOverrides(): Record<string, string> | undefined {
  const accent = portalOrgAccent();
  if (accent === null) return undefined;
  return {
    "--m-accent": accent,
    "--m-accent-soft": `color-mix(in oklab, ${accent} 12%, transparent)`,
  };
}
