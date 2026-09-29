// Popout opening + mode detection.
//
// Purpose: `openPopout(routeKey, params)` — window.open with the registry's
// feature string, the `?popout=<routeKey>` flag, and an honest popup-blocker
// outcome (null window → { ok: false, blocked: true }; callers render the
// inline hint — NEVER silent failure). The opener also records the child in
// `sessionStorage` (`marula-popouts`) so a reloaded main window can still
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
import { openNativeWindow } from "../desktop/windows";
import { isDesktopShell } from "../desktop/shell";

export type OpenPopoutResult =
  | { ok: true; windowKey: string; window: Window | null }
  | { ok: false; blocked: true; routeKey: string; path: string };

/** Where `openPopout` records its children (same-tab session scope). */
const CHILDREN_KEY = "marula-popouts";

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
  // Desktop shell (Wave 38 lane 2): the SAME affordance becomes a native
  // OS window — same route key, same compact shell, per-role remembered
  // geometry. Fire-and-honest: the sync return keeps every caller's
  // contract; a rare factory failure is console-reported (support reality,
  // same posture as the desktop update check) rather than an inline hint,
  // because the normal outcome is a window on screen, not a blocker.
  if (isDesktopShell()) {
    const pathSuffix = path.replace(/^\//, "").replace(/\?.*$/, "");
    void openNativeWindow(routeKey, pathSuffix).then((result) => {
      if (result !== null && !result.ok) {
        console.warn(
          `native window for ${routeKey} failed to open (factory returned an error)`,
        );
      }
    });
    return { ok: true, windowKey: `${routeKey}:${params.id ?? ""}`, window: null };
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
