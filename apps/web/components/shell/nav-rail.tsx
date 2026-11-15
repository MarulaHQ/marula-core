// NavRail — the app shell's primary navigation.
//
// Purpose: an icon rail with two states. COLLAPSED shows icon tiles only;
// EXPANDED shows the tiles with labels and, for the section you are in, its
// pages. A section's pages open *inside* the rail as a disclosure, never as a
// hover flyout: a click opens them, Esc or a second click closes them, and
// opening one closes the rest so the rail never turns into a forty-row scroll.
// Today is the first destination and has no pages; Settings is a leaf because
// /settings draws its own grouped section rail.
//
// Non-purpose: no route state beyond the URL; collapse persistence and the
// open-count badge fetch live in app-shell.tsx. The rail never invents data —
// the only badge is the ticket count the shell already fetches.

"use client";

import { Tooltip, TooltipProvider } from "@marula-core/ui";
import { AppRail, Icon, RailItem, RailSpacer, RailToggle, type IconName } from "@marula-core/ui/kit";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Fragment, useEffect, useState } from "react";
import { activeRailHref, activeRailSection, RAIL_SECTIONS } from "./rail-model";
import { NavCounter } from "./nav-counter";

/** Section id -> the kit's icon name. The rail draws from the one icon door. */
const SECTION_ICONS: Record<string, IconName> = {
  today: "today",
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
  const activeId = active?.id ?? null;
  // The section you are in is the one open, on first paint and on every
  // navigation. Initialised from the pathname rather than opened in an effect,
  // so there is no frame where the rail looks empty.
  const [openSection, setOpenSection] = useState<string | null>(() =>
    active !== null && active.links.length > 0 ? active.id : null,
  );
  const expanded = !collapsed;

  useEffect(() => {
    setOpenSection(activeId !== null ? activeId : null);
  }, [activeId]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpenSection(null);
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  function toggleSection(id: string) {
    setOpenSection((prev) => (prev === id ? null : id));
  }

  return (
    <TooltipProvider delayDuration={250}>
      {/* Wave41 lane 20: the rail is the kit's AppRail. Its rows are the kit's
          RailItem through asChild, so next/link keeps client-side navigation and
          the row content (icon, label, badge) rides inside the link.

          The shell owns the rail's width, and it is the handoff kit's pair:
          64px icons-only, 188px expanded, 44px tiles with 20px glyphs (the
          nav-bar fidelity landing record measures exactly that). Pages are not
          a third width: they open inside these 188px, which is why the child
          row metrics are the kit's `.sub2` (36px row, 13px label, 46px text
          column) rather than a new scale. */}
      <div className="contents">
        <AppRail
          expanded={expanded}
          onExpandedChange={() => onToggleCollapsed()}
          data-testid="nav-rail"
          data-collapsed={collapsed}
          className={`overflow-y-auto ${collapsed ? "w-16" : "w-47"}`}
        >
          <div className="flex w-full flex-col items-center gap-1.5">
            {RAIL_SECTIONS.map((section) => {
              const iconName = SECTION_ICONS[section.id] ?? "ticket";
              const sectionActive = active?.id === section.id;
              const hasChildren = section.links.length > 0;
              const open = expanded && hasChildren && openSection === section.id;
              // One page is "the page you are on", even when nested routes
              // share a parent link (see activeRailHref).
              const currentHref = sectionActive ? activeRailHref(pathname, section) : null;
              const countedLabel =
                collapsed && section.id === "tickets" && openCount !== null && openCount > 0
                  ? `${section.label}, ${openCount} open`
                  : section.label;
              // A section that contains the current page wears the tile, but
              // `aria-current` belongs to the child page itself; see below.
              const rowClassName = hasChildren
                ? `data-[section-active=true]:bg-mc-act data-[section-active=true]:text-mc-act-fg${
                    expanded ? " w-full pr-9" : ""
                  }`
                : undefined;

              return (
                <div
                  key={section.id}
                  className="relative flex w-full flex-col items-center"
                >
                  <div className="relative flex w-full items-center justify-center">
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
                          label={countedLabel}
                          current={sectionActive && !hasChildren}
                          href={section.href}
                        >
                          <Link
                            href={section.href}
                            data-testid={`rail-${section.id}`}
                            /* A section that contains the current page is marked
                               for sight, not as `aria-current`: the page itself
                               is one of its children, and two elements claiming
                               to be the current page is a lie to a screen
                               reader. Leaf sections (Today, Settings) use
                               `current`, because they are the page. */
                            data-section-active={
                              sectionActive && hasChildren ? "true" : undefined
                            }
                            /* Re-clicking the section you are on folds its pages
                               instead of pushing the same URL again. */
                            onClick={() => {
                              if (sectionActive && hasChildren) toggleSection(section.id);
                            }}
                            className={rowClassName}
                          >
                            <span className="grid shrink-0 place-items-center">
                              <Icon name={iconName} size={20} />
                            </span>
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
                                {/* One count chip, two placements: on the tile's
                                    corner while the rail is icons-only, and inline
                                    at the row's end once the rail is expanded. */}
                                <NavCounter
                                  count={openCount}
                                  className={collapsed ? "absolute -right-1 -top-1" : "ml-auto"}
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

                    {/* The disclosure control. A separate button, not the row:
                        the row must stay a link — clicking it goes to the
                        section's page, which is what the rail's contract and the
                        shell-chrome flow both expect. 28px target, the WCAG 2.2
                        minimum for a pointer target. */}
                    {expanded && hasChildren && (
                      <button
                        type="button"
                        aria-expanded={open}
                        aria-controls={`rail-pages-${section.id}`}
                        aria-label={`${open ? "Hide" : "Show"} ${section.label} pages`}
                        data-testid={`rail-${section.id}-disclosure`}
                        onClick={() => toggleSection(section.id)}
                        className="absolute right-1 top-1/2 grid h-7 w-7 -translate-y-1/2 cursor-pointer place-items-center rounded-ctl border-0 bg-transparent text-muted-foreground hover:bg-muted hover:text-foreground"
                      >
                        <span
                          className={`grid place-items-center transition-transform duration-100 ${open ? "rotate-90" : ""}`}
                        >
                          <Icon name="chev" size={14} />
                        </span>
                      </button>
                    )}
                  </div>

                  {expanded && hasChildren && (
                    <div
                      id={`rail-pages-${section.id}`}
                      className="m-rail-disclosure"
                      data-open={open}
                      /* Closed pages stay mounted so the height can animate, so
                         they must also leave the tab order and the a11y tree. */
                      inert={!open}
                    >
                      <div>
                        <div className="m-rail-children">
                          {section.links.map((link, linkIndex) => {
                            const heading =
                              link.group !== undefined &&
                              link.group !== section.links[linkIndex - 1]?.group
                                ? link.group
                                : null;
                            return (
                              <Fragment key={link.href}>
                                {heading !== null && (
                                  <div className="m-rail-group">{heading}</div>
                                )}
                                <Link
                                  href={link.href}
                                  aria-current={link.href === currentHref ? "page" : undefined}
                                  className="m-rail-child"
                                >
                                  {link.label}
                                </Link>
                              </Fragment>
                            );
                          })}
                        </div>
                      </div>
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
