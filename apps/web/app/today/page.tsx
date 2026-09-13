// /today — the my-work surface.
//
// Purpose: the cross-board priority queue assigned to me (TASKLIST 2.11):
// compact 24px rows — mono ticket number, squared client tag, title,
// source board, right-aligned SLA countdown in tabular numerals — ordered
// server-side by priority then earliest of SLA resolution/manual due.
// Plus the missing-time strip (yesterday's activity-vs-entries audit with
// a quick-log dialog) and the docked timer working set. Matches
// design/mockups/today-light.jpeg and today-dark.png: dense table, quiet
// chrome, status color only where it carries meaning.
// Non-purpose: no board management or admin surfaces here; timer controls
// live on the ticket workspace and the layout-level dock.
"use client";

import { friendlyMutationText } from "../../lib/error-copy";
import type { components } from "@meridian/sdk";
import { Button, Spinner, Tag } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { BoardNav } from "../../components/board-nav";
import { api, useRequireAuth } from "../../lib/api";

type MyWorkItem = components["schemas"]["MyWorkItem"];
type GapReport = components["schemas"]["GapReportResponse"];

function PriorityTag({ priority }: { priority: string }) {
  const tone =
    priority === "p1" ? "danger" : priority === "p2" ? "warning" : "neutral";
  return (
    <span
      className="px-1 text-[10px] font-medium uppercase"
      style={{
        border: "1px solid var(--m-border)",
        color:
          tone === "danger"
            ? "var(--m-status-danger-fg)"
            : tone === "warning"
              ? "var(--m-status-warning-fg)"
              : "var(--m-text-muted)",
      }}
    >
      {priority.toUpperCase()}
    </span>
  );
}

function ClientTag({ name }: { name: string }) {
  // Squared client tag per mockup: initials derived from the client name,
  // outline tone so it stays quiet against status color.
  const initials = name
    .split(/\s+/)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
  return (
    <Tag tone="outline" title={name}>
      {initials}
    </Tag>
  );
}

/// Compact relative countdown ("2d 4h", "3h", "overdue 5h") from now.
function dueLabel(iso: string): { text: string; overdue: boolean } {
  const due = new Date(iso).getTime();
  const diffMs = due - Date.now();
  const mins = Math.round(Math.abs(diffMs) / 60_000);
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const span = d > 0 ? `${d}d ${h}h` : h > 0 ? `${h}h` : `${mins}m`;
  return diffMs < 0
    ? { text: `overdue ${span}`, overdue: true }
    : { text: span, overdue: false };
}

function fmtMins(mins: number): string {
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h ${m}m`;
}

function yesterdayIso(): string {
  const d = new Date();
  d.setUTCDate(d.getUTCDate() - 1);
  return d.toISOString().slice(0, 10);
}

export default function TodayPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [items, setItems] = useState<MyWorkItem[] | null>(null);
  const [gap, setGap] = useState<GapReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [logOpen, setLogOpen] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [mine, gaps] = await Promise.all([
        api.api.GET("/api/v1/tickets/my-work"),
        api.api.GET("/api/v1/time-gaps/me"),
      ]);
      if (mine.error !== undefined) throw new Error("failed to load tickets");
      setItems(mine.data);
      if (gaps.data) setGap(gaps.data);
      setError(null);
    } catch (err) {
      setError(friendlyMutationText(err, "Load failed"));
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // missing_time nudges land here: /today?log=1 opens the quick-log dialog.
  useEffect(() => {
    if (
      auth.status === "authed" &&
      new URLSearchParams(window.location.search).get("log") === "1"
    ) {
      setLogOpen(true);
    }
  }, [auth.status]);

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  const day = gap?.days.find((d) => d.has_gap) ?? gap?.days[0];

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <BoardNav />
        <main
          className="flex min-w-0 flex-1 flex-col gap-4 overflow-auto p-4"
          style={{ borderLeft: "1px solid var(--m-border)" }}
        >
          <div className="flex items-center justify-between">
            <h1 className="text-sm font-semibold">Today</h1>
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              {items?.length ?? 0} open · assigned to you · across all boards
            </span>
          </div>

          {/* Missing-time strip: yesterday's activity-vs-entries audit */}
          {day !== undefined && day.activity && (
            <section
              className="rounded-[2px] border p-3"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
              aria-label="Missing time"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="text-[13px]" style={{ color: "var(--m-text)" }}>
                  <span className="font-semibold">Yesterday:</span>{" "}
                  {fmtMins(day.logged_minutes)} logged of{" "}
                  {fmtMins(day.expected_minutes)} expected
                  {day.has_gap ? (
                    <>
                      {" — "}
                      <span style={{ color: "var(--m-status-warning-fg, var(--m-text))" }}>
                        {fmtMins(day.unaccounted_minutes)} unaccounted
                      </span>
                    </>
                  ) : null}
                </p>
                <Button size="sm" onClick={() => setLogOpen(true)}>
                  Log time
                </Button>
              </div>
              <div
                className="mt-2 flex h-3 gap-[2px] overflow-hidden rounded-[2px]"
                style={{ background: "var(--m-canvas)" }}
                aria-hidden="true"
              >
                {Array.from({ length: 8 }).map((_, i) => {
                  const filled =
                    day.logged_minutes >= (day.expected_minutes / 8) * (i + 1);
                  return (
                    <span
                      key={i}
                      className="h-full flex-1"
                      style={{
                        background: filled
                          ? "var(--m-accent-muted, var(--m-border))"
                          : "transparent",
                        border: "1px solid var(--m-border)",
                      }}
                    />
                  );
                })}
              </div>
            </section>
          )}

          {error !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {error}
            </p>
          )}

          {items === null ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : items.length === 0 ? (
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              Nothing assigned to you yet. Tickets appear here highest-priority first.
            </p>
          ) : (
            <ol role="list" aria-label="My work queue" className="flex flex-col">
              {items.map((t) => {
                const dueIso = t.sla_resolution_due ?? t.due_at;
                const due = dueIso !== null && dueIso !== undefined ? dueLabel(dueIso) : null;
                return (
                  <li key={t.id}>
                    <button
                      type="button"
                      onClick={() => router.push(`/tickets/${t.id}`)}
                      className="row-hover grid h-6 w-full grid-cols-[auto_auto_1fr_auto_auto_auto] items-center gap-2 text-left text-xs"
                    >
                      <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
                        {t.number}
                      </span>
                      <PriorityTag priority={t.priority} />
                      <span className="truncate">{t.title}</span>
                      <span className="flex items-center gap-2">
                        {t.client_name !== null && t.client_name !== undefined && (
                          <ClientTag name={t.client_name} />
                        )}
                      </span>
                      <span className="truncate text-xs" style={{ color: "var(--m-text-muted)" }}>
                        {t.board_name}
                      </span>
                      <span
                        className="text-right tabular-nums"
                        style={{
                          color:
                            due?.overdue === true
                              ? "var(--m-status-danger-fg)"
                              : "var(--m-text-secondary)",
                          minWidth: "6rem",
                        }}
                        title={
                          dueIso !== null && dueIso !== undefined
                            ? `Due ${new Date(dueIso).toLocaleString()}`
                            : undefined
                        }
                      >
                        {due === null ? "—" : due.text}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ol>
          )}
        </main>
      </div>

      {/* Quick-log dialog for closing a detected gap */}
      {logOpen && items !== null && (
        <QuickLogDialog items={items} onClose={() => { setLogOpen(false); void refresh(); }} />
      )}
    </div>
  );
}

/// Minimal modal shell for the quick-log form (no portal dependency).
function QuickLogDialog({
  items,
  onClose,
}: {
  items: MyWorkItem[];
  onClose: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
      role="dialog"
      aria-modal="true"
      aria-label="Log missing time"
    >
      <div
        className="w-full max-w-md rounded-[2px] border p-4"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
        <QuickLogForm items={items} onDone={onClose} />
      </div>
    </div>
  );
}

/// Quick manual entry for closing a missing-time gap: defaults to
/// yesterday 09:00 + 60 minutes on the first queue item.
function QuickLogForm({
  items,
  onDone,
}: {
  items: MyWorkItem[];
  onDone: () => void;
}) {
  const [ticketId, setTicketId] = useState(items[0]?.id ?? "");
  const [minutes, setMinutes] = useState("60");
  const [startTime, setStartTime] = useState("09:00");
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    const mins = Number.parseInt(minutes, 10);
    if (!ticketId || !Number.isFinite(mins) || mins <= 0) {
      setError("Pick a ticket and a positive minute count.");
      return;
    }
    const day = yesterdayIso();
    const started = new Date(`${day}T${startTime}:00`);
    if (Number.isNaN(started.getTime())) {
      setError("Invalid start time.");
      return;
    }
    setBusy(true);
    try {
      const { error: apiError } = await api.api.POST("/api/v1/time-entries", {
        body: {
          ticket_id: ticketId,
          started_at: started.toISOString(),
          ended_at: new Date(started.getTime() + mins * 60_000).toISOString(),
          billable: true,
          notes: notes === "" ? null : notes,
        },
      });
      if (apiError !== undefined) throw new Error("could not save the entry");
      onDone();
    } catch (err) {
      setError(friendlyMutationText(err, "Save failed"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1">
        <label className="text-xs" htmlFor="log-ticket" style={{ color: "var(--m-text-secondary)" }}>
          Ticket
        </label>
        <select
          id="log-ticket"
          value={ticketId}
          onChange={(e) => setTicketId(e.target.value)}
          className="rounded-[2px] px-2 py-1 text-xs"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
        >
          {items.map((t) => (
            <option key={t.id} value={t.id}>
              {t.number} — {t.title}
            </option>
          ))}
        </select>
      </div>
      <div className="flex gap-3">
        <div className="flex flex-1 flex-col gap-1">
          <label className="text-xs" htmlFor="log-date" style={{ color: "var(--m-text-secondary)" }}>
            Date
          </label>
          <input
            id="log-date"
            readOnly
            value={yesterdayIso()}
            className="rounded-[2px] px-2 py-1 text-xs"
            style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
          />
        </div>
        <div className="flex flex-1 flex-col gap-1">
          <label className="text-xs" htmlFor="log-start" style={{ color: "var(--m-text-secondary)" }}>
            Start
          </label>
          <input
            id="log-start"
            type="time"
            value={startTime}
            onChange={(e) => setStartTime(e.target.value)}
            className="rounded-[2px] px-2 py-1 text-xs"
            style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
          />
        </div>
        <div className="flex flex-1 flex-col gap-1">
          <label className="text-xs" htmlFor="log-minutes" style={{ color: "var(--m-text-secondary)" }}>
            Minutes
          </label>
          <input
            id="log-minutes"
            type="number"
            min={1}
            value={minutes}
            onChange={(e) => setMinutes(e.target.value)}
            className="rounded-[2px] px-2 py-1 text-xs"
            style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
          />
        </div>
      </div>
      <div className="flex flex-col gap-1">
        <label className="text-xs" htmlFor="log-notes" style={{ color: "var(--m-text-secondary)" }}>
          Notes
        </label>
        <input
          id="log-notes"
          value={notes}
          placeholder="What did you work on?"
          onChange={(e) => setNotes(e.target.value)}
          className="rounded-[2px] px-2 py-1 text-xs"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
        />
      </div>
      {error !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button size="sm" disabled={busy} onClick={() => void submit()}>
          {busy ? "Saving…" : "Log entry"}
        </Button>
      </div>
    </div>
  );
}
