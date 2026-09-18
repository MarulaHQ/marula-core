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

import { useTheme } from "@meridian/ui";
import type { Theme } from "@meridian/ui";
import { Button, Kbd } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { API_BASE, useAuth } from "../lib/api";
import { BrandMark } from "./brand";
import { NotificationDock } from "./notification-dock";
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
        <svg aria-hidden="true" width="15" height="15" viewBox="0 0 16 16" fill="none">
          <circle cx="8" cy="8" r="3.1" stroke="currentColor" strokeWidth="1.4" />
          <path
            d="M8 1.2v1.4M8 13.4v1.4M1.2 8h1.4M13.4 8h1.4M3.3 3.3l1 1M11.7 11.7l1 1M12.7 3.3l-1 1M4.3 11.7l-1 1"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      ) : (
        <svg aria-hidden="true" width="15" height="15" viewBox="0 0 16 16" fill="none">
          <path
            d="M13.2 9.6A5.6 5.6 0 0 1 6.4 2.8 5.6 5.6 0 1 0 13.2 9.6Z"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinejoin="round"
          />
        </svg>
      )}
    </Button>
  );
}

/**
 * Unified-search entry: a quiet pill that reads like the mockup's search
 * field (placeholder + ⌘K) and jumps to /search. Results live on the
 * search page; the global ⌘K / Ctrl+K shortcut is preserved.
 */
function SearchLauncher() {
  const router = useRouter();
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        router.push("/search");
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [router]);
  return (
    <button
      type="button"
      data-testid="search-launcher"
      onClick={() => router.push("/search")}
      aria-label="Search (Command K)"
      className="m-top-search flex h-8 min-w-0 max-w-[420px] flex-1 cursor-pointer items-center gap-2 rounded-lg border px-3 text-left"
      style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}
    >
      <svg aria-hidden="true" width="13" height="13" viewBox="0 0 16 16" fill="none" className="shrink-0">
        <circle cx="7" cy="7" r="4.5" stroke="currentColor" strokeWidth="1.4" />
        <path d="m10.5 10.5 3 3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      </svg>
      <span
        className="min-w-0 flex-1 truncate text-[12.5px]"
        style={{ color: "var(--m-text-muted)" }}
      >
        Search tickets, clients, devices…
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
 * which it only does with MERIDIAN_DEV_TOOLS=1. Any failure (404, network,
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
        <svg aria-hidden="true" width="10" height="10" viewBox="0 0 10 10" fill="none">
          <path d="M2 3.5 5 6.5 8 3.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
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
    <header
      data-testid="top-bar"
      className="flex h-12 shrink-0 items-center gap-2 border-b px-3 md:px-4"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      <div className="flex shrink-0 items-center gap-2">
        <BrandMark size={24} />
        <span className="text-sm font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
          Meridian
        </span>
      </div>

      <div className="mx-auto flex min-w-0 max-w-[520px] flex-1 justify-center px-1 md:px-4">
        <SearchLauncher />
      </div>

      <div className="flex shrink-0 items-center gap-1 md:gap-1.5">
        <ConnectionBanner />
        <OutboxDock />
        <NotificationDock />
        <ThemeToggle />
        <UserMenu />
      </div>
    </header>
  );
}

export type { Theme };
