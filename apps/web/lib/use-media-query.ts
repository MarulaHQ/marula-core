"use client";

// A media query as React state, for the places where a breakpoint has to change
// WHICH elements exist rather than how they look.
//
// Purpose: the ticket ledger is a `table-layout: fixed` table whose columns are
// percentage shares, so CSS alone cannot drop a column — the `<col>` and both
// its cells have to leave the DOM together, and the remaining shares have to be
// recomputed without them. A class that hides a cell would leave the column's
// width behind, which is the defect this exists to fix.
//
// Non-purpose: layout that Tailwind's breakpoint variants already express. Use
// `hidden xl:block` for a panel; reach for this only when the structure differs.

import { useEffect, useState } from "react";

/** The viewport matches `query`, updated on resize. False before mount, so the
 *  server and the first client render agree. */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false);

  useEffect(() => {
    const list = window.matchMedia(query);
    const apply = () => setMatches(list.matches);
    apply();
    list.addEventListener("change", apply);
    return () => list.removeEventListener("change", apply);
  }, [query]);

  return matches;
}
