// App header shared by authenticated surfaces.
//
// Purpose: product mark, the primary nav (MainNav — every reachable staff
// surface, grouped Workspace/Dispatch/Money/Ops/Settings), unified-search
// entry point (⌘K), theme toggle, identity + logout. Chrome-quiet per
// docs/theme.md — greyscale, one hairline border, no marketing surface.
// Non-purpose: no navigation state of its own; the search page owns results.
"use client";

import { useTheme } from "@meridian/ui";
import type { Theme } from "@meridian/ui";
import { Button, Kbd } from "@meridian/ui";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { API_BASE, useAuth } from "../lib/api";
import { NotificationDock } from "./notification-dock";
import { OutboxDock } from "./instant/outbox-dock";
import { ConnectionBanner } from "./instant/connection-banner";
import { MainNav } from "./main-nav";

// Surfaces that own their chrome (auth flows, the client portal) render no
// staff header.
const HEADERLESS = [
  /^\/login$/,
  /^\/mfa$/,
  /^\/sso$/,
  /^\/welcome$/,
  /^\/portal(\/|$)/,
  /^\/s\//,
  /^\/accept-invite/,
];

function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
      aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
    >
      {theme === "dark" ? "Light" : "Dark"}
    </Button>
  );
}

/**
 * Search entry point: a quiet button plus the global ⌘K / Ctrl+K shortcut
 * that jumps to /search. Results live on the search page.
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
    <Button
      variant="ghost"
      size="sm"
      onClick={() => router.push("/search")}
      className="flex items-center gap-1.5"
      aria-label="Search (Command K)"
      title="Search (⌘K)"
    >
      <span className="hidden md:inline">Search</span>
      <svg aria-hidden="true" width="14" height="14" viewBox="0 0 16 16" fill="none" className="md:hidden">
        <circle cx="7" cy="7" r="4.5" stroke="currentColor" strokeWidth="1.4" />
        <path d="m10.5 10.5 3 3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      </svg>
      <span className="hidden md:inline">
        <Kbd>⌘K</Kbd>
      </span>
    </Button>
  );
}

/**
 * Dev-tools entry (TASKLIST DX3): ONE probe call to GET /api/dev/status on
 * mount; the link renders ONLY when the API answers `enabled: true` —
 * which it only does with MERIDIAN_DEV_TOOLS=1. Any failure (404, network,
 * non-JSON) silently hides the link: no dev surface by default.
 */
function DevToolsLink() {
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
    <Button variant="ghost" size="sm" onClick={() => router.push("/dev")}>
      Dev
    </Button>
  );
}

export function AppHeader() {
  const { me, logout } = useAuth();
  const router = useRouter();
  const pathname = usePathname();
  if (HEADERLESS.some((pattern) => pattern.test(pathname))) return null;
  return (
    <header
      className="flex h-12 shrink-0 items-center justify-between border-b px-3 md:px-4"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      <div className="flex min-w-0 items-center gap-3 md:gap-5">
        <div className="flex items-baseline gap-2">
          <Link
            href="/today"
            className="text-sm font-semibold tracking-wide no-underline"
            style={{ color: "var(--m-text)" }}
            aria-label="Meridian home"
          >
            Meridian
          </Link>
          <span className="hidden text-xs lg:inline" style={{ color: "var(--m-text-muted)" }}>
            open-core PSA
          </span>
        </div>
        <MainNav />
      </div>
      <div className="flex shrink-0 items-center gap-1 md:gap-2">
        <ConnectionBanner />
        <OutboxDock />
        <SearchLauncher />
        <NotificationDock />
        <span className="hidden lg:inline-flex">
          <DevToolsLink />
        </span>
        <ThemeToggle />
        <span className="hidden whitespace-nowrap text-xs lg:inline" style={{ color: "var(--m-text-secondary)" }}>
          {me?.display_name ?? ""}
        </span>
        <Button variant="ghost" size="sm" onClick={() => void logout()}>
          Sign out
        </Button>
      </div>
    </header>
  );
}

export type { Theme };
