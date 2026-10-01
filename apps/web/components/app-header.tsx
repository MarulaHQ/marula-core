// AppHeader — the SLIM top bar (Wave 15 lane 6, mockup paradigm).
//
// Purpose: brand mark, the unified-search entry pill (⌘K), connection
// status (riding the existing ConnectionBanner state + the IF6 outbox
// dock), notifications, theme toggle, identity menu with sign out. The
// old grouped page nav (MainNav) moved to the left icon rail (NavRail) —
// this bar is chrome only. Chrome-quiet per docs/theme.md — tokens only,
// one hairline border, no marketing surface.
//
// Non-purpose: no navigation state of its own; the search page owns
// results; notification preferences live at /settings/notifications.
"use client";

import { useTheme } from "@marula-core/ui";
import { Icon, TopBar } from "@marula-core/ui/kit";
import type { Theme } from "@marula-core/ui";
import { Button, Kbd } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { API_BASE, useAuth } from "../lib/api";
import { BrandLockup, BrandMark, LOCKUP_COMPLIANT_HEIGHT } from "./brand";
import { NotificationDock } from "./notification-dock";
import { HelpPanel } from "./help/help-panel";
import { OutboxDock } from "./instant/outbox-dock";
import { ConnectionBanner } from "./instant/connection-banner";
import { isHeaderlessPath } from "./shell/chrome";

function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  const dark = theme === "dark";
  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={() => setTheme(dark ? "light" : "dark")}
      aria-label={`Switch to ${dark ? "light" : "dark"} mode`}
      className="px-2"
    >
      {dark ? (
        <Icon name="sun" size={15} />
      ) : (
        <Icon name="moon" size={15} />
      )}
    </Button>
  );
}

/**
 * Unified-search entry: a quiet pill that reads like the mockup's search
 * field (placeholder + ⌘K) and jumps to /search. Results live on the
 * search page. The global ⌘K / Ctrl+K chord now rides the Wave 29 command
 * registry (components/shortcuts) — one dispatcher, no duplicate listener.
 */
function SearchLauncher() {
  const router = useRouter();
  return (
    <button
      type="button"
      data-testid="search-launcher"
      onClick={() => router.push("/search")}
      aria-label="Search (Command K)"
      className="m-top-search flex h-[var(--m-density-control)] min-w-0 max-w-[420px] flex-1 cursor-pointer items-center gap-2 rounded-lg border px-3 text-left"
      style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}
    >
      <Icon name="search" size={13} className="shrink-0" />
      <span
        className="min-w-0 flex-1 truncate text-[12.5px] max-md:hidden"
        style={{ color: "var(--m-text-muted)" }}
      >
        Search tickets, clients, devices…
      </span>
      <span
        className="md:hidden min-w-0 flex-1 truncate text-[12.5px]"
        style={{ color: "var(--m-text-muted)" }}
      >
        Search
      </span>
      <span className="hidden shrink-0 md:inline">
        <Kbd>⌘K</Kbd>
      </span>
    </button>
  );
}

/**
 * Dev-tools entry (TASKLIST DX3): ONE probe call to GET /api/dev/status on
 * mount; the entry renders ONLY when the API answers `enabled: true` —
 * which it only does with MARULA_DEV_TOOLS=1. Any failure (404, network,
 * non-JSON) silently hides it: no dev surface by default.
 */
function DevToolsLink({ onNavigate }: { onNavigate?: () => void }) {
  const router = useRouter();
  const [enabled, setEnabled] = useState(false);
  useEffect(() => {
    let cancelled = false;
    fetch(`${API_BASE}/api/dev/status`)
      .then(async (response) => {
        if (!response.ok || cancelled) return null;
        return (await response.json()) as { enabled?: boolean };
      })
      .then((body) => {
        if (!cancelled && body?.enabled === true) setEnabled(true);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);
  if (!enabled) return null;
  return (
    <Button variant="ghost" size="sm" onClick={() => { onNavigate?.(); router.push("/dev"); }}>
      Dev tools
    </Button>
  );
}

function initialsOf(name: string | undefined): string {
  const parts = (name ?? "").trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "M";
  return parts.slice(0, 2).map((p) => p[0]?.toUpperCase() ?? "").join("");
}

/** Identity menu: avatar + name, sign out, and (when enabled) dev tools. */
function UserMenu() {
  const { me, logout } = useAuth();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: PointerEvent) {
      if (rootRef.current !== null && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="relative" data-testid="user-menu">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex cursor-pointer items-center gap-2 rounded-lg border-0 bg-transparent py-1 pl-1 pr-1.5"
        aria-label="Account menu"
      >
        <span
          aria-hidden="true"
          className="flex h-6 w-6 items-center justify-center rounded-full text-[10px] font-semibold"
          style={{ background: "var(--m-ink)", color: "var(--m-paper)" }}
        >
          {initialsOf(me?.display_name)}
        </span>
        <span
          className="hidden max-w-28 truncate text-xs lg:inline"
          style={{ color: "var(--m-text-secondary)" }}
        >
          {me?.display_name ?? ""}
        </span>
        <Icon name="down" size={10} />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Account"
          className="absolute right-0 top-full z-50 mt-1 min-w-44 rounded-lg border py-1"
          style={{
            background: "var(--m-overlay-surface)",
            borderColor: "var(--m-border)",
            boxShadow: "var(--m-shadow-overlay)",
          }}
        >
          <div
            className="truncate px-3 py-1.5 text-[12px]"
            style={{ color: "var(--m-text-muted)", borderBottom: "1px solid var(--m-border)" }}
          >
            {me?.email ?? me?.display_name ?? "Signed in"}
          </div>
          <div className="p-1">
            <DevToolsLink onNavigate={() => setOpen(false)} />
            <Button variant="ghost" size="sm" className="w-full" onClick={() => void logout()}>
              Sign out
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

export function AppHeader() {
  const pathname = usePathname();
  if (isHeaderlessPath(pathname)) return null;
  return (
    // Wave41 lane 20: the container is the kit's TopBar. The contents stay the
    // app's: the canonical SVG lockup (no font dependency, unlike the kit's
    // text logo), the search launcher that opens /search, and the dock
    // cluster. TopBar supplies the height, border and surface.
    <TopBar data-testid="top-bar">
      <div className="flex shrink-0 items-center gap-2">
        {/*
          The canonical lockup: gold fruit, green leaf, outlined lowercase
          wordmark — one asset, with no font dependency, which is the point of
          the kit drawing the wordmark as outlines.

          This header was the one surface that re-drew the identity: it took
          the mark from the kit and then re-typed the wordmark as live text in
          Fraunces, which put a capital "Marula" beside a lowercase brand and
          depended on a webfont being installed. It also reached for the MONO
          lockup, a single-colour silhouette — right where one colour is
          required, wrong everywhere else, because it renders the fruit as a
          dark blob. `brand.tsx` now defaults to the colour variant, so this
          header just asks for the brand and gets the brand.

          `BrandMark` drops to the favicon dot below the kit's MARK_MIN_SIZE
          (32), so the narrow call below passes 32 and renders as itself.
        */}
        <span className="hidden md:inline-flex items-center">
          <BrandLockup size={LOCKUP_COMPLIANT_HEIGHT} />
        </span>
        {/*
          Below the md breakpoint the wordmark collides with the right dock
          cluster at ~480px, so the mark alone carries the identity there.
        */}
        <span className="inline-flex md:hidden items-center">
          <BrandMark size={32} />
        </span>
      </div>

      <div className="flex min-w-0 flex-1 justify-center px-1 md:mx-auto md:max-w-[520px] md:px-4">
        <SearchLauncher />
      </div>

      <div className="flex shrink-0 items-center gap-1 md:gap-1.5">
        <ConnectionBanner />
        <OutboxDock />
        <NotificationDock />
        <HelpPanel />
        <ThemeToggle />
        <UserMenu />
      </div>
    </TopBar>
  );
}

export type { Theme };
