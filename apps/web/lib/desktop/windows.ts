// Desktop-native window bridge — lane 2 of Wave 38.
//
// Purpose: inside the Tauri shell, every lane-1 popout affordance becomes a
// REAL OS window (same route keys, same compact shell, per-role remembered
// geometry) instead of a browser popup. One seam: `openPopout` asks this
// module's opener when `isDesktopShell()` is true; plain-web behavior is
// untouched (window.open + feature string, popup-blocker honest hint).
//
// Non-purpose: no window chrome simulation — the OS draws the frame; the
// renderer supplies only the compact shell content. Geometry persistence
// lives shell-side (move/resize events), not here.

import { isDesktopShell, tauriInvoke } from "./shell";

/** The label the window factory minted for THIS window
 * ("subwindow::<role>::<n>"), or null in the main window / plain web. */
export function currentNativeWindowLabel(): string | null {
  if (typeof window === "undefined") return null;
  const wid = new URLSearchParams(window.location.search).get("wid");
  return wid !== null && wid !== "" ? wid : null;
}

/** Open a route-keyed window through the native factory. Returns null when
 * not in the desktop shell (caller falls back to the browser path); a shell
 * error surfaces as { ok: false } — never a fake success. */
export async function openNativeWindow(
  routeKey: string,
  pathSuffix: string,
): Promise<{ ok: boolean; label: string | null } | null> {
  if (!isDesktopShell()) return null;
  try {
    const label = (await tauriInvoke("open_native_window", {
      role: routeKey,
      pathSuffix,
    })) as string | null;
    return { ok: true, label };
  } catch (error) {
    console.warn("native window open failed:", error);
    return { ok: false, label: null };
  }
}

/** Ask the shell to open-or-FOCUS the window a meridian:// link targets.
 * Returns the acted-on label, or null when the shell is absent. */
export async function routeMeridianLink(url: string): Promise<string | null> {
  if (!isDesktopShell()) return null;
  try {
    return (await tauriInvoke("route_meridian_link", { url })) as string;
  } catch (error) {
    console.warn("meridian link routing failed:", error);
    return null;
  }
}
