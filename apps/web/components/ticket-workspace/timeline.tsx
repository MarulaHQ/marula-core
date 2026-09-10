// Unified ticket timeline — one chronological activity stream.
//
// Purpose: merge the sources the API actually exposes for one ticket:
// (a) structured notes (comments, badged internal / client-visible /
// in-house), (b) key ticket events from the audit trail (status changes,
// board moves — action prefix `ticket.`, filtered to this ticket's id),
// and (c) time-entry activity from the ticket's time-entries read. The
// stream is filterable by kind (all / notes / events / time) and by
// visibility (all / internal-only / client-visible-only). Email rows in
// AND out have NO read endpoint on the current API surface (emails are
// recorded and fanned out server-side; nothing lists them per ticket), so
// that section renders a typed "not available yet" state — never fake
// data; recorded as a backend follow-up.
// Non-purpose: no authoring here (the notes composer owns writes).

"use client";

import type { components } from "@meridian/sdk";
import { Spinner, StatusTag } from "@meridian/ui";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import type { TicketComment } from "../../lib/instant/read-models";

type AuditEvent = components["schemas"]["AuditEvent"];
type TimeEntry = components["schemas"]["TimeEntry"];

export type TimelineFilterKind = "all" | "notes" | "events" | "time";
export type TimelineFilterVisibility = "all" | "internal" | "client_visible";

const NOTE_KIND_LABEL: Record<string, string> = {
  internal: "Internal",
  client_visible: "Client-visible",
  in_house_update: "In-house update",
};

const EVENT_ACTION_LABEL: Record<string, string> = {
  "ticket.status_change": "Status changed",
  "ticket.board_move": "Moved to another board",
  "ticket.created": "Ticket created",
};

function kindBadge(kind: string): "neutral" | "success" | "warning" {
  if (kind === "client_visible") return "success";
  if (kind === "in_house_update") return "warning";
  return "neutral";
}

interface TimelineEntry {
  id: string;
  at: string;
  kind: "notes" | "events" | "time";
  /** Visibility dimension, applied only to notes. */
  visibility: "internal" | "client_visible" | null;
  title: string;
  detail?: string | null;
  badge?: string | null;
  badgeTone?: "neutral" | "success" | "warning";
}

export interface TimelineProps {
  ticketId: string;
  comments: TicketComment[];
  /** Org directory rows keyed by user id (author display names). */
  namesByUser: Map<string, string>;
}

export function TicketTimeline({ ticketId, comments, namesByUser }: TimelineProps): ReactNode {
  const [kindFilter, setKindFilter] = useState<TimelineFilterKind>("all");
  const [visFilter, setVisFilter] = useState<TimelineFilterVisibility>("all");
  const [events, setEvents] = useState<AuditEvent[] | null>(null);
  const [entries, setEntries] = useState<TimeEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Key events + time activity: plain SDK reads scoped to this ticket.
  // Audit rows arrive org-wide for the `ticket.` action prefix; we narrow
  // client-side to this ticket's entity id (no entity filter param exists).
  useEffect(() => {
    let cancelled = false;
    setLoadError(null);
    const run = async (): Promise<void> => {
      const [audit, time] = await Promise.all([
        api.api.GET("/api/v1/audit-events", { params: { query: { action: "ticket.", limit: 200 } } }),
        api.api.GET("/api/v1/tickets/{id}/time-entries", { params: { path: { id: ticketId } } }),
      ]);
      if (cancelled) return;
      if (audit.data === undefined) {
        setLoadError("Could not load ticket events");
      } else {
        setEvents(
          (audit.data.events ?? []).filter(
            (e) => e.entity_type === "ticket" && e.entity_id === ticketId,
          ),
        );
      }
      setEntries(time.data === undefined ? null : (time.data ?? []));
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  const stream = useMemo<TimelineEntry[]>(() => {
    const rows: TimelineEntry[] = [];
    for (const c of comments) {
      rows.push({
        id: `note-${c.id}`,
        at: c.created_at,
        kind: "notes",
        visibility: c.kind === "client_visible" ? "client_visible" : "internal",
        badge: NOTE_KIND_LABEL[c.kind] ?? c.kind,
        badgeTone: kindBadge(c.kind),
        title: namesByUser.get(c.author_id ?? "") ?? "Unknown author",
        detail: c.body,
      });
    }
    for (const e of events ?? []) {
      const label = EVENT_ACTION_LABEL[e.action] ?? e.action;
      const detail = e.detail as Record<string, unknown> | null;
      const from = typeof detail?.from_name === "string" ? detail.from_name : null;
      const to = typeof detail?.to_name === "string" ? detail.to_name : null;
      rows.push({
        id: `event-${e.id}`,
        at: e.occurred_at,
        kind: "events",
        visibility: "internal",
        badge: "Event",
        badgeTone: "neutral",
        title: label + (from !== null && to !== null ? `: ${from} → ${to}` : ""),
        detail: e.ip === null ? null : undefined,
      });
    }
    for (const t of entries ?? []) {
      rows.push({
        id: `time-${t.id}`,
        at: t.started_at,
        kind: "time",
        visibility: "internal",
        badge: t.billable ? "Time (billable)" : "Time (non-billable)",
        badgeTone: "neutral",
        title: `${Math.round(t.minutes)} min · ${namesByUser.get(t.user_id) ?? "Unknown"}`,
        detail: t.notes,
      });
    }
    rows.sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0));
    return rows;
  }, [comments, events, entries, namesByUser]);

  const filtered = stream.filter((row) => {
    if (kindFilter !== "all" && row.kind !== kindFilter) return false;
    if (visFilter !== "all" && row.visibility !== visFilter) return false;
    return true;
  });

  return (
    <section className="grid content-start gap-2" data-testid="ticket-timeline" aria-label="Ticket timeline">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Timeline
        </h2>
        <div className="flex flex-wrap gap-1" role="group" aria-label="Filter by kind">
          {(
            [
              ["all", "All"],
              ["notes", "Notes"],
              ["events", "Events"],
              ["time", "Time"],
            ] as const
          ).map(([value, label]) => (
            <button
              key={value}
              type="button"
              className="rounded-[2px] border px-2 py-0.5 text-[11px] transition-opacity duration-100"
              style={{
                borderColor: kindFilter === value ? "var(--m-accent)" : "var(--m-border)",
                color: kindFilter === value ? "var(--m-accent)" : "var(--m-text-muted)",
                opacity: kindFilter === value ? 1 : 0.85,
              }}
              data-testid={`timeline-filter-${value}`}
              aria-pressed={kindFilter === value}
              onClick={() => setKindFilter(value)}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap gap-1" role="group" aria-label="Filter by visibility">
          {(
            [
              ["all", "Any visibility"],
              ["internal", "Internal"],
              ["client_visible", "Client-visible"],
            ] as const
          ).map(([value, label]) => (
            <button
              key={value}
              type="button"
              className="rounded-[2px] border px-2 py-0.5 text-[11px] transition-opacity duration-100"
              style={{
                borderColor: visFilter === value ? "var(--m-accent)" : "var(--m-border)",
                color: visFilter === value ? "var(--m-accent)" : "var(--m-text-muted)",
                opacity: visFilter === value ? 1 : 0.85,
              }}
              data-testid={`timeline-visibility-${value}`}
              aria-pressed={visFilter === value}
              onClick={() => setVisFilter(value)}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <TypedEmailGap />

      {loadError !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">{loadError}</p>
      )}
      {events === null && entries === null ? (
        <Spinner />
      ) : (
        <ol className="grid gap-1.5">
          {filtered.map((row) => (
            <li
              key={row.id}
              className="grid gap-0.5 rounded-[2px] border p-2 text-xs"
              style={{ borderColor: "var(--m-border)" }}
              data-testid={`timeline-row-${row.kind}`}
            >
              <div className="flex flex-wrap items-center gap-2">
                {row.badge !== null && row.badge !== undefined && (
                  <StatusTag tone={row.badgeTone ?? "neutral"}>{row.badge}</StatusTag>
                )}
                <span className="font-medium">{row.title}</span>
                <time className="ml-auto" style={{ color: "var(--m-text-muted)" }}>
                  {new Date(row.at).toLocaleString()}
                </time>
              </div>
              {row.detail != null && row.detail !== "" && (
                <p className="whitespace-pre-wrap" style={{ color: "var(--m-text-secondary)" }}>
                  {row.detail}
                </p>
              )}
            </li>
          ))}
          {filtered.length === 0 && (
            <li className="px-2 py-1 text-xs" style={{ color: "var(--m-text-muted)" }}>
              No activity matches the current filters.
            </li>
          )}
        </ol>
      )}
    </section>
  );
}

/**
 * Typed empty state for the email legs of the timeline. The API records
 * and transports email in/out (2.4 path) but exposes no per-ticket email
 * read endpoint, so the rows cannot be rendered without inventing data.
 */
function TypedEmailGap(): ReactNode {
  return (
    <p
      className="rounded-[2px] border border-dashed px-2 py-1.5 text-[11px]"
      style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
      data-testid="timeline-email-gap"
    >
      Email in/out rows are not available yet — the API records and sends
      ticket email but has no per-ticket email read endpoint yet. Notes and
      key events below are live.
    </p>
  );
}
