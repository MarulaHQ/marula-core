"use client";
/**
 * LinkedRecordsStrip — a ticket's linked configurations, one row each.
 *
 * Purpose: §6.5 of `docs/reference/connector-surface.md`. A ticket shows the
 * configurations it involves above its activity timeline, because that is how
 * a technician thinks about the work ("the printer at their second site").
 *
 * The rules the component enforces:
 *
 * - **Each row is one configuration.** Chips are never merged across rows: a
 *   merged row hides which device the next press is about to touch, which is
 *   the one thing this strip exists to make obvious.
 * - **It is a summary, not a table.** More than `max` rows collapse to `max`
 *   plus `Show all (N)`, and the expanded list can be folded back. A ticket
 *   with forty linked devices must not push the conversation off the screen.
 * - **`hidden` is a real prop.** A client with no configurations should not
 *   see an empty strip on every ticket; the caller decides whether the
 *   surface could plausibly attach one and passes `hidden`. The empty state
 *   renders once, with the link action, only when the caller asks for it.
 *
 * Non-purpose: no search, no linking flow and no fetching. `onAdd` opens the
 * picker the ticket owns; `onLinkClick` opens the configuration. The chips are
 * the shared `CapabilityChip`, so a third-party connector reaches this strip
 * with no change here.
 *
 * Design research (Calendly, Chili Piper): one row per item, one action, and
 * "add another" sits at the end of the list rather than in a separate toolbar.
 * The strip follows that shape — its add control is part of the list header,
 * and the rows stay rows instead of becoming a table with columns per vendor.
 * The share-dialog references contribute the refusal to hide a choice: every
 * provider stays visible in its chip's menu even while one is degraded.
 */
import { useId, useState, type ReactNode } from "react";
import { Button } from "./button";
import { CapabilityChip, type CapabilityProvider } from "./capability-chip";
import { EmptyState } from "./empty-state";
import { cn } from "../lib/cn";

export interface LinkedRecordCapability {
  /** The capability kind the row's chips are grouped by (`rmm`, `dms`, …). */
  kind: string;
  providers: CapabilityProvider[];
}

export interface LinkedRecord {
  id: string;
  /** The configuration's name, e.g. "Dell Latitude 5540". */
  name: string;
  /** Its external id or serial, shown in the data face. */
  subtitle?: string;
  /** One entry per capability; each entry renders one chip. */
  capabilityGroups: LinkedRecordCapability[];
}

export interface LinkedRecordsStripProps {
  records: LinkedRecord[];
  /** Rows shown before the strip collapses. Defaults to 4 (§6.5). */
  max?: number;
  /**
   * Suppresses the whole strip. A ticket for a client with no configurations
   * passes this instead of rendering an empty frame on every ticket.
   */
  hidden?: boolean;
  onLinkClick?: (recordId: string) => void;
  /** Opens the ticket's link picker. The strip never invents one. */
  onAdd: () => void;
  /** Passed to every chip; see `CapabilityChip`. */
  onLaunch?: (connectionId: string, actionKind: "open" | "launch") => void;
  onFix?: (connectionId: string) => void;
  /** Heading noun. Defaults to "Configurations". */
  title?: string;
  className?: string;
}

export function LinkedRecordsStrip({
  records,
  max = 4,
  hidden = false,
  onLinkClick,
  onAdd,
  onLaunch,
  onFix,
  title = "Configurations",
  className,
}: LinkedRecordsStripProps): ReactNode {
  const [expanded, setExpanded] = useState(false);
  const headingId = `${useId()}-heading`;

  if (hidden) return null;

  const limit = Math.max(1, max);
  const visible = expanded ? records : records.slice(0, limit);
  const collapsible = records.length > limit;

  return (
    <section
      aria-labelledby={headingId}
      className={cn(className)}
      data-testid="linked-records-strip"
      style={{ display: "grid", gap: 8 }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "baseline",
          justifyContent: "space-between",
          gap: 12,
        }}
      >
        <h2
          id={headingId}
          style={{ margin: 0, fontSize: 13, fontWeight: 600, color: "var(--m-text)" }}
        >
          {title}
          <span style={{ marginLeft: 6, color: "var(--m-text-muted)", fontWeight: 400 }}>
            ({records.length})
          </span>
        </h2>
        {records.length === 0 ? null : (
          <Button size="sm" variant="ghost" onClick={onAdd} data-testid="linked-records-add">
            + Link configuration
          </Button>
        )}
      </div>

      {records.length === 0 ? (
        <EmptyState
          message="No configurations linked yet. Link the device or service this ticket is about."
          action={
            <Button size="sm" variant="secondary" onClick={onAdd}>
              Link configuration
            </Button>
          }
        />
      ) : (
        <>
          <ul
            style={{
              listStyle: "none",
              margin: 0,
              padding: 0,
              display: "grid",
              gap: 6,
            }}
          >
            {visible.map((record) => (
              <li
                key={record.id}
                data-testid={`linked-record-${record.id}`}
                style={{
                  display: "flex",
                  flexWrap: "wrap",
                  alignItems: "center",
                  gap: 10,
                  minHeight: "var(--m-density-row)",
                  padding: "4px 2px",
                  borderBottom: "1px solid var(--m-border)",
                }}
              >
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "baseline",
                    gap: 8,
                    minWidth: 0,
                    marginRight: "auto",
                  }}
                >
                  <button
                    type="button"
                    onClick={() => onLinkClick?.(record.id)}
                    disabled={onLinkClick === undefined}
                    style={{
                      padding: 0,
                      border: 0,
                      background: "none",
                      color: onLinkClick === undefined ? "var(--m-text)" : "var(--m-accent-text)",
                      font: "inherit",
                      fontSize: 13,
                      fontWeight: 600,
                      textAlign: "left",
                      textDecoration: onLinkClick === undefined ? "none" : "underline",
                      cursor: onLinkClick === undefined ? "default" : "pointer",
                    }}
                  >
                    {record.name}
                  </button>
                  {record.subtitle !== undefined ? (
                    <span
                      style={{
                        fontFamily: "var(--m-font-mono)",
                        fontSize: 11,
                        color: "var(--m-text-muted)",
                        fontVariantNumeric: "tabular-nums",
                      }}
                    >
                      {record.subtitle}
                    </span>
                  ) : null}
                </span>

                <span style={{ display: "inline-flex", flexWrap: "wrap", gap: 6 }}>
                  {record.capabilityGroups.map((group) => (
                    <CapabilityChip
                      key={`${record.id}-${group.kind}`}
                      kind={group.kind}
                      providers={group.providers}
                      onLaunch={onLaunch}
                      onFix={onFix}
                    />
                  ))}
                </span>
              </li>
            ))}
          </ul>

          {collapsible ? (
            <div>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setExpanded((current) => !current)}
                aria-expanded={expanded}
                data-testid="linked-records-show-all"
              >
                {expanded ? "Show fewer" : `Show all (${records.length})`}
              </Button>
            </div>
          ) : null}
        </>
      )}
    </section>
  );
}
