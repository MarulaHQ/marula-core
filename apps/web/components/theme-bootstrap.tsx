"use client";
/**
 * ThemeBootstrap — the pre-paint resolution's recovery leg.
 *
 * The inline bootstrap script in the root layout's <head> resolves the theme
 * (stored preference, else the system's light/dark) and writes the kit's
 * attribute BEFORE first paint, so the login route never paints the light
 * baseline to a dark-mode visitor. That is the paint order of record.
 *
 * This component exists for what comes after. When React's hydration
 * recovery re-creates the <html> element on an authed route (hydration
 * error 418 recovery), the script-set attribute goes with it and the page
 * repaints light until something writes again — the wrong-mode flash this
 * lane kills. `useLayoutEffect` runs after hydration but BEFORE the browser
 * paints that DOM, so re-applying the resolved attribute here closes the gap
 * in the same frame: no wrong mode is ever on screen, no paint is delayed
 * (the script already resolved it), and no element moves — only the
 * attribute the CSS already selects on is restated.
 *
 * The attribute is written unconditionally, for the same reason the script
 * is: the twin writes exactly what the script would have written. React's
 * ordering makes the precedence safe on its own — layout effects flush
 * before passive effects, so this runs before AppearanceSync's account
 * re-apply lands, and a value that is already correct is a no-op write.
 *
 * Non-purpose: it never persists a preference (public entry screens inherit
 * the device's choice without writing it) and it never overrides the
 * account-saved mode — AppearanceSync applies that after the session
 * resolves, which is the precedence the theme doc records.
 */
import { useLayoutEffect } from "react";
import { applyResolvedAppearance } from "@marula-core/ui";

export function ThemeBootstrap(): null {
  useLayoutEffect(() => {
    applyResolvedAppearance();
  }, []);
  return null;
}
