// /today — the my-work surface.
//
// Purpose: priority-ordered list of the caller's open tickets with inline
// timer start (switch semantics: starting one auto-pauses the previous),
// plus the missing-time strip per design/mockups/today-*: yesterday's
// logged-vs-expected summary with a "Log time" affordance that opens a
// quick manual-entry dialog (also the landing target for missing_time
// nudges via /today?log=1).
// Non-purpose: no board management or admin surfaces here.
"use client";

import type { components } from "@meridian/sdk";
import {
  Button,
  Dialog,
  DialogContent,
  DialogTrigger,
  EmptyState,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  StatusTag,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { AppHeader } from "../../components/app-header";
import { api, useRequireAuth } from "../../lib/api";
import { sessionElapsedSecs, useTimer } from "../../lib/use-timer";

type Ticket = components["schemas"]["Ticket"];
type GapReport = components["schemas"]["GapReportResponse"];

const PRIORITY_ORDER: Record<string, number> = { p1: 0, p2: 1, p3: 2, p4: 3 };

function PriorityTag({ priority }: { priority: string }) {
  const tone =
    priority === "p1" ? "danger" : priority === "p2" ? "warning" : "neutral";
  return <StatusTag tone={tone}>{priority.toUpperCase()}</StatusTag>;
}

function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
}

function fmtMins(mins: number): string {
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/// Yesterday's local date as YYYY-MM-DD (the report's own default, made
/// explicit here for the strip label).
function yesterdayIso(): string {
  const d = new Date(Date.now() - 86_400_000);
  return d.toISOString().slice(0, 10);
}

export default function TodayPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const timer = useTimer();
  const [tickets, setTickets] = useState<Ticket[] | null>(null);
  const [gap, setGap] = useState<GapReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [logOpen, setLogOpen] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [mine, gaps] = await Promise.all([
        api.api.GET("/api/tickets/my-work"),
        api.api.GET("/api/time-gaps/me"),
      ]);
      if (mine.error !== undefined) throw new Error("failed to load tickets");
      setTickets(
        [...(mine.data ?? [])].sort(
          (a, b) =>
            (PRIORITY_ORDER[a.priority] ?? 9) - (PRIORITY_ORDER[b.priority] ?? 9),
        ),
      );
      if (gaps.data) setGap(gaps.data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Load failed");
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // missing_time nudges land here: /today?log=1 opens the quick-log dialog.
  useEffect(() => {
    if (auth.status === "authed" && new URLSearchParams(window.location.search).get("log") === "1") {
      setLogOpen(true);
    }
  }, [auth.status]);

  async function startTimer(ticketId: string) {
    await api.api.POST("/api/timers/start", { body: { ticket_id: ticketId } });
    void timer.refresh();
  }

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-screen items-center justify-center">
        <Spinner />
      </main>
    );
  }

  const day = gap?.days[0];
  const running = timer.active;

  return (
    <div className="flex h-screen flex-col">
      <AppHeader />
      <main
        className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4"
        style={{ paddingBottom: running !== null ? 96 : undefined }}
      >
        <div className="flex items-center justify-between">
          <h1 className="text-sm font-semibold">Today</h1>
          <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            {tickets?.length ?? 0} open · assigned to you
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
                {fmtMins(day.logged_minutes)} logged of {fmtMins(day.expected_minutes)} expected
                {day.has_gap ? (
                  <>
                    {" — "}
                    <span style={{ color: "var(--m-status-warning-fg, var(--m-text))" }}>
                      {fmtMins(day.unaccounted_minutes)} unaccounted
                    </span>
                  </>
                ) : null}
              </p>
              <Dialog open={logOpen} onOpenChange={setLogOpen}>
                <DialogTrigger asChild>
                  <Button size="sm">Log time</Button>
                </DialogTrigger>
                <DialogContent title="Log missing time">
                  <QuickLogForm
                    tickets={tickets ?? []}
                    onDone={() => {
                      setLogOpen(false);
                      void refresh();
                    }}
                  />
                </DialogContent>
              </Dialog>
            </div>
            <div
              className="mt-2 flex h-3 gap-[2px] overflow-hidden rounded-[2px]"
              style={{ background: "var(--m-canvas)" }}
              aria-hidden="true"
            >
              {Array.from({ length: 8 }).map((_, i) => {
                const filled = day.logged_minutes >= (day.expected_minutes / 8) * (i + 1);
                return (
                  <span
                    key={i}
                    className="h-full flex-1"
                    style={{
                      background: filled ? "var(--m-accent-muted, var(--m-border))" : "transparent",
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

        {tickets === null ? (
          <div className="flex flex-1 items-center justify-center">
            <Spinner />
          </div>
        ) : tickets.length === 0 ? (
          <EmptyState message="Nothing assigned to you yet. Tickets appear here highest-priority first." />
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Number</Th>
                <Th>Priority</Th>
                <Th>Title</Th>
                <Th>Status</Th>
                <Th>Due</Th>
                <Th>Timer</Th>
              </tr>
            </thead>
            <tbody>
              {tickets.map((t) => {
                const session = timer.sessions.find(
                  (s) => s.ticket_id === t.id && s.stopped_at === null,
                );
                return (
                  <Tr key={t.id} onClick={() => router.push(`/tickets/${t.id}`)}>
                    <Td>{t.number}</Td>
                    <Td><PriorityTag priority={t.priority} /></Td>
                    <Td>{t.title}</Td>
                    <Td><StatusTag tone={categoryTone(t.category)}>{t.category}</StatusTag></Td>
                    <Td>{t.due_at !== null && t.due_at !== undefined ? new Date(t.due_at).toLocaleDateString() : "—"}</Td>
                    <Td>
                      {session !== undefined ? (
                        <span
                          className="font-mono text-xs tabular-nums"
                          style={{ color: "var(--m-accent)" }}
                        >
                          {session.stopped_at === null &&
                          session.segments?.slice(-1)[0]?.ended_at === null
                            ? "▶ "
                            : "⏸ "}
                          {Math.floor(sessionElapsedSecs(session, timer.nowMs) / 60)}m
                        </span>
                      ) : (
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={(e) => {
                            e.stopPropagation();
                            void startTimer(t.id);
                          }}
                        >
                          Start
                        </Button>
                      )}
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </main>
    </div>
  );
}

/// Quick manual entry for closing a missing-time gap: defaults to
/// yesterday 09:00 + 60 minutes on the first open ticket.
function QuickLogForm({
  tickets,
  onDone,
}: {
  tickets: Ticket[];
  onDone: () => void;
}) {
  const [ticketId, setTicketId] = useState(tickets[0]?.id ?? "");
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
      const { error: apiError } = await api.api.POST("/api/time-entries", {
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
      setError(err instanceof Error ? err.message : "Save failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1">
        <Label htmlFor="log-ticket">Ticket</Label>
        <Select value={ticketId} onValueChange={setTicketId}>
          <SelectTrigger id="log-ticket" className="w-full">
            <SelectValue placeholder="Choose a ticket" />
          </SelectTrigger>
          <SelectContent>
            {tickets.map((t) => (
              <SelectItem key={t.id} value={t.id}>
                {t.number} — {t.title}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="flex gap-3">
        <div className="flex flex-1 flex-col gap-1">
          <Label htmlFor="log-date">Date</Label>
          <Input id="log-date" value={yesterdayIso()} readOnly />
        </div>
        <div className="flex flex-1 flex-col gap-1">
          <Label htmlFor="log-start">Start</Label>
          <Input
            id="log-start"
            type="time"
            value={startTime}
            onChange={(e) => setStartTime(e.target.value)}
          />
        </div>
        <div className="flex flex-1 flex-col gap-1">
          <Label htmlFor="log-minutes">Minutes</Label>
          <Input
            id="log-minutes"
            type="number"
            min={1}
            value={minutes}
            onChange={(e) => setMinutes(e.target.value)}
          />
        </div>
      </div>
      <div className="flex flex-col gap-1">
        <Label htmlFor="log-notes">Notes</Label>
        <Input
          id="log-notes"
          value={notes}
          placeholder="What did you work on?"
          onChange={(e) => setNotes(e.target.value)}
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
