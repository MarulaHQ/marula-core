"use client";
/**
 * ToastHost — the client boundary that owns the toast surface.
 *
 * Purpose: exist so the ROOT LAYOUT never imports the `@marula-core/ui` barrel.
 * The barrel re-exports with `export *`, and Next 15 refuses an `export *` in a
 * module that sits on a client boundary reached from a server component — so a
 * server layout importing a client component straight out of the barrel fails
 * the build. Routing the import through this `"use client"` module makes the
 * boundary explicit and keeps the barrel's shape out of the layout's build
 * path.
 *
 * Non-purpose: it is not the app shell and it renders nothing else. The shell
 * is conditional (popout and headerless layouts skip it) and a toast that
 * cannot appear on a page is worse than no toast, so the host lives here
 * instead, next to the one route that always renders.
 */
import { Toaster } from "@marula-core/ui";

export function ToastHost() {
  return <Toaster />;
}
