// /portal — the CLIENT PORTAL shell, deliberately distinct from the staff
// app.
//
// Purpose: a separate layout family under one URL prefix so client users
// never see staff chrome (no multi-pane workbench, no admin nav). Same
// Workbench tokens and light/dark parity as the rest of Meridian, but the
// shell is a single centered column with its own quiet header.
// Non-purpose: no staff routes, no notification dock, no settings entry.
"use client";

import { Button } from "@meridian/ui";
import { useTheme } from "@meridian/ui";
import type { Theme } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { PortalAuthProvider, usePortalAuth } from "../../lib/portal-api";

function PortalThemeToggle() {
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

function PortalHeader() {
  const { me, signOut } = usePortalAuth();
  const router = useRouter();
  return (
    <header
      className="flex h-12 shrink-0 items-center justify-between border-b px-4"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      <div className="flex items-baseline gap-2">
        <span className="text-sm font-semibold tracking-wide">Meridian</span>
        <span
          className="text-xs uppercase tracking-widest"
          style={{ color: "var(--m-text-muted)" }}
        >
          Client Portal
        </span>
      </div>
      <div className="flex items-center gap-2">
        {me !== null && (
          <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            {me.display_name}
          </span>
        )}
        <PortalThemeToggle />
        {me !== null && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              signOut();
              router.replace("/portal/login");
            }}
          >
            Sign out
          </Button>
        )}
      </div>
    </header>
  );
}

export default function PortalLayout({ children }: { children: React.ReactNode }) {
  return (
    <PortalAuthProvider>
      <div className="flex min-h-screen flex-col" style={{ background: "var(--m-canvas)" }}>
        <PortalHeader />
        <main className="mx-auto w-full max-w-3xl flex-1 px-4 py-8">{children}</main>
      </div>
    </PortalAuthProvider>
  );
}
