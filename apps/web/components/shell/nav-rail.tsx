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
import { NavCounter } from "./nav-counter";

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
          the rail content (icon, label, badge) rides inside the link.

          The shell owns the rail's width (the kit's contract: 64px icons-only,
          188px expanded). The app had no width at all, so the column measured
          itself from a 44px tile and drew a 44px rail — narrower than every
          mockup surface and narrower than the tiles it had to hold under the
          phone override. Set it here, where the collapse state already lives. */}
      <div ref={rootRef} className="contents">
        <AppRail
          expanded={!collapsed}
          onExpandedChange={() => onToggleCollapsed()}
          data-testid="nav-rail"
          data-collapsed={collapsed}
          className={collapsed ? "w-16" : "w-47"}
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
                        icon={<Icon name={iconName} size={20} />}
                        /* The rail's own name, and the only place the count can
                           reach a screen reader while the rail is icons-only:
                           RailItem owns `aria-label` on the collapsed row, so
                           the sr-only text below is inside a subtree that the
                           label masks. Expanded rows have no aria-label and get
                           the count from that sr-only text instead. */
                        label={
                          collapsed && section.id === "tickets" && openCount !== null && openCount > 0
                            ? `${section.label}, ${openCount} open`
                            : section.label
                        }
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
                          {/* The mockup's rail glyphs are 20px on the 24 grid
                              (`#rail a svg{width:20px;height:20px}`), and the
                              icon set's 1.75px stroke is drawn for that size.
                              At 18 they read thinner than the pack. */}
                          <span className="grid shrink-0 place-items-center">
                            <Icon name={iconName} size={20} />
                          </span>
                          {/* Weight is the kit's rule, not this file's: a
                              destination is medium, the current one is
                              semibold. A fixed font-medium on the label
                              overrode the kit's own semibold for the current
                              row, so every row weighed the same. */}
                          {!collapsed && (
                            <span
                              className={`min-w-0 truncate text-sm ${sectionActive ? "font-semibold" : "font-medium"}`}
                              data-testid={`rail-${section.id}-label`}
                            >
                              {section.label}
                            </span>
                          )}
                          {section.id === "tickets" && (
                            <>
                              {/*
                                One count chip, two placements: on the tile's
                                corner while the rail is icons-only, and inline
                                at the row's end once the rail is expanded (a
                                corner chip on a 188px row floats far from the
                                word it counts). The chip itself is aria-hidden;
                                the count reaches a screen reader here, once.
                              */}
                              <NavCounter
                                count={openCount}
                                className={
                                  collapsed ? "absolute -right-1 -top-1" : "ml-auto"
                                }
                              />
                              {openCount !== null && openCount > 0 && (
                                <span className="sr-only">{openCount} open</span>
                              )}
                            </>
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
