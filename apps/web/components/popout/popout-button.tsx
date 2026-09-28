// PopoutButton — the in-context "pop out" affordance (Wave 38 lane 1).
//
// Purpose: opens the CURRENT route as a route-keyed popout window. Popup
// blockers never fail silently: a blocked open renders an honest inline
// hint (with the direct link) instead of a dead button. Inside a popout
// window the button hides — no popouts from popouts.

"use client";

import { useEffect, useState, type ReactNode } from "react";
import { openPopout } from "../../lib/popout/popout";
import { popoutRouteFor, paramsFor } from "../../lib/popout/registry";
import { currentPopoutKey } from "../../lib/popout/window-key";

export function PopoutButton({
  routeKey,
  params,
  iconOnly = false,
}: {
  /** Force a specific route key (defaults to the current route). */
  routeKey?: string;
  params?: Record<string, string>;
  iconOnly?: boolean;
}): ReactNode {
  const [hint, setHint] = useState<string | null>(null);
  const [current, setCurrent] = useState<{ key: string; params: Record<string, string> } | null>(null);

  useEffect(() => {
    if (currentPopoutKey() !== null) return;
    if (routeKey !== undefined) {
      setCurrent({ key: routeKey, params: params ?? {} });
      return;
    }
    const def = popoutRouteFor(window.location.pathname);
    if (def === null) return;
    setCurrent({ key: def.key, params: paramsFor(def, window.location.pathname) });
  }, [routeKey, params]);

  if (current === null) return null;

  const open = () => {
    const result = openPopout(current.key, current.params);
    if (result.ok) {
      setHint(null);
      return;
    }
    // Honest fallback: the browser blocked the popup — never silent.
    setHint(`${window.location.origin}${result.path}?popout=${current.key}`);
  };

  return (
    <span className="relative inline-flex items-center">
      <button
        type="button"
        data-testid="popout-open"
        aria-haspopup="dialog"
        onClick={open}
        className="flex h-7.5 shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 leading-none text-xs font-semibold"
        style={{
          borderColor: "var(--m-border)",
          background: "var(--m-surface)",
          color: "var(--m-text-secondary)",
        }}
        title="Pop out to its own window (for a second monitor)"
      >
        {iconOnly ? "⧉" : "⧉ Pop out"}
      </button>
      {hint !== null && (
        <span
          role="status"
          data-testid="popout-blocked-hint"
          className="absolute right-0 top-8.5 z-50 w-64 rounded-[var(--m-radius-control)] border p-2.5 text-xs leading-snug"
          style={{
            borderColor: "var(--m-border)",
            background: "var(--m-surface)",
            color: "var(--m-text-secondary)",
          }}
        >
          Your browser blocked the pop-up. Allow pop-ups for this site, or{" "}
          <a href={hint} target="_blank" rel="noreferrer" className="underline" style={{ color: "var(--m-accent)" }}>
            open the window directly
          </a>
          .
        </span>
      )}
    </span>
  );
}
