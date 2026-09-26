// /settings — configuration hub shell.
//
// Purpose: the IA wrapper for every settings surface: product header on
// top, section nav as a left rail inside the content pane (a horizontal
// strip above the content below `lg` — the lane-10 narrow-viewport fix:
// the always-side-rail row clipped page content at 768/480), pages render
// to its right. Nav items whose backends land in later waves render a
// disabled "coming from another wave" state instead of dead links.
// Wave 17 lane 7: sections are grouped into labeled bands so 19 sections
// scan as five IAs instead of one long list; groups flatten on mobile.
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

interface NavGroup {
  label: string;
  items: NavItem[];
}

const GROUPS: NavGroup[] = [
  {
    label: "Workspace",
    items: [
      { href: "/settings/general", label: "General" },
      { href: "/settings/preferences", label: "Preferences" },
      { href: "/settings/notifications", label: "Notifications" },
      { href: "/settings/onboarding", label: "Onboarding" },
      { href: "/settings/shortcuts", label: "Shortcuts" },
    ],
  },
  {
    label: "People & access",
    items: [
      { href: "/settings/team", label: "Team" },
      { href: "/settings/security", label: "Security" },
      { href: "/settings/api-keys", label: "API keys" },
    ],
  },
  {
    label: "Delivery",
    items: [
      { href: "/settings/boards", label: "Boards" },
      { href: "/settings/taxonomy", label: "Taxonomy" },
      { href: "/settings/tags", label: "Tags" },
      { href: "/settings/workflow", label: "Workflow" },
      { href: "/settings/sla", label: "SLA" },
      { href: "/settings/portal", label: "Portal" },
      { href: "/settings/reply-snippets", label: "Reply snippets" },
      { href: "/settings/pm-schedules", label: "PM schedules" },
    ],
  },
  {
    label: "Systems",
    items: [
      { href: "/settings/email", label: "Email" },
      { href: "/settings/surveys", label: "Surveys" },
      { href: "/settings/integrations", label: "Integrations" },
      { href: "/settings/plugins", label: "Plugins" },
      { href: "/settings/agent-platform", label: "Agent platform" },
      { href: "/settings/ai", label: "AI & connectors" },
      { href: "/settings/inventory", label: "Inventory & merge" },
      { href: "/settings/labels", label: "Labels & scanning" },
      { href: "/settings/toolboxes", label: "Toolboxes & day pack" },
      { href: "/settings/self-host", label: "Self-host services" },
    ],
  },
  {
    label: "Data",
    items: [{ href: "/settings/export", label: "Export" }],
  },
];

function NavLink({
  href,
  label,
  pending,
  active,
}: NavItem & { active: boolean }) {
  if (pending) {
    return (
      <span
        aria-disabled="true"
        title="Coming from another wave"
        className="block rounded px-2 py-1.5 text-[13px] cursor-not-allowed whitespace-nowrap lg:whitespace-normal"
        style={{ color: "var(--m-text-muted)" }}
      >
        {label}
        <span className="ml-1 text-[10px] uppercase tracking-wide">soon</span>
      </span>
    );
  }
  const activeStyle = active
    ? {
        background: "var(--m-surface)",
        fontWeight: 600,
        color: "var(--m-text)",
        boxShadow: "inset 2px 0 0 0 var(--m-accent)",
      }
    : { color: "var(--m-text-secondary)" };
  return (
    <Link
      href={href}
      aria-current={active ? "page" : undefined}
      className="block rounded px-2 py-1.5 text-[13px] whitespace-nowrap lg:whitespace-normal"
      style={{
        transition: "background-color 100ms ease, opacity 100ms ease",
        ...activeStyle,
      }}
    >
      {label}
    </Link>
  );
}

export default function SettingsLayout({ children }: { children: ReactNode }) {
  const auth = useRequireAuth();
  const pathname = usePathname();

  return (
    <div
      className="min-h-full flex flex-col"
      style={{ background: "var(--m-canvas)" }}
    >
      {/* Narrow-viewport fix (lane-10): below `lg` the section rail
          collapses into a horizontal strip ABOVE the content, so pages
          get the full pane width — the old always-side-rail flex row
          clipped content at 768/480 on EVERY settings page (pane
          scrollWidth == viewport, content unreachable). Wide viewports
          keep the side rail unchanged. */}
      <div className="flex-1 w-full max-w-6xl mx-auto flex flex-col lg:flex-row gap-4 lg:gap-8 px-4 py-6 lg:px-6 lg:py-8">
        <nav
          aria-label="Settings sections"
          className="lg:w-44 shrink-0 overflow-x-auto"
        >
          <div className="flex flex-row lg:flex-col gap-4 lg:gap-5 w-max lg:w-auto">
            {GROUPS.map((group) => (
              <div
                key={group.label}
                data-testid={`settings-nav-group-${group.label.toLowerCase().replace(/[^a-z]+/g, "-")}`}
                className="flex flex-row lg:flex-col gap-1"
              >
                <p
                  className="text-[10px] uppercase tracking-wider pt-1 lg:pb-0 pb-1 pr-2 shrink-0 self-center lg:self-auto"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  {group.label}
                </p>
                <ul className="flex flex-row lg:grid flex-row gap-1 w-max lg:w-auto">
                  {group.items.map((section) => (
                    <li key={section.href}>
                      <NavLink
                        href={section.href}
                        label={section.label}
                        pending={section.pending}
                        active={pathname.startsWith(section.href)}
                      />
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </nav>
        <main className="flex-1 min-w-0">
          {auth.status === "authed" ? children : null}
        </main>
      </div>
    </div>
  );
}
