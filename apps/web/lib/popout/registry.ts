// Route-keyed popout registry — the ONE source of truth for which routes
// may open in a secondary browser window, what each window role is called,
// and how it is sized.
//
// Purpose: `window.open` multi-window for the browser (Wave 38 lane 1).
// Every popout window carries `?popout=<routeKey>` so the shell can render
// its COMPACT variant (no nav rail, slim workbar) and persist panel layout
// per window key — screen 2's arrangement is remembered per window role,
// never clobbering the main window's layout.
//
// Desktop contract (lane 2 handoff): the route keys here are the SAME keys
// the Tauri shell uses for native multi-window, and `pathOf` builds the
// canonical in-app URL for any key + params. The `marula://` deep-link
// grammar (crates/core-domain labels.rs `deep_link`) maps onto these keys
// via `kindForPath` — one grammar, two shells.

/** Param extraction per route key: pulls the dynamic segment from a real
 * pathname so a popout opened from "/tickets/<id>" is keyed per ticket. */
export interface PopoutRouteDef {
  /** Stable window role, e.g. "ticket-detail" — the layout persistence key. */
  key: string;
  /** Path template with one dynamic segment, e.g. "/tickets/[id]". */
  template: string;
  /** Compact-shell title (shown in the slim bar; also the window title). */
  title: string;
  /** Feature-string sizing for window.open. */
  width: number;
  height: number;
}

export const POPOUT_ROUTES: ReadonlyArray<PopoutRouteDef> = [
  { key: "ticket-workspace", template: "/tickets/classic/[id]", title: "Ticket workspace", width: 1280, height: 860 },
  { key: "ticket-detail", template: "/tickets/[id]", title: "Ticket", width: 1100, height: 820 },
  { key: "dispatch-board", template: "/dispatch", title: "Dispatch board", width: 1440, height: 900 },
  { key: "stock-lookup", template: "/stock", title: "Stock lookup", width: 960, height: 720 },
  { key: "decisions", template: "/approvals", title: "Decisions", width: 1100, height: 800 },
  { key: "merge-workbench", template: "/tickets/[id]", title: "Merge workbench", width: 1200, height: 840 },
];

/** True when `pathname` matches the template (one `[seg]` = one non-empty
 * segment, any value). Same matching rules as the shortcut registry. */
export function popoutRouteFor(pathname: string): PopoutRouteDef | null {
  for (const def of POPOUT_ROUTES) {
    const t = def.template.split("/").filter(Boolean);
    const p = pathname.split("/").filter(Boolean);
    if (t.length !== p.length) continue;
    let ok = true;
    for (let i = 0; i < t.length; i += 1) {
      const seg = t[i] ?? "";
      const actual = p[i] ?? "";
      if (seg !== actual && !seg.startsWith("[")) { ok = false; break; }
    }
    if (ok) return def;
  }
  return null;
}

/** Extract the dynamic segment value from a concrete pathname. */
export function paramsFor(def: PopoutRouteDef, pathname: string): Record<string, string> {
  const t = def.template.split("/").filter(Boolean);
  const p = pathname.split("/").filter(Boolean);
  const params: Record<string, string> = {};
  for (let i = 0; i < t.length; i += 1) {
    const seg = t[i] ?? "";
    if (seg.startsWith("[")) params[seg.slice(1, -1)] = p[i] ?? "";
  }
  return params;
}

/** Canonical in-app URL for a route key + params (desktop lane 2 shares this). */
export function pathOf(key: string, params: Record<string, string> = {}): string | null {
  const def = POPOUT_ROUTES.find((d) => d.key === key);
  if (def === undefined) return null;
  return def.template.replace(/\[([a-z_]+)\]/g, (_, name: string) => params[name] ?? "");
}

/** The route key a `marula://<kind>/<id>` deep link opens as a popout.
 * Mirrors deepLinkTarget (apps/web/lib/labels/model.ts). */
export function popoutKeyForDeepLinkKind(kind: string): string | null {
  switch (kind) {
    case "ticket": return "ticket-detail";
    case "board": return "dispatch-board";
    case "stock-item": return "stock-lookup";
    default: return null;
  }
}

/** True inside a popout window (query flag set by openPopout). */
export function isPopoutWindow(search: string): string | null {
  const flag = new URLSearchParams(search).get("popout");
  return flag !== null && flag !== "" ? flag : null;
}

export function featureString(def: PopoutRouteDef): string {
  const w = Math.min(def.width, Math.max(360, (typeof window !== "undefined" ? window.screen.availWidth : def.width) - 40));
  const h = Math.min(def.height, Math.max(320, (typeof window !== "undefined" ? window.screen.availHeight : def.height) - 80));
  // No `noopener`: it nulls the return value (which the popup-blocker
  // detection depends on) and breaks the immediate focus() handoff.
  return `popup=yes,width=${w},height=${h}`;
}
