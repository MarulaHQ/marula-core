// /settings — the settings shell: an accordion section rail and the page.
//
// Purpose: the IA wrapper every settings surface renders into, built to
// `brand/marula-mockups/marula-surface-pack/web/settings/neutral/states/
// 01-settings-shell.png` and DESIGN-NOTES-settings.md §2: a centred column of
// about 1152px, a 232px section rail, a 32px gap, then the page. The rail is
// an ACCORDION — the eight groups are always visible, only the current group
// is open, so 28 pages never need a scroll. Below `lg` the rail becomes a
// horizontal strip so the page gets the full width (the lane-10 fix, kept).
//
// The rail lists every real page in `SETTINGS_RAIL`; the index page draws the
// pack's card list. Both read the same eight groups from `lib/settings/ia.ts`.
//
// Non-purpose: no settings logic, no permission filtering (a page a user may
// not open guards itself), no data fetching.

"use client";

import { useRequireAuth } from "../../lib/api";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useState, type ReactNode } from "react";
import { Icon } from "@marula-core/ui/kit";
import { SETTINGS_GROUPS, SETTINGS_RAIL, groupForPathname } from "../../lib/settings/ia";
import { SETTINGS_COLUMN, SHELL_GAP } from "../../components/settings/settings-forms";

function Chevron({ open }: { open: boolean }): React.JSX.Element {
  return (
    <span style={{ display: "inline-flex", color: "var(--m-text-secondary)" }}>
      <Icon name={open ? "down" : "chev"} size={16} />
    </span>
  );
}

export default function SettingsLayout({ children }: { children: ReactNode }): React.JSX.Element {
  const auth = useRequireAuth();
  const pathname = usePathname();
  const current = groupForPathname(pathname);
  // The current group is the one open on first paint; an admin can open others.
  const [open, setOpen] = useState<string[]>(current ? [current.group.id] : []);

  return (
    <div
      className="min-h-full flex flex-col"
      style={{ background: "var(--m-canvas)" }}
    >
      <div
        className="flex-1 w-full mx-auto flex flex-col lg:flex-row"
        style={{
          maxWidth: SETTINGS_COLUMN,
          gap: SHELL_GAP,
          paddingTop: 24,
          paddingBottom: 24,
          paddingLeft: 24,
          paddingRight: 24,
        }}
      >
        <nav
          aria-label="Settings sections"
          data-testid="settings-rail"
          className="w-full lg:shrink-0 lg:w-[232px] overflow-x-auto"
        >
          <div
            className="flex flex-row lg:flex-col"
            style={{ gap: 2, width: "max-content" }}
          >
            <div className="lg:w-full" style={{ marginBottom: 2 }}>
              <Link
                href="/settings"
                aria-current={pathname === "/settings" ? "page" : undefined}
                className="flex items-center"
                style={{
                  height: 36,
                  padding: "0 12px",
                  borderRadius: 10,
                  fontWeight: 500,
                  textDecoration: "none",
                  background: pathname === "/settings" ? "var(--m-accent)" : "transparent",
                  color: pathname === "/settings" ? "var(--m-accent-text)" : "var(--m-text)",
                }}
              >
                All settings
              </Link>
            </div>
            {SETTINGS_GROUPS.map((group) => {
              const isOpen = open.includes(group.id);
              const pages = SETTINGS_RAIL[group.id] ?? [];
              return (
                <div key={group.id} data-testid={`settings-group-${group.id}`} style={{ marginBottom: 2 }}>
                  <button
                    type="button"
                    aria-expanded={isOpen}
                    onClick={() =>
                      setOpen((prev) =>
                        prev.includes(group.id)
                          ? prev.filter((id) => id !== group.id)
                          : [...prev, group.id],
                      )
                    }
                    style={{
                      width: "100%",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      height: 36,
                      padding: "0 12px",
                      border: "none",
                      borderRadius: 10,
                      background: "transparent",
                      color: "var(--m-text)",
                      fontWeight: 600,
                      fontSize: 14,
                      cursor: "pointer",
                    }}
                  >
                    <span>{group.label}</span>
                    <Chevron open={isOpen} />
                  </button>
                  {isOpen ? (
                    <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
                      {pages.map((page) => {
                        const active =
                          pathname === page.href || pathname.startsWith(`${page.href}/`);
                        return (
                          <li key={`${group.id}-${page.href}-${page.label}`}>
                            <Link
                              href={page.href}
                              aria-current={active ? "page" : undefined}
                              className="flex items-center"
                              style={{
                                height: 36,
                                padding: "0 12px",
                                borderRadius: 10,
                                textDecoration: "none",
                                fontWeight: active ? 600 : 500,
                                color: active ? "var(--m-accent-text)" : "var(--m-text)",
                                background: active ? "var(--m-accent)" : "transparent",
                                transition: "background-color 120ms ease, color 120ms ease",
                                whiteSpace: "nowrap",
                              }}
                            >
                              {page.label}
                            </Link>
                          </li>
                        );
                      })}
                    </ul>
                  ) : null}
                </div>
              );
            })}
          </div>
        </nav>
        <main
          className="flex-1 min-w-0 relative"
          style={{ paddingBottom: 24 }}
          data-testid="settings-main"
        >
          {auth.status === "authed" ? children : null}
        </main>
      </div>
    </div>
  );
}
