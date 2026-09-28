// Popout opening + mode detection.
//
// Purpose: `openPopout(routeKey, params)` — window.open with the registry's
// feature string, the `?popout=<routeKey>` flag, and an honest popup-blocker
// outcome (null window → { ok: false, blocked: true }; callers render the
// inline hint — NEVER silent failure). The opener also records the child in
// `sessionStorage` (`meridian-popouts`) so a reloaded main window can still
// reason about its popouts; each popout announces itself on the window bus
// so ALL windows share the registry.
//
// Desktop lane 2 consumes the same registry keys and `pathOf` URLs; the
// `openPopout` call site is the only browser-specific piece.

import {
  featureString,
  pathOf,
  POPOUT_ROUTES,
} from "./registry";
import { announceWindow } from "./window-bus";

export type OpenPopoutResult =
  | { ok: true; windowKey: string; window: Window | null }
  | { ok: false; blocked: true; routeKey: string; path: string };

/** Where `openPopout` records its children (same-tab session scope). */
const CHILDREN_KEY = "meridian-popouts";

function rememberChild(routeKey: string, params: Record<string, string>): void {
  try {
    const raw = window.sessionStorage.getItem(CHILDREN_KEY);
    const list: Array<{ routeKey: string; params: Record<string, string> }> =
      raw === null ? [] : (JSON.parse(raw) as typeof list);
    list.push({ routeKey, params });
    window.sessionStorage.setItem(CHILDREN_KEY, JSON.stringify(list.slice(-20)));
  } catch {
    // storage unavailable — the bus registry still carries the window
  }
}

export function openPopout(
  routeKey: string,
  params: Record<string, string> = {},
): OpenPopoutResult {
  if (typeof window === "undefined") {
    return { ok: false, blocked: true, routeKey, path: routeKey };
  }
  const path = pathOf(routeKey, params);
  if (path === null) {
    return { ok: false, blocked: true, routeKey, path: routeKey };
  }
  const def = POPOUT_ROUTES.find((d) => d.key === routeKey) ?? null;
  const url = `${path}?popout=${encodeURIComponent(routeKey)}`;
  const features = featureString(
    def ?? { key: routeKey, template: path, title: "Meridian", width: 1100, height: 820 },
  );
  const child = window.open(url, `meridian-${routeKey}`, features);
  // Popup blocker (or synchronous navigation failure): return null window —
  // surfaced as an honest inline hint by the caller.
  if (child === null) {
    return { ok: false, blocked: true, routeKey, path };
  }
  rememberChild(routeKey, params);
  child.focus();
  return { ok: true, windowKey: `${routeKey}:${params.id ?? ""}`, window: child };
}

/** Called from inside a popout window once it knows its route key + the
 * entities it shows; announces on the bus. */
export function initPopoutIdentity(routeKey: string, entities: string[]): void {
  announceWindow(routeKey, entities);
}
