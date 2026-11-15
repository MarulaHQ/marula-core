"use client";

// SurfaceSubnav — the tab row that keeps a demoted page reachable.
//
// Purpose: draw the tab set for the page you are on, in the surface pack's
// shape (the row its quotes page draws): 14px labels, 10px 2px padding, a 2px
// underline on the current tab, 22px gaps, no pill fills. It renders nothing
// when the pathname is not part of the set, so a page can mount it safely and
// a detail route (an invoice, a quote) still marks its parent tab.
//
// Non-purpose: no state, no fetching. Every tab is a link; the current page is
// the longest matching href, exactly like the rail.

import Link from "next/link";
import { usePathname } from "next/navigation";
import { SURFACE_SUBNAVS } from "../../lib/nav/surface-subnavs";

export function SurfaceSubnav({
  setId,
  className,
}: {
  setId?: string;
  className?: string;
}): React.JSX.Element | null {
  const pathname = usePathname();
  const sets = setId === undefined ? SURFACE_SUBNAVS : SURFACE_SUBNAVS.filter((s) => s.id === setId);

  for (const set of sets) {
    // Longest match wins: /billing/credit-notes must not light both tabs.
    let activeHref: string | null = null;
    for (const tab of set.tabs) {
      if (pathname !== tab.href && !pathname.startsWith(`${tab.href}/`)) continue;
      if (activeHref === null || tab.href.length > activeHref.length) activeHref = tab.href;
    }
    if (activeHref === null) continue;

    return (
      <nav aria-label={set.label} className={`m-subnav${className !== undefined ? ` ${className}` : ""}`}>
        {set.tabs.map((tab) => (
          <Link
            key={tab.href}
            href={tab.href}
            aria-current={tab.href === activeHref ? "page" : undefined}
          >
            {tab.label}
          </Link>
        ))}
      </nav>
    );
  }
  return null;
}
