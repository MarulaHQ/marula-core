// /portal — the CLIENT PORTAL shell, deliberately distinct from the staff
// app.
//
// Purpose: a separate layout family under one URL prefix so client users
// never see staff chrome (no multi-pane workbench, no admin nav). Same
// Workbench tokens and light/dark parity as the rest of Meridian, but the
// shell is a single centered column with its own quiet header.
// Non-purpose: no staff routes, no notification dock, no settings entry.
//
// Wave 17 lane 8 UX pass: serif display wordmark (mockup v2 §7), icon+label
// section nav that collapses to labels-only at narrow widths, entrance
// motion riding the shared m-rise keyframe (≤120ms, transform/opacity,
// reduced-motion honored in globals.css), and a quiet footer.
"use client";

import { Button } from "@meridian/ui";
import { useTheme } from "@meridian/ui";
import type { Theme } from "@meridian/ui";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useRouter } from "next/navigation";
import { PortalAuthProvider, usePortalAuth } from "../../lib/portal-api";
import { BrandMark } from "../../components/brand";
import {
  portalAccentOverrides,
  portalOrgName,
} from "../../lib/portal-branding";

/** The honest brand substitution: configured org name, else "Meridian". */
const ORG_NAME = portalOrgName() ?? "Meridian";

function NavLinkIcon({ name, size = 14 }: { name: string; size?: number }) {
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 2,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };
  return name === "PaperPlaneTilt" ? (
    <svg {...common}>
      <path d="M22 2 11 13" />
      <path d="M22 2 15 22l-4-9-9-4 20-7Z" />
    </svg>
  ) : name === "Receipt" ? (
    <svg {...common}>
      <path d="M4 2v20l2-1 2 1 2-1 2 1 2-1 2 1 2-1 2 1V2l-2 1-2-1-2 1-2-1-2 1-2-1-2 1-2-1Z" />
      <path d="M16 8h-6a2 2 0 1 0 0 4h4a2 2 0 1 1 0 4H8" />
    </svg>
  ) : name === "FileText" ? (
    <svg {...common}>
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z" />
      <path d="M14 2v6h6" />
      <path d="M16 13H8M16 17H8M10 9H8" />
    </svg>
  ) : name === "BookOpen" ? (
    <svg {...common}>
      <path d="M12 7v14" />
      <path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3H3Z" />
    </svg>
  ) : (
    <svg {...common}>
      <rect x="3" y="4" width="18" height="18" rx="2" />
      <path d="M16 2v4M8 2v4M3 10h18" />
    </svg>
  );
}

const PORTAL_SECTIONS = [
  { href: "/portal/tickets", label: "Requests", icon: "PaperPlaneTilt" },
  { href: "/portal/quotes", label: "Quotes", icon: "FileText" },
  { href: "/portal/invoices", label: "Invoices", icon: "Receipt" },
  { href: "/portal/kb", label: "Knowledge", icon: "BookOpen" },
  { href: "/portal/schedule", label: "Schedule", icon: "CalendarBlank" },
] as const;

/** The quiet section nav: active section picks up the pill treatment, and
 * hover feedback rides the 100ms transition (site motion law). */
function PortalNavLink({ href, label }: { href: string; label: string }) {
  const pathname = usePathname();
  const active = pathname === href || pathname.startsWith(`${href}/`);
  const icon = PORTAL_SECTIONS.find((s) => s.href === href)?.icon ?? "PaperPlaneTilt";
  return (
    <Link
      href={href}
      aria-current={active ? "page" : undefined}
      className="flex shrink-0 items-center gap-1.5 px-2.5 py-1 text-xs transition-colors duration-100 hover:opacity-80"
      style={{
        borderRadius: "999px",
        background: active ? "var(--m-accent-soft)" : "transparent",
        color: active ? "var(--m-text)" : "var(--m-text-secondary)",
      }}
    >
      <NavLinkIcon name={icon} />
      <span className="whitespace-nowrap">{label}</span>
    </Link>
  );
}

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
      className="sticky top-0 z-10 shrink-0 border-b px-4 transition-colors duration-100"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      <div className="mx-auto flex h-14 w-full max-w-3xl items-center justify-between gap-3 px-1">
        <div className="flex min-w-0 items-center gap-3">
          <Link
            href="/portal"
            className="flex shrink-0 items-center gap-2"
            aria-label={`${ORG_NAME} client portal`}
          >
            <BrandMark size={20} />
            <span className="text-lg font-semibold tracking-tight">{ORG_NAME}</span>
            {me === null && (
              <span
                className="text-[10px] font-semibold uppercase tracking-widest"
                style={{ color: "var(--m-text-muted)" }}
              >
                Client portal
              </span>
            )}
          </Link>
          <nav className="ml-1 flex gap-1 overflow-x-auto" aria-label="Portal sections">
            {PORTAL_SECTIONS.map((s) => (
              <PortalNavLink key={s.href} href={s.href} label={s.label} />
            ))}
          </nav>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {me !== null && (
            <span
              className="hidden text-xs sm:inline"
              style={{ color: "var(--m-text-secondary)" }}
            >
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
      </div>
    </header>
  );
}

export default function PortalLayout({ children }: { children: React.ReactNode }) {
  return (
    <PortalAuthProvider>
      <div
        className="m-rise flex min-h-screen flex-col"
        style={{ background: "var(--m-canvas)", ...portalAccentOverrides() }}
      >
        <PortalHeader />
        <main className="mx-auto w-full max-w-3xl flex-1 px-4 pb-16 pt-8">{children}</main>
        <footer
          className="border-t px-4 py-4 text-center text-xs"
          style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
        >
          {ORG_NAME} — support access provided by your service desk.
        </footer>
      </div>
    </PortalAuthProvider>
  );
}
