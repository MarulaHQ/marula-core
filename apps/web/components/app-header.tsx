// App header shared by authenticated surfaces.
//
// Purpose: product mark, theme toggle, identity + logout. Chrome-quiet per
// docs/theme.md — greyscale, one hairline border, no marketing surface.
// Non-purpose: no navigation state of its own.
"use client";

import { useTheme } from "@meridian/ui";
import type { Theme } from "@meridian/ui";
import { Button } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useAuth } from "../lib/api";
import { NotificationDock } from "./notification-dock";

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

export function AppHeader() {
  const { me, logout } = useAuth();
  const router = useRouter();
  return (
    <header
      className="flex h-12 shrink-0 items-center justify-between border-b px-4"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      <div className="flex items-baseline gap-2">
        <span className="text-sm font-semibold tracking-wide">Meridian</span>
        <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          open-core PSA
        </span>
      </div>
      <div className="flex items-center gap-2">
        <NotificationDock />
        <Button
          variant="ghost"
          size="sm"
          onClick={() => router.push("/settings/general")}
        >
          Settings
        </Button>
        <ThemeToggle />
        <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
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
