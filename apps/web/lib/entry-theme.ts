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
// So this delegates to the SHARED pre-paint resolution in
// `@marula-core/ui` — the same stored-then-system resolution and the same KIT
// attribute literal (`neutral` for the app's light) the layout's inline
// bootstrap writes, because a public page inheriting the machine's stored
// choice leaks nothing (only a WRITE would leak a preference). This file used
// to resolve the system preference alone and write the app's own `light`
// literal, which the kit reads as its CREAM block: every entry screen on a
// light machine painted cream, and a visitor whose stored preference was dark
// had it overridden on the door. Writing `light` here is what broke that.
"use client";

import { applyResolvedAppearance } from "@marula-core/ui";

/** Follows the same resolution the pre-paint bootstrap uses, and stores nothing. */
export function applyEntryTheme(): void {
  applyResolvedAppearance();
}
