// NavRail — the left navigation (Wave 15 lane 6, mockup paradigm; expand
// behavior amended in lane 7).
//
// Purpose: the app shell's primary navigation as a slim dark rail:
// Tickets (active state + open-count badge), Clients, Devices, Automation,
// Knowledge, Reports, Settings — plus a bottom collapse toggle. COLLAPSED
// shows icons only; EXPANDED shows icons and titles (sidebar-style rows).
// Hover or focus opens a flyout panel with the section's sub-pages (every
// href keeps its existing path) in BOTH states; keyboard access rides
// normal link focus + Escape.
//
// Non-purpose: no route state beyond the URL; collapse persistence and the
// open-count badge fetch live in app-shell.tsx.

"use client";

import { Tooltip, TooltipProvider } from "@meridian/ui";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { activeRailSection, isRailLinkActive, RAIL_SECTIONS } from "./rail-model";

function TicketsIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect x="2.5" y="3.5" width="13" height="11" rx="1.5" stroke="currentColor" strokeWidth="1.4" />
      <path d="M5.5 7h7M5.5 10h4.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

function ClientsIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <circle cx="9" cy="6" r="2.75" stroke="currentColor" strokeWidth="1.4" />
      <path d="M3.5 14.5c.6-2.6 2.9-4 5.5-4s4.9 1.4 5.5 4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

function DevicesIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect x="2.5" y="4" width="10" height="7.5" rx="1.2" stroke="currentColor" strokeWidth="1.4" />
      <rect x="11" y="8" width="4.5" height="6" rx="1" stroke="currentColor" strokeWidth="1.4" />
      <path d="M5.5 14h4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

function StockIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <path d="M2.5 6.5 9 3l6.5 3.5v5L9 15l-6.5-3.5v-5Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
      <path d="M2.5 6.5 9 10l6.5-3.5M9 10v5" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
    </svg>
  );
}

function PurchasingIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <path d="M2 3.5h2l1.6 8.2a1 1 0 0 0 1 .8h6.9a1 1 0 0 0 1-.8L15.8 6H5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
      <circle cx="7" cy="15" r="1.2" stroke="currentColor" strokeWidth="1.4" />
      <circle cx="13" cy="15" r="1.2" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}

function AutomationIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <path d="M9 2.5v3M9 12.5v3M2.5 9h3M12.5 9h3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      <circle cx="9" cy="9" r="3.2" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}

function KnowledgeIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <path d="M3 4.5c2-1 4-1 6 0v9c-2-1-4-1-6 0v-9Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
      <path d="M15 4.5c-2-1-4-1-6 0v9c2-1 4-1 6 0v-9Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
    </svg>
  );
}

function ReportsIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <path d="M3 15V9M7.5 15V4M12 15v-4M16.5 15H1.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18" fill="none">
      <circle cx="9" cy="9" r="2.2" stroke="currentColor" strokeWidth="1.4" />
      <path
        d="M9 2.8v1.9M9 13.3v1.9M2.8 9h1.9M13.3 9h1.9M4.6 4.6l1.35 1.35M12.05 12.05l1.35 1.35M13.4 4.6l-1.35 1.35M5.95 12.05 4.6 13.4"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}

const SECTION_ICONS: Record<string, () => React.JSX.Element> = {
  tickets: TicketsIcon,
  clients: ClientsIcon,
  devices: DevicesIcon,
  stock: StockIcon,
  purchasing: PurchasingIcon,
  automation: AutomationIcon,
  knowledge: KnowledgeIcon,
  reports: ReportsIcon,
  settings: SettingsIcon,
};

/** Active fill on the dark rail — preset-style 12% white wash (token-mixed). */
const RAIL_ACTIVE_BG = "color-mix(in srgb, var(--m-text-dark) 12%, transparent)";

function CollapseIcon({ collapsed }: { collapsed: boolean }) {
  return (
    <svg
      aria-hidden="true"
      width="14"
      height="14"
      viewBox="0 0 14 14"
      fill="none"
      style={{
        // Collapsed: chevron points outward (→) — the affordance invites
        // expanding. Expanded: points inward (←) to collapse.
        transform: collapsed ? "none" : "rotate(180deg)",
        transition: "transform 120ms ease",
      }}
    >
      <path
        d="M5.5 3.5 9 7l-3.5 3.5"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function NavRail({
  openCount,
  collapsed,
  onToggleCollapsed,
}: {
  /** Open-ticket count for the Tickets badge (null = not loaded yet). */
  openCount: number | null;
  collapsed: boolean;
  onToggleCollapsed: () => void;
}) {
  const pathname = usePathname();
  const active = activeRailSection(pathname);
  const [openSection, setOpenSection] = useState<string | null>(null);
  const rootRef = useRef<HTMLElement>(null);
  const closeTimer = useRef<number | null>(null);

  useEffect(() => {
    setOpenSection(null);
  }, [pathname]);

  useEffect(() => {
    if (openSection === null) return;
    function onPointerDown(event: PointerEvent) {
      if (rootRef.current !== null && !rootRef.current.contains(event.target as Node)) {
        setOpenSection(null);
      }
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpenSection(null);
    }
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
      if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    };
  }, [openSection]);

  function scheduleClose() {
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => setOpenSection(null), 150);
  }
  function cancelClose(id: string) {
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    closeTimer.current = null;
    setOpenSection(id);
  }

  return (
    <TooltipProvider delayDuration={250}>
      <nav
        ref={rootRef}
        aria-label="Primary"
        data-testid="nav-rail"
        data-collapsed={collapsed}
        className="relative z-40 flex shrink-0 flex-col items-center justify-between border-r py-2 transition-none"
        style={{
          width: collapsed ? 36 : 188,
          background: "var(--m-overlay-dark)",
          borderColor: "var(--m-border-dark)",
        }}
      >
        <div className="flex w-full flex-col items-center gap-1">
          {RAIL_SECTIONS.map((section, index) => {
            const Icon = SECTION_ICONS[section.id] ?? TicketsIcon;
            const sectionActive = active?.id === section.id;
            const flyoutOpen = openSection === section.id;
            return (
              <div
                key={section.id}
                className={`m-rail-item-wrap relative flex w-full ${collapsed ? "justify-center" : "justify-start px-1.5"}`}
                style={{ "--stagger-i": Math.min(index, 7) } as React.CSSProperties}
                onMouseEnter={() => cancelClose(section.id)}
                onMouseLeave={scheduleClose}
              >
                <Tooltip
                  trigger={
                    <Link
                      href={section.href}
                      aria-current={sectionActive ? "page" : undefined}
                      aria-expanded={flyoutOpen}
                      aria-haspopup="true"
                      data-testid={`rail-${section.id}`}
                      onFocus={() => cancelClose(section.id)}
                      onClick={() => setOpenSection(null)}
                      className={`m-rail-item relative flex h-9 ${collapsed ? "w-9 justify-center" : "min-w-0 flex-1 items-center gap-2.5 px-1.5"} rounded-lg no-underline`}
                      style={{
                        color: sectionActive ? "var(--m-text-dark)" : "var(--m-mist)",
                        background: sectionActive ? RAIL_ACTIVE_BG : "transparent",
                      }}
                    >
                      <span className="grid shrink-0 place-items-center">
                        <Icon />
                      </span>
                      {!collapsed && (
                        <span
                          className="min-w-0 truncate text-[12.5px] font-medium"
                          data-testid={`rail-${section.id}-label`}
                        >
                          {section.label}
                        </span>
                      )}
                      {section.id === "tickets" && openCount !== null && openCount > 0 && (
                        <span
                          data-testid="rail-tickets-badge"
                          className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] font-semibold leading-none"
                          style={{ background: "var(--m-accent)", color: "var(--m-paper)" }}
                        >
                          {openCount > 99 ? "99+" : openCount}
                        </span>
                      )}
                      {sectionActive && (
                        <span
                          aria-hidden="true"
                          className="absolute -left-2 top-1/2 h-5 w-[3px] -translate-y-1/2 rounded-r-full"
                          style={{ background: "var(--m-accent)" }}
                        />
                      )}
                    </Link>
                  }
                >
                  {section.label}
                </Tooltip>

                {flyoutOpen && (
                  <div
                    role="menu"
                    aria-label={`${section.label} pages`}
                    className="m-rail-flyout absolute left-full top-0 z-50 ml-1 min-w-44 rounded-lg border py-1"
                    style={{
                      background: "var(--m-overlay-dark)",
                      borderColor: "var(--m-border-dark)",
                      boxShadow: "var(--m-shadow-overlay)",
                    }}
                  >
                    {section.links.map((link) => {
                      const linkActive = isRailLinkActive(pathname, link.href);
                      return (
                        <Link
                          key={link.href}
                          href={link.href}
                          role="menuitem"
                          aria-current={linkActive ? "page" : undefined}
                          onClick={() => setOpenSection(null)}
                          className="block px-3 py-1.5 text-[13px] no-underline"
                          style={{
                            color: linkActive ? "var(--m-text-dark)" : "var(--m-mist)",
                            background: linkActive ? RAIL_ACTIVE_BG : "transparent",
                            fontWeight: linkActive ? 600 : 400,
                          }}
                        >
                          {link.label}
                        </Link>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>

        <div className={`flex w-full ${collapsed ? "justify-center" : "px-1.5"}`}>
          <Tooltip
            trigger={
              <button
                type="button"
                data-testid="rail-collapse"
                aria-label={collapsed ? "Expand rail" : "Collapse rail"}
                aria-pressed={collapsed}
                onClick={onToggleCollapsed}
                className={`m-rail-item flex h-8 ${collapsed ? "w-8 justify-center" : "min-w-0 flex-1 items-center gap-2 px-1"} items-center rounded-lg border-0 bg-transparent`}
                style={{ color: "var(--m-mist)" }}
              >
                <CollapseIcon collapsed={collapsed} />
                {!collapsed && (
                  <span className="truncate text-[12px] font-medium">Collapse</span>
                )}
              </button>
            }
          >
            {collapsed ? "Expand rail" : "Collapse rail"}
          </Tooltip>
        </div>
      </nav>
    </TooltipProvider>
  );
}
