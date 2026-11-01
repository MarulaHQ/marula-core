// AppShell — the Wave 15 lane 6 shell grid.
//
// Purpose: one client wrapper composing the mockup paradigm: dark left icon
// rail (NavRail) + slim top bar (AppHeader) + scrolling content + the dark
// bottom work bar (TimerDock, in-flow — never a fixed overlay). Owns the
// rail-collapse persistence (localStorage) and the shell-level open-ticket
// count the rail badge renders.
//
// Non-purpose: no route state; headerless surfaces (auth flows, portal,
// shared links) render their own chrome — the rail, top bar, and work bar
// all hide there via isHeaderlessPath.

"use client";

import { usePathname } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { api, useAuth } from "../../lib/api";
import { AppHeader } from "../app-header";
import { TimerDock } from "../timer-dock";
import { isHeaderlessPath } from "./chrome";
import { DesktopUpdateStrip } from "./desktop-update-strip";
import { NavRail } from "./nav-rail";
import { PopoutShell } from "./popout-shell";
import { currentPopoutKey, currentShownEntity } from "../../lib/popout/window-key";
import { announceWindow, updateShownEntities } from "../../lib/popout/window-bus";
import { AppearanceSync } from "../appearance-sync";
import { TourScheduler } from "../help/tour-scheduler";
import { ShortcutLayer } from "../shortcuts/shortcut-layer";
import { LiveRegion } from "../a11y/live-region";


const RAIL_COLLAPSE_KEY = "marula-rail-collapsed";

/**
 * Open-ticket count for the rail badge: the three non-closed status
 * categories fetched through the typed SDK (100 each; ≥100 per category
 * renders "99+" honestly). Refreshes on mount, window focus, and a slow
 * interval; any failure leaves the badge hidden — never a fake zero.
 */
function useOpenTicketCount(enabled: boolean): number | null {
  const [count, setCount] = useState<number | null>(null);
  const load = useCallback(async () => {
    if (!enabled) return;
    const categories = ["open", "in_progress", "waiting"] as const;
    try {
      const results = await Promise.all(
        categories.map((category) =>
          api.api.GET("/api/v1/tickets", {
            params: { query: { category, limit: 100 } },
          }),
        ),
      );
      let total = 0;
      for (const result of results) {
        if (result.error !== undefined) {
          setCount(null);
          return;
        }
        total += result.data?.length ?? 0;
      }
      setCount(total);
    } catch {
      setCount(null);
    }
  }, [enabled]);
  useEffect(() => {
    if (!enabled) return;
    void load();
    const onFocus = () => {
      if (document.visibilityState === "visible") void load();
    };
    window.addEventListener("focus", onFocus);
    const timer = window.setInterval(onFocus, 60_000);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearInterval(timer);
    };
  }, [enabled, load]);
  return count;
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const auth = useAuth();
  const headerless = isHeaderlessPath(pathname);
  // Wave 38: popout windows (route-keyed) render the compact shell — no
  // nav rail, slim workbar. The flag comes from openPopout's URL.
  const [popoutKey, setPopoutKey] = useState<string | null>(null);
  useEffect(() => {
    setPopoutKey(currentPopoutKey());
  }, []);
  const openCount = useOpenTicketCount(
    auth.status === "authed" && !headerless && popoutKey === null,
  );
  const [collapsed, setCollapsed] = useState(false);

  // Window registry (Wave 38): the main window announces itself and keeps
  // its shown entity current, so notification routing can FOCUS the window
  // already showing a target instead of duplicating it.
  useEffect(() => {
    if (currentPopoutKey() !== null) return;
    announceWindow(null, [currentShownEntity()].filter((e): e is string => e !== null));
  }, []);
  useEffect(() => {
    if (currentPopoutKey() !== null) return;
    updateShownEntities([currentShownEntity()].filter((e): e is string => e !== null));
  }, [pathname]);

  useEffect(() => {
    try {
      // The pack draws a surface on the 64px collapsed rail (S1, and every
      // `web/*/states` sheet), so that is what a session with no saved choice
      // lands on. An explicit `"0"` still wins: a user who widened the rail
      // keeps their widened rail.
      setCollapsed(window.localStorage.getItem(RAIL_COLLAPSE_KEY) !== "0");
    } catch {
      // storage unavailable — default collapsed, matching the pack
    }
  }, []);

  const toggleCollapsed = useCallback(() => {
    setCollapsed((v) => {
      const next = !v;
      try {
        window.localStorage.setItem(RAIL_COLLAPSE_KEY, next ? "1" : "0");
      } catch {
        // storage unavailable — session-only state
      }
      return next;
    });
  }, []);

  // Popout windows render the compact shell AFTER all hooks (hook-order
  // safety): no rail, slim bar, slim workbar.
  if (popoutKey !== null) {
    return <PopoutShell authed={auth.status === "authed"}>{children}</PopoutShell>;
  }

  if (headerless) return (
    <>
      <AppearanceSync />
      {children}
    </>
  );

  return (
    // Two axes, not one. The top bar is the full width of the window and the
    // rail starts underneath it, which is the composition the mockup draws and
    // the one the kit documents for `TopBar` ("on every staff surface") beside
    // `AppRail` ("the shell owns the grid column"): the brand sits over the
    // rail's column, the search centred on the window, the dock cluster at the
    // right edge. Nesting the header inside the content column instead made it
    // start 45px in, so the bar stopped at the rail's edge and the brand sat
    // next to the first nav icon rather than above it.
    <div className="flex h-dvh min-h-0 flex-col">
      <AppearanceSync />
      {/* WCAG 2.4.1 bypass block: keyboard users skip the rail + top bar chrome. */}
      <a
        href="#m-main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:left-3 focus:top-3 focus:z-50 focus:rounded-[var(--m-radius-control)] focus:border focus:bg-[var(--m-surface)] focus:px-3 focus:py-2 focus:text-sm"
      >
        Skip to main content
      </a>
      <AppHeader />
      <div className="flex min-h-0 min-w-0 flex-1">
        <NavRail openCount={openCount} collapsed={collapsed} onToggleCollapsed={toggleCollapsed} />
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <DesktopUpdateStrip />
          <main id="m-main-content" className="min-h-0 flex-1 overflow-y-auto">{children}</main>
          <TimerDock />
        </div>
      </div>
      <TourScheduler />
      <ShortcutLayer authed={auth.status === "authed"} />
      <LiveRegion />
    </div>
  );
}
