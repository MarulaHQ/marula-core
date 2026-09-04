// MainNav — the primary staff navigation (TASKLIST UI-backlog, nav pass).
//
// Purpose: every reachable staff surface lives here, grouped by area:
// Workspace (Today, Boards, Search, Calendar) · Dispatch (board, planning,
// travel) · Money (Billing; Clients/Projects/Contracts join as they land) ·
// Ops (Dashboards, Provenance; later Alerts/Reports/Automations/Audit) ·
// Settings (the settings hub family). Groups with more than one page render
// as a submenu (hover flyout + click/keyboard toggle); the current page and
// its submenu parent are visibly highlighted, with hover/focus parity, in
// both themes — Workbench tokens only.
// Non-purpose: no navigation state of its own beyond the URL; board-level
// navigation stays in BoardNav on the surfaces that host it.
"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";

export interface NavLeaf {
  label: string;
  href: string;
  /** Pathname prefixes (besides the item's own href) that count as "here". */
  match?: string[];
}

export interface NavGroup {
  label: string;
  items: NavLeaf[];
}

/**
 * The nav tree. Money/Ops grow as their surfaces land (TASKLIST "Known
 * holes"): Clients/Projects/Contracts join Money; Alerts/Reports/
 * Automations/Audit join Ops.
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    label: "Workspace",
    items: [
      { label: "Today", href: "/today" },
      { label: "Boards", href: "/boards", match: ["/boards"] },
      { label: "Search", href: "/search" },
      { label: "Calendar", href: "/calendar" },
    ],
  },
  {
    label: "Dispatch",
    items: [
      { label: "Dispatch board", href: "/dispatch", match: ["/dispatch"] },
      { label: "Planning", href: "/dispatch/planning" },
      { label: "Travel", href: "/dispatch/travel" },
    ],
  },
  {
    label: "Money",
    items: [
      { label: "Clients", href: "/clients", match: ["/clients"] },
      { label: "Billing", href: "/billing", match: ["/billing"] },
    ],
  },
  {
    label: "Ops",
    items: [
      { label: "Dashboards", href: "/dashboards" },
      { label: "Provenance", href: "/provenance" },
    ],
  },
  {
    label: "Settings",
    items: [
      { label: "Overview", href: "/settings" },
      { label: "General", href: "/settings/general" },
      { label: "Workflow", href: "/settings/workflow" },
      { label: "SLA", href: "/settings/sla" },
      { label: "Notifications", href: "/settings/notifications" },
      { label: "Preferences", href: "/settings/preferences" },
      { label: "Mail", href: "/settings/mail" },
      { label: "Security", href: "/settings/security" },
      { label: "API keys", href: "/settings/api-keys" },
      { label: "Integrations", href: "/settings/integrations" },
      { label: "AI", href: "/settings/ai" },
      { label: "Agent platform", href: "/settings/agent-platform" },
    ],
  },
];

function isActive(pathname: string, item: NavLeaf): boolean {
  if (pathname === item.href) return true;
  return (item.match ?? []).some((prefix) => pathname.startsWith(prefix));
}

function linkStyle(active: boolean): React.CSSProperties {
  return {
    color: active ? "var(--m-text)" : "var(--m-text-secondary)",
    background: active ? "var(--m-accent-soft)" : "transparent",
  };
}

/** A single leaf link; active state is derived from the pathname. */
function NavItemLink({ item, onNavigate }: { item: NavLeaf; onNavigate?: () => void }) {
  const pathname = usePathname();
  const active = isActive(pathname, item);
  return (
    <Link
      href={item.href}
      onClick={onNavigate}
      aria-current={active ? "page" : undefined}
      className="no-underline transition-opacity duration-100 hover:opacity-100 focus-visible:opacity-100"
      style={{
        ...linkStyle(active),
        fontWeight: active ? 600 : 400,
        opacity: active ? 1 : undefined,
        borderBottom: `2px solid ${active ? "var(--m-text)" : "transparent"}`,
        padding: "2px 1px",
      }}
    >
      {item.label}
    </Link>
  );
}

interface SubmenuProps {
  group: NavGroup;
  onNavigate?: () => void;
}

/**
 * One submenu: opens on hover (pointer) and on click/Enter/ArrowDown
 * (keyboard/touch). The trigger carries the group's active state so the
 * current section stays visible while any child page is open.
 */
function Submenu({ group, onNavigate }: SubmenuProps) {
  const pathname = usePathname();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const groupActive = group.items.some((item) => isActive(pathname, item));

  useEffect(() => {
    setOpen(false);
  }, [pathname]);

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
    <div
      ref={rootRef}
      className="relative"
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
    >
      <button
        type="button"
        aria-haspopup="true"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setOpen(true);
          }
        }}
        className="cursor-pointer border-none bg-transparent text-[13px] transition-opacity duration-100 hover:opacity-100 focus-visible:opacity-100"
        style={{
          ...linkStyle(groupActive),
          fontWeight: groupActive ? 600 : 400,
          borderBottom: `2px solid ${groupActive ? "var(--m-text)" : "transparent"}`,
          padding: "2px 1px",
          opacity: groupActive ? 1 : undefined,
        }}
      >
        {group.label}
        <span aria-hidden="true" className="ml-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
          ▾
        </span>
      </button>
      <div
        className={`absolute left-0 top-full z-40 mt-1 min-w-44 py-1 ${open ? "" : "pointer-events-none opacity-0"}`}
        role="menu"
        aria-label={group.label}
        style={{
          background: "var(--m-overlay-surface)",
          border: "1px solid var(--m-border)",
          boxShadow: "0 4px 16px var(--m-shadow)",
          transition: "opacity 100ms",
        }}
      >
        {group.items.map((item) => {
          const active = isActive(pathname, item);
          return (
            <Link
              key={item.href}
              href={item.href}
              role="menuitem"
              aria-current={active ? "page" : undefined}
              onClick={onNavigate}
              className="block px-3 py-1.5 text-[13px] no-underline transition-colors duration-100 hover:bg-[var(--m-canvas)] focus-visible:bg-[var(--m-canvas)]"
              style={{
                color: active ? "var(--m-text)" : "var(--m-text-secondary)",
                fontWeight: active ? 600 : 400,
                background: active ? "var(--m-accent-soft)" : undefined,
                borderLeft: `2px solid ${active ? "var(--m-text)" : "transparent"}`,
              }}
            >
              {item.label}
            </Link>
          );
        })}
      </div>
    </div>
  );
}

/**
 * The full nav row: single-page groups render as direct links, multi-page
 * groups as submenus. `extra` lets callers append entries that render only
 * in some environments (the DevTools link).
 */
export function MainNav() {
  return (
    <nav aria-label="Primary" className="flex items-center gap-4 text-[13px]">
      {NAV_GROUPS.map((group) =>
        group.items.length === 1 && group.items[0] !== undefined ? (
          <NavItemLink key={group.label} item={group.items[0]} />
        ) : (
          <Submenu key={group.label} group={group} />
        ),
      )}
    </nav>
  );
}
