// Headerless chrome matcher (Wave 15 lane 6).
//
// Surfaces that own their chrome (auth flows, the client portal, shared
// links) render no staff shell: the rail, the slim top bar, and the work
// bar all consult this one list so they can never disagree.

export const HEADERLESS_PATTERNS: RegExp[] = [
  /^\/login$/,
  /^\/mfa$/,
  /^\/sso$/,
  /^\/welcome$/,
  /^\/portal(\/|$)/,
  /^\/s\//,
  /^\/accept-invite/,
];

export function isHeaderlessPath(pathname: string): boolean {
  return HEADERLESS_PATTERNS.some((pattern) => pattern.test(pathname));
}
