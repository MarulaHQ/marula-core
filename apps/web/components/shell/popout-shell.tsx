// PopoutShell — the COMPACT window chrome for route-keyed popout windows.
//
// Purpose (Wave 38 lane 1): a popout window renders NO nav rail and a slim
// top bar (route-key title, back-to-main link, close) plus a slim workbar
// (TimerDock, in-flow). Everything keyboard-first stays: ShortcutLayer
// (shortcuts work per window), LiveRegion, AppearanceSync. Lifecycle
// honesty lives here too: the bye announcement + presence leave fire on
// unload, and the dirty guard engages beforeunload while unsaved edits
// exist (save-bar semantics, never silent data loss).

"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect, type ReactNode } from "react";
import { TimerDock } from "../timer-dock";
import { ShortcutLayer } from "../shortcuts/shortcut-layer";
import { LiveRegion } from "../a11y/live-region";
import { AppearanceSync } from "../appearance-sync";
import { currentPopoutKey, currentShownEntity } from "../../lib/popout/window-key";
import { leaveWindow, announceWindow, updateShownEntities, onFocusRequest } from "../../lib/popout/window-bus";
import { POPOUT_ROUTES } from "../../lib/popout/registry";

/** True while unsaved edits exist in this window; the beforeunload guard
 * reads it. Save bars register/unregister through setPopoutDirty. */
let popoutDirty = false;
export function setPopoutDirty(dirty: boolean): void {
  popoutDirty = dirty;
}

/** Read the flag (the desktop updater's restart survey asks every window;
 * the strip in each window answers with this). */
export function popoutDirtyFlag(): boolean {
  return popoutDirty;
}

function onBeforeUnload(event: BeforeUnloadEvent): void {
  if (!popoutDirty) return;
  event.preventDefault();
  // Legacy requirement for Chrome/Edge to show the confirmation.
  event.returnValue = "";
}

function leaveBeacon(): void {
  // Server-tolerant cleanup: the heartbeat simply stops (90s TTL expires
  // the entry); the bus registry is updated synchronously so OTHER windows
  // never route notifications to a closing window.
  const popoutKey = currentPopoutKey();
  if (popoutKey !== null) leaveWindow(popoutKey);
}

export function PopoutShell({
  children,
  authed,
}: {
  children: ReactNode;
  authed: boolean;
}) {
  const pathname = usePathname();
  const router = useRouter();
  const popoutKey = currentPopoutKey();
  const def = POPOUT_ROUTES.find((d) => d.key === popoutKey) ?? null;

  // Dock routing: focus requests aimed at this window navigate it to the
  // target entity (the window is already on screen — focus, not duplicate).
  useEffect(() => {
    return onFocusRequest((path) => router.push(path));
  }, [router]);

  useEffect(() => {
    announceWindow(popoutKey, [currentShownEntity()].filter((e): e is string => e !== null));
    return leaveBeacon;
  }, [popoutKey]);

  useEffect(() => {
    updateShownEntities([currentShownEntity()].filter((e): e is string => e !== null));
    document.title = def !== null ? `${def.title} — Marula` : "Marula";
  }, [pathname, def]);

  useEffect(() => {
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, []);

  return (
    <div className="flex h-dvh min-h-0 flex-col">
      <AppearanceSync />
      <header
        data-testid="popout-bar"
        className="flex h-9 shrink-0 items-center gap-2 border-b px-3"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface-sunken, var(--m-surface))" }}
      >
        <span className="text-xs uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          {def?.title ?? "Marula"}
        </span>
        <span className="ml-auto flex items-center gap-2">
          <a
            href="/"
            className="text-xs hover:underline"
            style={{ color: "var(--m-text-muted)" }}
          >
            Main window
          </a>
        </span>
      </header>
      <main className="min-h-0 flex-1 overflow-y-auto">{children}</main>
      <TimerDock />
      <ShortcutLayer authed={authed} />
      <LiveRegion />
    </div>
  );
}
