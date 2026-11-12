// AppHeader — the top bar (mockup paradigm, staff surfaces).
//
// Purpose: brand, the unified-search field (⌘K), and the dock cluster —
// connection presence, the outbox chip when there is anything queued, help,
// theme, notifications with its unread count, and the identity menu. The old
// grouped page nav (MainNav) lives on the left icon rail (NavRail); this bar is
// chrome only. Chrome-quiet per docs/theme.md: tokens only, one hairline
// border, no marketing surface.
//
// Composition, in the order the mockup draws it: a fixed-width brand column so
// the search always starts at the same x, the search field, then a cluster
// pinned right. Every control in the cluster is the kit's 40px `IconButton`
// (one bordered square each) except the avatar, which is unframed — that
// rhythm is what makes the bar read as chrome rather than a toolbar.
//
// Non-purpose: no navigation state of its own; the search page owns results;
// notification preferences live at /settings/notifications.

"use client";

import { Button, cn, useTheme } from "@marula-core/ui";
import {
  Avatar,
  Icon,
  IconButton,
  Kbd,
  TopBar,
  topBarSearchClasses,
} from "@marula-core/ui/kit";
import type { Theme } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { API_BASE, useAuth } from "../lib/api";
import { BrandLockup, BrandMark, LOCKUP_COMPLIANT_HEIGHT } from "./brand";
import { NotificationDock } from "./notification-dock";
import { HelpPanel } from "./help/help-panel";
import { OutboxDock } from "./instant/outbox-dock";
import { ConnectionPresence } from "./instant/connection-presence";
import { isHeaderlessPath } from "./shell/chrome";

function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  const dark = theme === "dark";
  return (
    <IconButton
      aria-label={dark ? "Switch to light mode" : "Switch to dark mode"}
      title={dark ? "Switch to light mode" : "Switch to dark mode"}
      onClick={() => setTheme(dark ? "light" : "dark")}
    >
      <Icon name={dark ? "sun" : "moon"} size={18} />
    </IconButton>
  );
}

/**
 * Unified-search entry: the mockup's search field, opening /search instead of
 * filtering in place, because results live on their own page (grouped by kind
 * with a keyboard cursor). It borrows the kit field's shell rather than
 * restyling a lookalike, so the launcher and any real field stay identical.
 * The global ⌘K / Ctrl+K chord rides the Wave 29 command registry
 * (components/shortcuts) — one dispatcher, no duplicate listener.
 */
function SearchLauncher() {
  const router = useRouter();
  return (
    <button
      type="button"
      data-testid="search-launcher"
      onClick={() => router.push("/search")}
      aria-label="Search (Command K)"
      className={cn(topBarSearchClasses, "m-top-search cursor-pointer text-left")}
    >
      <Icon name="search" size={16} className="shrink-0" />
      <span className="min-w-0 flex-1 truncate text-sm max-md:hidden">
        Search tickets, clients, devices…
      </span>
      <span className="min-w-0 flex-1 truncate text-sm md:hidden">Search</span>
      <span className="hidden shrink-0 md:inline">
        <Kbd className="shrink-0">⌘K</Kbd>
      </span>
    </button>
  );
}

/**
 * Dev-tools entry (the private build tracker DX3): ONE probe call to GET /api/dev/status on
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

/** Identity menu: the unframed avatar (the mockup's one soft edge in the cluster),
 *  the name at wide widths, and a menu with sign out plus (when enabled) dev tools. */
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

  const name = me?.display_name ?? "";
  return (
    <div ref={rootRef} className="relative" data-testid="user-menu">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex cursor-pointer items-center gap-2 rounded-full border-0 bg-transparent p-0.5 hover:bg-muted"
        aria-label={name ? `Account menu, ${name}` : "Account menu"}
      >
        <Avatar initials={initialsOf(me?.display_name)} size="lg" />
        <span className="hidden max-w-32 truncate text-sm text-muted-foreground lg:inline">
          {name}
        </span>
        <Icon name="down" size={10} className="mr-1 hidden sm:inline" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Account"
          className="absolute right-0 top-full z-50 mt-2 min-w-48 overflow-hidden rounded-lg border border-border bg-popover py-1 shadow-mc"
        >
          <div className="truncate border-b border-border px-3 py-2 text-xs text-muted-foreground">
            {me?.email ?? name ?? "Signed in"}
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
    // Wave41 lane 20: the container is the kit's TopBar (56px, one hairline,
    // surface colour). The contents stay the app's: the canonical SVG lockup
    // (no font dependency, unlike the kit's text wordmark), the launcher that
    // opens /search, and the dock cluster.
    <TopBar data-testid="top-bar">
      {/*
        The canonical lockup: gold fruit, green leaf, outlined lowercase
        wordmark — one asset, with no font dependency, which is the point of
        the kit drawing the wordmark as outlines.

        `BrandMark` drops to the favicon dot below the kit's MARK_MIN_SIZE
        (32), so the narrow call below passes 32 and renders as itself. Below
        md the wordmark collides with the dock cluster, so the mark alone
        carries the identity there.

        The column is fixed-width from sm up (150px, 200px at md) so the search
        field starts at the same x on every surface and every zoom level, the
        way the mockup draws it. It used to be an auto-width span, so a wider
        lockup pushed the whole bar right.
      */}
      <div className="flex shrink-0 items-center sm:w-37.5 md:w-50">
        <span className="hidden items-center md:inline-flex">
          <BrandLockup size={LOCKUP_COMPLIANT_HEIGHT} />
        </span>
        <span className="inline-flex items-center md:hidden">
          <BrandMark size={32} />
        </span>
      </div>

      <SearchLauncher />

      <div className="ml-auto flex shrink-0 items-center gap-1.5 sm:gap-2">
        <OutboxDock />
        <span className="hidden pr-1.5 sm:inline-flex">
          <ConnectionPresence />
        </span>
        <span className="hidden sm:inline-flex">
          <HelpPanel />
        </span>
        <ThemeToggle />
        <NotificationDock />
        <UserMenu />
      </div>
    </TopBar>
  );
}

export type { Theme };
