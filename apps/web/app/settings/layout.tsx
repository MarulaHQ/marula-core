// /settings — configuration hub shell.
//
// Purpose: the IA wrapper for every settings surface: product header on
// top, section nav as a left rail inside the content pane, pages render
// to its right. Nav items whose backends land in later waves render a
// disabled "coming from another wave" state instead of dead links.
//
// Non-purpose: no settings logic here — pages own their data; this file
// never reads or writes configuration.
"use client";

import { useRequireAuth } from "../../lib/api";
import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";

interface NavItem {
  href: string;
  label: string;
  /** Placeholder sections are rendered but not yet backed by a wave. */
  pending?: boolean;
}

const SECTIONS: NavItem[] = [
  { href: "/settings/general", label: "General" },
  { href: "/settings/team", label: "Team" },
  { href: "/settings/preferences", label: "Preferences" },
  { href: "/settings/notifications", label: "Notifications" },
  { href: "/settings/security", label: "Security" },
  { href: "/settings/workflow", label: "Workflow" },
  { href: "/settings/tags", label: "Tags" },
  { href: "/settings/pm-schedules", label: "PM schedules" },
  { href: "/settings/sla", label: "SLA" },
  { href: "/settings/portal", label: "Portal" },
  { href: "/settings/mail", label: "Mail", pending: true },
  { href: "/settings/integrations", label: "Integrations" },
  { href: "/settings/api-keys", label: "API keys" },
  { href: "/settings/agent-platform", label: "Agent platform" },
  { href: "/settings/ai", label: "AI & connectors" },
];

export default function SettingsLayout({ children }: { children: ReactNode }) {
  const auth = useRequireAuth();
  const pathname = usePathname();

  return (
    <div
      className="min-h-full flex flex-col"
      style={{ background: "var(--m-canvas)" }}
    >
      <div className="flex-1 w-full max-w-5xl mx-auto flex gap-8 px-6 py-8">
        <nav aria-label="Settings sections" className="w-44 shrink-0">
          <ul className="grid gap-1">
            {SECTIONS.map((section) => {
              const active = pathname.startsWith(section.href);
              if (section.pending) {
                return (
                  <li
                    key={section.href}
                    aria-disabled="true"
                    title="Coming from another wave"
                    className="rounded-[2px] px-2 py-1.5 text-[13px] cursor-not-allowed"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    {section.label}
                    <span className="ml-1 text-[10px] uppercase tracking-wide">
                      soon
                    </span>
                  </li>
                );
              }
              return (
                <li key={section.href}>
                  <Link
                    href={section.href}
                    aria-current={active ? "page" : undefined}
                    className="block rounded-[2px] px-2 py-1.5 text-[13px]"
                    style={{
                      color: active
                        ? "var(--m-text)"
                        : "var(--m-text-secondary)",
                      background: active ? "var(--m-surface)" : "transparent",
                      fontWeight: active ? 600 : 400,
                      borderLeft: active
                        ? "2px solid var(--m-accent)"
                        : "2px solid transparent",
                    }}
                  >
                    {section.label}
                  </Link>
                </li>
              );
            })}
          </ul>
        </nav>
        <main className="flex-1 min-w-0">
          {auth.status === "authed" ? children : null}
        </main>
      </div>
    </div>
  );
}
