// /settings — the index: every group, every page, one screen.
//
// Purpose: state 01 of the settings pack (`web/settings/neutral/states/
// 01-settings-shell.png`). The heading, the sentence under it, then a
// two-column grid of eight cards, one per group, each with a count tag and
// its pages, every page with a one-line description.
//
// The card list is the pack's. Two honest departures, both documented in the
// lane landing record and both visible on screen rather than hidden: where
// the app merged two drawn pages onto one route, both rows are kept and the
// extra row says so; where the pack drew a page the product has no route for,
// the row points at the nearest real surface and the description says it is
// derived. The section rail on the left lists every real page, so nothing
// this index omits becomes unreachable.
//
// Non-purpose: no settings logic. Every card row is a link.

import Link from "next/link";
import { SETTINGS_GROUPS } from "../../lib/settings/ia";
import { SettingsHeader } from "../../components/settings/settings-shell";

export default function SettingsIndexPage(): React.JSX.Element {
  return (
    <div data-testid="settings-index">
      <SettingsHeader
        title="Settings"
        description="Change the system once, so nobody has to think about it again."
      />
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
          gap: 14,
        }}      >
        {SETTINGS_GROUPS.map((group) => (
          <section
            key={group.id}
            data-testid={`settings-group-card-${group.id}`}
            style={{
              border: "1px solid var(--m-border)",
              borderRadius: 16,
              background: "var(--m-surface)",
              padding: "16px 18px",
            }}
          >
            <h2
              style={{
                margin: "0 0 4px",
                fontSize: 15,
                fontWeight: 600,
                color: "var(--m-text)",
                display: "flex",
                alignItems: "center",
                gap: 8,
              }}
            >
              {group.label}
              <span
                data-testid={`settings-group-count-${group.id}`}
                style={{
                  fontSize: 12,
                  fontWeight: 600,
                  color: "var(--m-text-secondary)",
                  background: "var(--m-surface-2)",
                  borderRadius: 999,
                  padding: "0 8px",
                  height: 20,
                  display: "inline-flex",
                  alignItems: "center",
                }}
              >
                {group.pages.length}
              </span>
            </h2>
            {group.pages.map((page) => (
              <Link
                key={`${group.id}-${page.label}`}
                href={page.href}
                data-testid={`settings-index-link-${page.label.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`}
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  gap: 10,
                  minHeight: 36,
                  color: "var(--m-text)",
                  textDecoration: "none",
                  fontWeight: 500,
                  borderTop: "1px solid var(--m-border)",
                }}
              >
                <span style={{ whiteSpace: "nowrap" }}>{page.label}</span>
                <span
                  style={{
                    color: "var(--m-text-secondary)",
                    fontSize: 13,
                    fontWeight: 400,
                    textAlign: "right",
                  }}
                >
                  {page.alias ?? (page.derived ? `${page.description} · not built yet` : page.description)}
                </span>
              </Link>
            ))}
          </section>
        ))}
      </div>
    </div>
  );
}
