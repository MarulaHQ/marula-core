// The entry screens' appearance, on a public page with no account yet.
//
// Purpose: the pack ships an auth DARK column
// (`auth/dark/states/01-login.png` and the rest), so the entry screens have to
// be able to render dark. Forcing them light would make half the pack
// unreachable and would ignore somebody's device.
//
// WHY NOT `applyTheme`. Two reasons, and the second is the important one:
// 1. It always writes the choice to localStorage. On a sign-in screen there is
//    no account to attach a preference to, and writing one means the next
//    person to use the machine inherits a theme they did not pick — which is
//    exactly the shared-device case these public pages exist to protect.
// 2. It takes a concrete value, so the caller has to decide light or dark
//    rather than deferring to the device.
//
// So this sets the SAME `data-theme` attribute `applyTheme` sets, resolves the
// DEVICE's preference, and persists nothing. The staff app's own preference
// resolution (`resolveInitialTheme`) is untouched: a signed-in user still gets
// their stored choice, and a public page never writes over it.
"use client";

/** Follows the operating system's light/dark setting, and stores nothing. */
export function applyEntryTheme(): void {
  if (typeof document === "undefined") return;
  let dark = false;
  if (typeof window !== "undefined" && window.matchMedia) {
    dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  }
  document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
}