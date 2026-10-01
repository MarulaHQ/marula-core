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

import { Tooltip, TooltipProvider } from "@marula-core/ui";
import { AppRail, Icon, RailItem, RailSpacer, RailToggle, type IconName } from "@marula-core/ui/kit";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { activeRailSection, isRailLinkActive, RAIL_SECTIONS } from "./rail-model";

/** Section id -> the kit's icon name. The rail draws from the one icon door. */
const SECTION_ICONS: Record<string, IconName> = {
  tickets: "ticket",
  clients: "users",
  devices: "monitor",
  stock: "box",
  purchasing: "cart",
  automation: "bolt",
  knowledge: "book",
  reports: "chart",
  settings: "settings",
};

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
  const rootRef = useRef<HTMLDivElement>(null);
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
      {/* Wave41 lane 20: the rail is the kit's AppRail. Its items are the kit's
          RailItem through asChild, so next/link keeps client-side navigation and
          the rail content (icon, label, badge) rides inside the link. */}
      <div ref={rootRef} className="contents">
        <AppRail
          expanded={!collapsed}
          onExpandedChange={() => onToggleCollapsed()}
          data-testid="nav-rail"
          data-collapsed={collapsed}
        >
          <div className="flex w-full flex-col items-center gap-1">
            {RAIL_SECTIONS.map((section, index) => {
              const iconName = SECTION_ICONS[section.id] ?? "ticket";
              const sectionActive = active?.id === section.id;
              const flyoutOpen = openSection === section.id;
              return (
                <div
                  key={section.id}
                  className="relative flex w-full justify-center"
                  style={{ "--stagger-i": Math.min(index, 7) } as React.CSSProperties}
                  onMouseEnter={() => cancelClose(section.id)}
                  onMouseLeave={scheduleClose}
                >
                  <Tooltip
                    trigger={
                      <RailItem
                        asChild
                        icon={<Icon name={iconName} size={18} />}
                        label={section.label}
                        current={sectionActive}
                        href={section.href}
                      >
                        <Link
                          href={section.href}
                          data-testid={`rail-${section.id}`}
                          aria-expanded={flyoutOpen}
                          aria-haspopup="true"
                          onFocus={() => cancelClose(section.id)}
                          onClick={() => setOpenSection(null)}
                        >
                          <span className="grid shrink-0 place-items-center">
                            <Icon name={iconName} size={18} />
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
                              className="absolute right-0 top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-mc-act px-1 text-[10px] font-semibold leading-none text-mc-act-fg"
                            >
                              {openCount > 99 ? "99+" : openCount}
                            </span>
                          )}
                        </Link>
                      </RailItem>
                    }
                  >
                    {section.label}
                  </Tooltip>

                  {flyoutOpen && (
                    <div
                      role="menu"
                      aria-label={`${section.label} pages`}
                      className="absolute left-full top-0 z-50 ml-1 min-w-44 rounded-ctl border border-border bg-popover py-1 shadow-mc"
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
                            className={`block px-3 py-1.5 text-[13px] no-underline ${linkActive ? "font-semibold text-foreground" : "text-muted-foreground"}`}
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

          <RailSpacer />

          <div className={`flex w-full ${collapsed ? "justify-center" : "px-1.5"}`}>
            <Tooltip
              trigger={<RailToggle data-testid="rail-collapse" aria-pressed={collapsed} />}
            >
              {collapsed ? "Expand rail" : "Collapse rail"}
            </Tooltip>
          </div>
        </AppRail>
      </div>
    </TooltipProvider>
  );
}
