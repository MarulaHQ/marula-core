// Workbench v2 — the ticket activity stream (Wave 15 lane 7).
//
// Purpose: the mockup's timeline rebuilt on real data. One chronological
// stream merged from the sources the API actually exposes — comments,
// audit events, time entries, the email ledger — with the mockup's grid
// grammar: time gutter (right-aligned mono), per-kind icon column
// (client / internal / system / time / agent), and bubble variants
// (client tint, agent tint, neutral internal). Filter chips ride the
// same dimensions as the classic timeline (kind + visibility). New rows
// and filter changes stagger in (transform/opacity, ≤120ms, ≤40ms/item).
// Non-purpose: no authoring (the floating composer owns writes); no
// agent data fabrication — the agent row renders only real agent events
// when the ledger carries them.

"use client";

import type { components } from "@meridian/sdk";
import { Spinner } from "@meridian/ui";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api, downloadAttachment } from "../../lib/api";
import type { Attachment, TicketComment } from "../../lib/instant/read-models";

type AuditEvent = components["schemas"]["AuditEvent"];
type TimeEntry = components["schemas"]["TimeEntry"];
type TicketEmailRow = components["schemas"]["TicketEmailRow"];

export type WbFilter = "all" | "notes" | "client" | "internal" | "system" | "time" | "agent";

type WbKind = "client" | "internal" | "system" | "time" | "agent";

interface WbEntry {
  id: string;
  at: string;
  kind: WbKind;
  /** Visibility dimension for the chips row (notes only). */
  visibility: "internal" | "client_visible" | null;
  title: string;
  org: string | null;
  tag: string;
  body: string | null;
  /** Muted second line (device/source/delivery meta). */
  meta: string | null;
  attachments: Attachment[];
  /** Rendered as the time-entry summary line (minutes + billable). */
  timeLabel: string | null;
  billable: boolean | null;
}

const FILTERS: { value: WbFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "client", label: "Client" },
  { value: "internal", label: "Internal" },
  { value: "system", label: "System" },
  { value: "time", label: "Time" },
  { value: "agent", label: "Agent" },
];

function kindIcon(kind: WbKind): ReactNode {
  if (kind === "client") {
    return (
      <svg aria-hidden width="12" height="12" viewBox="0 0 18 18" fill="none">
        <circle cx="9" cy="6" r="2.75" stroke="currentColor" strokeWidth="1.6" />
        <path d="M3.5 14.5c.6-2.6 2.9-4 5.5-4s4.9 1.4 5.5 4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      </svg>
    );
  }
  if (kind === "system") {
    return (
      <svg aria-hidden width="12" height="12" viewBox="0 0 18 18" fill="none">
        <circle cx="9" cy="9" r="2.4" stroke="currentColor" strokeWidth="1.6" />
        <path d="M9 2.6v2M9 13.4v2M2.6 9h2M13.4 9h2M4.4 4.4l1.4 1.4M12.2 12.2l1.4 1.4M13.6 4.4l-1.4 1.4M5.8 12.2 4.4 13.6" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      </svg>
    );
  }
  if (kind === "time") {
    return (
      <svg aria-hidden width="12" height="12" viewBox="0 0 18 18" fill="none">
        <circle cx="9" cy="9" r="6" stroke="currentColor" strokeWidth="1.6" />
        <path d="M9 5.5V9l2.4 1.6" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      </svg>
    );
  }
  if (kind === "agent") {
    return (
      <svg aria-hidden width="12" height="12" viewBox="0 0 18 18" fill="none">
        <path d="M9 2.5 10.6 7.4 15.5 9 10.6 10.6 9 15.5 7.4 10.6 2.5 9 7.4 7.4Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
      </svg>
    );
  }
  return (
    <svg aria-hidden width="12" height="12" viewBox="0 0 18 18" fill="none">
      <path d="M5.5 7h7M5.5 10h4.5" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      <rect x="2.5" y="3.5" width="13" height="11" rx="1.5" stroke="currentColor" strokeWidth="1.6" />
    </svg>
  );
}

function timeLabel(at: string): string {
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function dayLabel(at: string): string | null {
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString([], { weekday: "long", month: "short", day: "numeric" });
}

const EVENT_ACTION_LABEL: Record<string, string> = {
  "ticket.status_change": "Status changed",
  "ticket.board_move": "Moved to another board",
  "ticket.created": "Ticket created",
  "ticket.checklist_updated": "Checklist updated",
};

export interface WbTimelineProps {
  ticketId: string;
  comments: TicketComment[];
  attachments: Attachment[];
  namesByUser: Map<string, string>;
}

export function WbTimeline({ ticketId, comments, attachments, namesByUser }: WbTimelineProps): ReactNode {
  const [filter, setFilter] = useState<WbFilter>("all");
  const [events, setEvents] = useState<AuditEvent[] | null>(null);
  const [entries, setEntries] = useState<TimeEntry[] | null>(null);
  const [emails, setEmails] = useState<TicketEmailRow[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoadError(null);
    void (async () => {
      const [audit, time, mail] = await Promise.all([
        api.api.GET("/api/v1/audit-events", { params: { query: { action: "ticket.", limit: 200 } } }),
        api.api.GET("/api/v1/tickets/{id}/time-entries", { params: { path: { id: ticketId } } }),
        api.api.GET("/api/v1/tickets/{id}/emails", { params: { path: { id: ticketId } } }),
      ]);
      if (cancelled) return;
      if (audit.data === undefined) setLoadError("Could not load ticket events");
      else setEvents((audit.data.events ?? []).filter((e) => e.entity_type === "ticket" && e.entity_id === ticketId));
      setEntries(time.data === undefined ? null : (time.data ?? []));
      setEmails(mail.data === undefined ? null : (mail.data.messages ?? []));
    })();
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  const attachmentsFor = (commentId: string | null): Attachment[] =>
    attachments.filter((a) =>
      commentId === null ? a.ticket_comment_id === null || a.ticket_comment_id === undefined : a.ticket_comment_id === commentId,
    );

  const stream = useMemo<WbEntry[]>(() => {
    const rows: WbEntry[] = [];
    for (const c of comments) {
      const clientVisible = c.kind === "client_visible";
      rows.push({
        id: `note-${c.id}`,
        at: c.created_at,
        kind: clientVisible ? "client" : "internal",
        visibility: clientVisible ? "client_visible" : "internal",
        title: namesByUser.get(c.author_id ?? "") ?? "Unknown author",
        org: clientVisible ? "(client-visible)" : "(internal)",
        tag: clientVisible ? "CLIENT" : "INTERNAL",
        body: c.body,
        meta: null,
        attachments: attachmentsFor(c.id),
        timeLabel: null,
        billable: null,
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
        kind: "system",
        visibility: "internal",
        title: "System Event",
        org: null,
        tag: "SYSTEM",
        body: label + (from !== null && to !== null ? `: ${from} → ${to}` : ""),
        meta: null,
        attachments: [],
        timeLabel: null,
        billable: null,
      });
    }
    for (const t of entries ?? []) {
      rows.push({
        id: `time-${t.id}`,
        at: t.started_at,
        kind: "time",
        visibility: "internal",
        title: "Time Entry",
        org: namesByUser.get(t.user_id) ?? null,
        tag: "TIME",
        body: t.notes ?? null,
        meta: null,
        attachments: [],
        timeLabel: `${t.minutes} min`,
        billable: t.billable,
      });
    }
    for (const m of emails ?? []) {
      const inbound = m.direction === "inbound";
      const parts: string[] = [];
      if (inbound) parts.push(`from ${m.from_address}`);
      else {
        parts.push(`to ${m.to_addresses.join(", ")}`);
        if (m.delivery !== null && m.delivery !== undefined) {
          parts.push(`delivery: ${m.delivery.status}`);
        }
      }
      if (m.scan_status === "infected") parts.push("SCAN: infected");
      rows.push({
        id: `email-${m.id}`,
        at: m.created_at,
        kind: inbound ? "client" : "system",
        visibility: "internal",
        title: inbound ? "Email in" : "Email out",
        org: null,
        tag: "EMAIL",
        body: m.body_text ?? null,
        meta: `${m.subject ?? "(no subject)"} · ${parts.join(" · ")}`,
        attachments: [],
        timeLabel: null,
        billable: null,
      });
    }
    rows.sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0));
    return rows;
    // attachmentsFor is stable per attachments identity
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [comments, events, entries, emails, namesByUser, attachments]);

  const filtered = stream.filter((row) => {
    if (filter === "all") return true;
    if (filter === "notes") return row.kind === "client" || row.kind === "internal";
    return row.kind === filter;
  });

  let lastDay: string | null = null;

  return (
    <section aria-label="Ticket activity" data-testid="wb-timeline" className="">
      <div className="mb-3 flex flex-wrap items-center gap-1.5" role="group" aria-label="Filter activity">
        {FILTERS.map((f) => {
          const active = filter === f.value;
          return (
            <button
              key={f.value}
              type="button"
              aria-pressed={active}
              data-testid={`wb-filter-${f.value}`}
              className="m-wb-chip cursor-pointer"
              style={active ? undefined : { color: "var(--m-text-muted)", background: "var(--m-surface)", borderColor: "var(--m-border)" }}
              onClick={() => setFilter(f.value)}
            >
              {f.label}
            </button>
          );
        })}
      </div>

      {loadError !== null && (
        <p className="mb-2 text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">{loadError}</p>
      )}
      {events === null && entries === null && emails === null ? (
        <div className="flex justify-center py-8"><Spinner /></div>
      ) : filtered.length === 0 ? (
        <p className="py-6 text-center text-xs" style={{ color: "var(--m-text-muted)" }} data-testid="wb-timeline-empty">
          No activity matches the current filter.
        </p>
      ) : (
        <ol className="m-wb-timeline relative">
          {filtered.map((row, i) => {
            const day = dayLabel(row.at);
            const showDay = day !== null && day !== lastDay;
            lastDay = day;
            return (
              <li key={row.id}>
                {showDay && (
                  <p className="m-wb-day mb-2 mt-4 text-[10px] font-bold uppercase tracking-[0.075em]" style={{ color: "var(--m-text-muted)" }}>
                    {day}
                  </p>
                )}
                <article
                  className="m-wb-event relative mb-3.5 grid"
                  style={{ "--stagger-i": Math.min(i, 8) } as React.CSSProperties}
                  data-testid={`wb-row-${row.kind}`}
                >
                  <div className="m-wb-event-time pt-1 text-right text-[10px]" style={{ color: "var(--m-text-muted)", fontVariantNumeric: "tabular-nums" }}>
                    {timeLabel(row.at)}
                  </div>
                  <div
                    className={`m-wb-event-icon z-10 grid place-items-center rounded-full border`}
                    data-kind={row.kind}
                    aria-hidden="true"
                  >
                    {kindIcon(row.kind)}
                  </div>
                  <div className="min-w-0">
                    <div className="mb-1 flex flex-wrap items-baseline gap-1.5">
                      <strong className="text-[12px]" style={{ color: "var(--m-text)" }}>{row.title}</strong>
                      {row.org !== null && (
                        <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>{row.org}</span>
                      )}
                      <span className="ml-auto text-[9px] font-bold tracking-[0.04em]" style={{ color: "var(--m-text-muted)" }}>
                        {row.tag}
                      </span>
                    </div>
                    {row.timeLabel !== null ? (
                      <div className="m-wb-bubble" data-variant="time">
                        <strong className="font-mono">{row.timeLabel}</strong>
                        {row.body !== null && row.body !== "" ? <span> · {row.body}</span> : null}
                        <span className="float-right text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                          {row.billable === true ? "Billable" : row.billable === false ? "Non-billable" : ""}
                        </span>
                      </div>
                    ) : (
                      <div className="m-wb-bubble" data-variant={row.kind}>
                        {row.body !== null && <div className="whitespace-pre-wrap">{row.body}</div>}
                        {row.meta !== null && (
                          <div className="mt-1 text-[10.5px]" style={{ color: "var(--m-text-muted)" }}>{row.meta}</div>
                        )}
                        {row.attachments.length > 0 && (
                          <div className="mt-2 flex flex-wrap gap-1.5">
                            {row.attachments.map((a) => (
                              <button
                                key={a.id}
                                type="button"
                                className="m-wb-attachment cursor-pointer"
                                onClick={() => void downloadAttachment(a.id, a.filename)}
                                title={`${a.filename} · ${Math.max(1, Math.round(a.size_bytes / 1024))} KB`}
                              >
                                <svg aria-hidden width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                                  <path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" />
                                </svg>
                                {a.filename}
                                <span style={{ color: "var(--m-text-muted)" }}>{Math.max(1, Math.round(a.size_bytes / 1024))} KB</span>
                              </button>
                            ))}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                </article>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
