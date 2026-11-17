// Tour surfaces (Wave 72).
//
// Why this exists: a tour used to be selected by the docs route key
// (`routeKeyForPath`). That key is deliberately coarse — a record page
// inherits its list's key because they share documentation — so
// /projects/<id> resolved to "projects", the list tour's targets were
// never on a record page, and no tour could start there. `/stock` was
// worse: it had no key at all, so its tour rode the "default" fallback
// and belonged to every unmapped path in the product.
//
// A TOUR SURFACE is the unit a tour is authored against: the list key
// where that is already precise, a finer key where a page has its own
// grammar. Docs keep using routeKeyForPath untouched, so the help panel
// still answers with the list's documentation on a record page.
//
// Rules are ordered and first-match-wins. Record patterns come before
// the list's own key can claim the path, and static child routes are
// excluded by name so /projects/portfolio stays a list, not a record.

import { routeKeyForPath } from "../help/route-keys";

export const TOUR_SURFACE_RULES: readonly (readonly [RegExp, string])[] = [
  // Projects: the engagement record and its own document pages, but not
  // the static sub-pages that are lists in their own right.
  [
    /^\/projects\/(?!portfolio(?:\/|$)|my-work(?:\/|$)|templates(?:\/|$))[^/]+/,
    "project-record",
  ],
  // Tickets: the workbench record. `/tickets/classic/<id>` is a DIFFERENT
  // record with its own markup, so it is deliberately not matched here;
  // it keeps the "tickets" key and the target check declines the tour.
  [/^\/tickets\/(?!classic(?:\/|$))[^/]+/, "ticket-record"],
  // The merge workbench is the review surface; /inventory is its queue.
  [/^\/inventory\/workbench(?:\/|$)/, "inventory-workbench"],
  // Stock had no route key, so its tour borrowed "default". A surface of
  // its own is enough; the docs fallback stays "default" as before.
  [/^\/stock(?:\/|$)/, "stock"],
  // Purchasing has no route key either, so orders and vendors would
  // otherwise sit in the fallback bucket.
  [/^\/purchasing(?:\/|$)/, "purchasing"],
];

/**
 * The tour surface for a pathname: the first matching rule, else the
 * docs route key (list surfaces have always used that and still do).
 */
export function tourSurfaceForPath(pathname: string): string {
  for (const [pattern, surface] of TOUR_SURFACE_RULES) {
    if (pattern.test(pathname)) return surface;
  }
  return routeKeyForPath(pathname);
}
