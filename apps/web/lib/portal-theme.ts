// The portal's OWN theme, per person, scoped to the portal and nothing else.
//
// Purpose: the client portal is a different surface for a different audience
// (DESIGN-NOTES-portal.md §1), so it carries its own two themes — sand (warm
// cream, the client-facing default) and dark. This module owns the persisted
// per-person choice and the DOM attribute that scopes it.
//
// SCOPING IS THE WHOLE POINT. The staff app's mode lives on
// `document.documentElement[data-theme]`, and this module never writes there.
// It sets `data-portal-theme` on the portal's own root element, and
// `app/portal/portal.css` redeclares the semantic tokens against that
// attribute alone. A client who picks Dark in the portal still walks into a
// light staff app (and the reverse), which is what
// `overview/portal-theme-separation.png` draws and what the account menu
// promises in words: "This changes the portal for you only."
//
// Non-purpose: this is not a theme engine and it does not write a hex. The
// two columns are declared once, in portal.css, from the pack's `tokens.json`
// sand and dark columns, which match the landed `--m-*` values exactly.
"use client";

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** The portal's two themes. Sand is the client-facing default. */
export type PortalTheme = "sand" | "dark";

export const PORTAL_THEMES: readonly PortalTheme[] = ["sand", "dark"];

export const PORTAL_THEME_ATTRIBUTE = "data-portal-theme";
export const PORTAL_THEME_STORAGE_KEY = "marula-portal-theme";

/** What the server renders, and what a first-time visitor sees. */
export const PORTAL_THEME_DEFAULT: PortalTheme = "sand";

function isPortalTheme(value: unknown): value is PortalTheme {
  return value === "sand" || value === "dark";
}

function readStoredTheme(): PortalTheme {
  if (typeof window === "undefined") return PORTAL_THEME_DEFAULT;
  try {
    const stored = window.localStorage.getItem(PORTAL_THEME_STORAGE_KEY);
    if (isPortalTheme(stored)) return stored;
  } catch {
    // Storage unavailable (private mode): the default still applies.
  }
  return PORTAL_THEME_DEFAULT;
}

export function usePortalTheme() {
  // Sand on the server, so the first paint is the client-facing default and
  // never the staff app's mode. A stored Dark is applied in the layout effect
  // below, which runs before paint, so a dark portal never flashes sand.
  const [theme, setTheme] = useState<PortalTheme>(PORTAL_THEME_DEFAULT);
  const rootRef = useRef<HTMLDivElement | null>(null);

  const apply = useCallback((next: PortalTheme) => {
    rootRef.current?.setAttribute(PORTAL_THEME_ATTRIBUTE, next);
  }, []);

  useLayoutEffect(() => {
    const stored = readStoredTheme();
    rootRef.current?.setAttribute(PORTAL_THEME_ATTRIBUTE, stored);
    setTheme(stored);
  }, []);

  useEffect(() => {
    apply(theme);
  }, [apply, theme]);

  const choose = useCallback(
    (next: PortalTheme) => {
      setTheme(next);
      apply(next);
      try {
        window.localStorage.setItem(PORTAL_THEME_STORAGE_KEY, next);
      } catch {
        // Storage unavailable; the attribute still applies for the session.
      }
    },
    [apply],
  );

  return { theme, choose, rootRef };
}
