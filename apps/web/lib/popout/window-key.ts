// Window-scoped state keys — the per-window-key seam.
//
// Purpose: every piece of state Meridian persists per window (panel layout,
// ticket pods) is keyed by a window role inside popout windows so screen 2's
// arrangement is remembered per window ROLE ("ticket-detail", "dispatch-
// board"), never globally clobbering the main window's layout. The main
// window keeps the UNSUFFIXED keys — existing users keep their layouts.
//
// Desktop contract (lane 2): native windows pass the same route key here,
// so a native dispatch popout and a browser dispatch popout share one
// remembered arrangement.

/** The route key of THIS window when it is a popout, else null. */
export function currentPopoutKey(): string | null {
  if (typeof window === "undefined") return null;
  const flag = new URLSearchParams(window.location.search).get("popout");
  return flag !== null && flag !== "" ? flag : null;
}

/** The entity segment currently shown (e.g. "tickets/<id>") or null. */
export function currentShownEntity(): string | null {
  if (typeof window === "undefined") return null;
  const seg = window.location.pathname.split("/").filter(Boolean);
  if (seg.length < 2) return null;
  return `${seg[0]}/${seg[1]}`;
}

/** Window-scoped persistence key: `<base>@@<routeKey>` in popouts, `<base>`
 * in the main window (backward compatible). */
export function windowScopedKey(base: string): string {
  const popoutKey = currentPopoutKey();
  return popoutKey === null ? base : `${base}@@${popoutKey}`;
}
