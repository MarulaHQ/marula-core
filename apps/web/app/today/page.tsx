// /today — the daily landing work queue (TASKLIST 2.11, Wave 17 lane 1).
//
// Purpose: glanceability first — what needs attention NOW: overdue/due
// today tickets (grouped queue), the active timer focus strip (live mono
// elapsed), the time-approval queue count for approvers, and yesterday's
// missing-time strip with the quick-log dialog. Rows are dense
// Workbench-queue rows — mono ticket number, squared client tag, title,
// status, source board, right-aligned SLA countdown in tabular numerals —
// grouped Overdue / Due today / Later / No due date. Starting a timer from
// a row rides the instant-first envelope (idempotency-keyed, optimistic
// pending state, typed rejection copy).
// Non-purpose: no board management here; stop/pause/wrap-up stay on the
// docked work bar and the ticket workspace; approval DECISIONS stay on
// /approvals (this surface only counts the queue and deep-links).
"use client";

import { friendlyMutationText } from "../../lib/error-copy";
import type { components } from "@marula-core/sdk";
import { Button, Card, EmptyState, Spinner, Tag } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { BoardNav } from "../../components/board-nav";
import { OnboardingResumeBanner } from "../../components/onboarding/resume-banner";
import { DayPackPanel } from "../../components/today/day-pack-panel";
import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { dueBucket, dueLabel, fmtMins } from "../../lib/format";
import {
  suggestCategorySlug,
  useBillingAuditForTicket,
  useTimeCategories,
} from "../../lib/use-time-categories";
import { formatElapsed, sessionElapsedSecs, useTimer, type TimerSession } from "../../lib/use-timer";

type MyWorkItem = components["schemas"]["MyWorkItem"];
type GapReport = components["schemas"]["GapReportResponse"];
type Ticket = components["schemas"]["Ticket"];

const APPROVALS_QUEUE_CAP = 25;

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

function PlayIcon() {
  return (
    <svg aria-hidden="true" width="11" height="11" viewBox="0 0 12 12" fill="none">
      <path d="M3.5 2.2v7.6L9.5 6 3.5 2.2Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
    </svg>
  );
}

/** The local calendar day as YYYY-MM-DD — the day pack's anchor. */
function localDateIso(): string {
  const d = new Date();
  const month = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${month}-${day}`;
}

function yesterdayIso(): string {
  const d = new Date();
  d.setUTCDate(d.getUTCDate() - 1);
  return d.toISOString().slice(0, 10);
}

const BUCKETS = [
  { key: "overdue", label: "Overdue", anchor: "overdue" },
  { key: "today", label: "Due today", anchor: "due-today" },
  { key: "later", label: "Later", anchor: "later" },
  { key: "none", label: "No due date", anchor: "no-due-date" },
] as const;

export default function TodayPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const { active, sessions, nowMs, refresh: timerRefresh } = useTimer();
  const [items, setItems] = useState<MyWorkItem[] | null>(null);
  const [gap, setGap] = useState<GapReport | null>(null);
  const [approvalsWaiting, setApprovalsWaiting] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [logOpen, setLogOpen] = useState(false);
  const [startingId, setStartingId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const isApprover = (auth.me?.permissions ?? []).includes("special:approve_time");

  const refresh = useCallback(async () => {
    try {
      const [mine, gaps, queue] = await Promise.all([
        api.api.GET("/api/v1/tickets/my-work"),
        api.api.GET("/api/v1/time-gaps/me"),
        isApprover
          ? api.api.GET("/api/v1/time-entries/approvals/queue", {
              params: { query: { limit: APPROVALS_QUEUE_CAP } },
            })
          : Promise.resolve({ data: undefined, error: undefined }),
      ]);
      if (mine.error !== undefined) throw new Error("failed to load tickets");
      setItems(mine.data);
      if (gaps.data) setGap(gaps.data);
      if (queue.data) setApprovalsWaiting(queue.data.length);
      setError(null);
    } catch (err) {
      setError(friendlyMutationText(err, "Load failed"));
    }
  }, [isApprover]);

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

  /** Instant-first timer start from a queue row (IF1/IF2 envelope). */
  const startTimer = async (t: MyWorkItem) => {
    setActionError(null);
    setStartingId(t.id);
    const result = await runEnvelopeMutation({
      describe: `start timer on ${t.number}`,
      entityId: t.id,
      buildBody: (base) => ({ ...base, ticket_id: t.id }),
      optimistic: () => setStartingId(t.id),
      revert: () => setStartingId(null),
      request: (client, body) => client.POST("/api/v1/timers/start", { body }),
    });
    setStartingId(null);
    if (!result.ok) {
      setActionError(result.friendly ?? "Could not start the timer.");
    } else {
      void timerRefresh();
    }
  };

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  // Pre-guard the read model: `gap` can be a truthy-but-partial payload
  // (empty array fixture, in-flight read model) — never crash on it; day
  // rows themselves can be partial objects, so field reads stay
  // undefined-safe (truthiness checks only, no nested assumes).
  const gapDay = Array.isArray(gap?.days)
    ? gap.days.find((d) => d !== null && d.has_gap) ?? gap.days[0]
    : undefined;
  const day = gapDay;
  const all = Array.isArray(items) ? items : [];
  const grouped: Record<(typeof BUCKETS)[number]["key"], MyWorkItem[]> = {
    overdue: [],
    today: [],
    later: [],
    none: [],
  };
  for (const t of all) {
    grouped[dueBucket(t.sla_resolution_due ?? t.due_at)].push(t);
  }
  const pausedCount = (Array.isArray(sessions) ? sessions : []).filter(
    (s) => s !== null && s.stopped_at === null && s !== active,
  ).length;

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <div className="hidden lg:block">
          <BoardNav />
        </div>
        <main
          className="mx-auto flex w-full max-w-5xl min-w-0 flex-1 flex-col gap-4 overflow-auto p-4 lg:border-l"
          style={{ borderColor: "var(--m-border)" }}
        >
          {/* Header: what day it is + one-line attention summary */}
          <header className="m-rise flex flex-wrap items-baseline justify-between gap-2">
            <div className="grid gap-0.5">
              <p
                className="text-[10px] font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
                Work queue ·{" "}
                {new Date().toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" })}
              </p>
              <h1 className="text-lg font-semibold tracking-tight">Today</h1>
            </div>
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              {all.length} open · assigned to you · across all boards
            </span>
          </header>

          {/* Wave 26 onboarding: sticky entry affordance above the attention strip */}
          <OnboardingResumeBanner />

          {/* Wave 32 lane 8: the tech's day pack — stops, what to grab, from where */}
          <DayPackPanel
            date={localDateIso()}
            meUserId={auth.me?.user_id ?? ""}
            meName={auth.me?.display_name ?? ""}
          />

          {/* Attention strip: the day at a glance */}
          <div className="m-rise grid gap-3 sm:grid-cols-2 xl:grid-cols-4" style={{ animationDelay: "30ms" }}>
            <AttentionCard
              label="Overdue"
              value={grouped.overdue.length}
              anchor="overdue"
              tone={grouped.overdue.length > 0 ? "danger" : "clear"}
            />
            <AttentionCard
              label="Due today"
              value={grouped.today.length}
              anchor="due-today"
              tone={grouped.today.length > 0 ? "warning" : "clear"}
            />
            {isApprover && approvalsWaiting !== null && (
              <AttentionCard
                label="Approvals waiting"
                value={approvalsWaiting}
                suffix={approvalsWaiting >= APPROVALS_QUEUE_CAP ? "+" : ""}
                href="/approvals"
                tone={approvalsWaiting > 0 ? "accent" : "clear"}
              />
            )}
            {day !== undefined && day.activity && (
              <AttentionCard
                label="Time to log"
                value={day.has_gap ? day.unaccounted_minutes : 0}
                format={fmtMins}
                onClick={day.has_gap ? () => setLogOpen(true) : undefined}
                tone={day.has_gap ? "warning" : "clear"}
              />
            )}
          </div>

          {/* Focus strip: the running/paused timer set, live */}
          <FocusStrip
            active={active}
            pausedCount={pausedCount}
            elapsed={active === null ? 0 : sessionElapsedSecs(active, nowMs)}
          />

          {error !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {error}
            </p>
          )}
          {actionError !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {actionError}
            </p>
          )}

          {/* Missing-time strip: yesterday's activity-vs-entries audit */}
          {day !== undefined && day.activity && (
            <Card className="m-rise grid gap-2 p-3" style={{ animationDelay: "60ms" }} aria-label="Missing time">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="text-[13px]" style={{ color: "var(--m-text)" }}>
                  <span
                    className="text-[10px] font-semibold uppercase tracking-wide"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    Yesterday
                  </span>{" "}
                  {fmtMins(day.logged_minutes)} logged of {fmtMins(day.expected_minutes)} expected
                  {day.has_gap ? (
                    <>
                      {" — "}
                      <span style={{ color: "var(--m-status-warning-fg)" }}>
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
                className="flex h-2 gap-[2px] overflow-hidden rounded-[var(--m-radius-container)]"
                style={{ background: "var(--m-canvas)" }}
                aria-hidden="true"
              >
                {Array.from({ length: 8 }).map((_, i) => {
                  const filled = day.logged_minutes >= (day.expected_minutes / 8) * (i + 1);
                  return (
                    <span
                      key={i}
                      className="h-full flex-1 rounded-[1px]"
                      style={{
                        background: filled ? "var(--m-accent-muted, var(--m-border))" : "transparent",
                        border: "1px solid var(--m-border)",
                      }}
                    />
                  );
                })}
              </div>
            </Card>
          )}

          {/* The grouped queue */}
          {items === null ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : all.length === 0 ? (
            <EmptyState message="Nothing assigned to you yet. Tickets appear here highest-priority first." />
          ) : (
            BUCKETS.map((bucket, bi) => {
              const rows = grouped[bucket.key];
              if (rows.length === 0) return null;
              return (
                <section
                  key={bucket.key}
                  id={bucket.anchor}
                  className="m-rise grid gap-1 scroll-mt-4"
                  style={{ animationDelay: `${Math.min(90 + bi * 30, 240)}ms` }}
                  aria-label={bucket.label}
                >
                  <h2
                    className="flex items-center gap-2 text-[10px] font-semibold uppercase tracking-wide"
                    style={{ color: bucket.key === "overdue" ? "var(--m-status-danger-fg)" : "var(--m-text-muted)" }}
                  >
                    {bucket.label}
                    <span style={{ color: "var(--m-text-muted)" }}>{rows.length}</span>
                    <span
                      className="h-px flex-1"
                      style={{ background: "var(--m-border)" }}
                      aria-hidden="true"
                    />
                  </h2>
                  <ol role="list" className="flex flex-col">
                    {rows.map((t) => {
                      const dueIso = t.sla_resolution_due ?? t.due_at;
                      const due = dueIso !== null && dueIso !== undefined ? dueLabel(dueIso) : null;
                      const starting = startingId === t.id;
                      return (
                        <li
                          key={t.id}
                          className="row-hover grid h-8 min-w-0 grid-cols-[auto_auto_minmax(0,1fr)_auto_auto] items-center gap-2 pl-1 pr-1 text-xs sm:grid-cols-[auto_auto_minmax(0,1fr)_auto_auto_auto_auto]"
                        >
                          <a
                            href={`/tickets/${t.id}`}
                            className="font-mono tabular-nums"
                            style={{ color: "var(--m-text-muted)" }}
                          >
                            {t.number}
                          </a>
                          <PriorityTag priority={t.priority} />
                          <a
                            href={`/tickets/${t.id}`}
                            className="flex min-w-0 items-center gap-2 truncate"
                            style={{ color: "var(--m-text)" }}
                          >
                            <span className="truncate hover:underline">{t.title}</span>
                            {t.client_name !== null && t.client_name !== undefined && (
                              <ClientTag name={t.client_name} />
                            )}
                          </a>
                          <span
                            className="hidden truncate text-xs md:block"
                            style={{ color: "var(--m-text-muted)" }}
                          >
                            {t.board_name}
                          </span>
                          <span className="hidden h-5 items-center sm:flex">
                            {t.category === "in_progress" && <Tag tone="neutral">{t.status_name}</Tag>}
                          </span>
                          <span
                            className="text-right font-mono tabular-nums"
                            style={{
                              color:
                                due?.overdue === true
                                  ? "var(--m-status-danger-fg)"
                                  : "var(--m-text-secondary)",
                              minWidth: "5.5rem",
                            }}
                            title={
                              dueIso !== null && dueIso !== undefined
                                ? `Due ${new Date(dueIso).toLocaleString()}`
                                : undefined
                            }
                          >
                            {due === null ? "—" : due.text}
                          </span>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="!px-1.5"
                            disabled={starting}
                            aria-busy={starting}
                            aria-label={`Start timer for ${t.number}`}
                            onClick={() => void startTimer(t)}
                          >
                            <PlayIcon />
                          </Button>
                        </li>
                      );
                    })}
                  </ol>
                </section>
              );
            })
          )}
        </main>
      </div>

      {/* Quick-log dialog for closing a detected gap */}
      {logOpen && items !== null && (
        <QuickLogDialog items={Array.isArray(items) ? items : []} onClose={() => { setLogOpen(false); void refresh(); }} />
      )}
    </div>
  );
}

/** One attention-strip cell: quiet label, mono figure, deep link. */
function AttentionCard({
  label,
  value,
  suffix = "",
  format,
  anchor,
  href,
  onClick,
  tone,
}: {
  label: string;
  value: number;
  suffix?: string;
  format?: (n: number) => string;
  anchor?: string;
  href?: string;
  onClick?: () => void;
  tone: "danger" | "warning" | "accent" | "clear";
}) {
  const fg =
    tone === "danger"
      ? "var(--m-status-danger-fg)"
      : tone === "warning"
        ? "var(--m-status-warning-fg)"
        : tone === "accent"
          ? "var(--m-accent)"
          : "var(--m-text-secondary)";
  const body = (
    <>
      <span
        className="text-[10px] font-semibold uppercase tracking-wide"
        style={{ color: "var(--m-text-muted)" }}
      >
        {label}
      </span>
      <span className="font-mono text-xl font-semibold tabular-nums" style={{ color: fg }}>
        {format !== undefined ? format(value) : value}
        {suffix}
      </span>
    </>
  );
  if (href !== undefined) {
    return (
      <Card className="m-rise grid gap-0.5 p-3 transition-shadow duration-100 hover:shadow-md">
        <a href={href} className="grid gap-0.5 outline-offset-4">
          {body}
        </a>
      </Card>
    );
  }
  if (onClick !== undefined) {
    return (
      <Card className="m-rise p-3 transition-shadow duration-100 hover:shadow-md">
        <button type="button" onClick={onClick} className="grid gap-0.5 text-left outline-offset-4">
          {body}
        </button>
      </Card>
    );
  }
  return (
    <Card className="m-rise p-3" style={anchor !== undefined ? { scrollMarginTop: "1rem" } : undefined}>
      <a href={anchor !== undefined ? `#${anchor}` : undefined} className="grid gap-0.5 outline-offset-4">
        {body}
      </a>
    </Card>
  );
}

/** The timer focus strip: running elapsed in mono digits, paused count. */
function FocusStrip({
  active,
  pausedCount,
  elapsed,
}: {
  active: TimerSession | null;
  pausedCount: number;
  elapsed: number;
}) {
  const [ticket, setTicket] = useState<Ticket | null>(null);
  useEffect(() => {
    if (active === null) {
      setTicket(null);
      return;
    }
    let cancelled = false;
    void api.api
      .GET("/api/v1/tickets/{id}", { params: { path: { id: active.ticket_id } } })
      .then(({ data }) => {
        if (!cancelled) setTicket(data ?? null);
      });
    return () => {
      cancelled = true;
    };
  }, [active?.ticket_id]); // eslint-disable-line react-hooks/exhaustive-deps
  if (active === null) {
    return (
      <p
        className="m-rise text-xs"
        style={{ color: "var(--m-text-muted)", animationDelay: "60ms" }}
      >
        No active timer — start one from a row below; the work bar tracks it.
      </p>
    );
  }
  return (
    <Card className="m-rise flex items-center gap-3 p-3" style={{ animationDelay: "60ms" }} aria-label="Active timer">
      <span
        className="h-2 w-2 shrink-0 rounded-full"
        style={{ background: "var(--m-accent)" }}
        aria-hidden="true"
      />
      <div className="grid min-w-0 gap-0.5">
        <span
          className="text-[10px] font-semibold uppercase tracking-wide"
          style={{ color: "var(--m-text-muted)" }}
        >
          In focus
        </span>
        <a
          href={`/tickets/${active.ticket_id}`}
          className="flex min-w-0 items-center gap-2 truncate text-[13px] font-medium hover:underline"
          style={{ color: "var(--m-text)" }}
        >
          {ticket?.number !== undefined && (
            <span className="font-mono text-xs" style={{ color: "var(--m-text-muted)" }}>
              {ticket.number}
            </span>
          )}
          <span className="truncate">{ticket?.title ?? "Ticket in focus"}</span>
        </a>
      </div>
      <span
        className="ml-auto font-mono text-sm font-semibold tabular-nums"
        style={{ color: "var(--m-accent)" }}
      >
        {formatElapsed(elapsed)}
      </span>
      {pausedCount > 0 && (
        <Tag tone="outline">
          {pausedCount} paused
        </Tag>
      )}
    </Card>
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
        className="w-full max-w-md rounded-[var(--m-radius-container)] border p-4"
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
  const [workType, setWorkType] = useState("");
  const [billable, setBillable] = useState(true);
  const [unbillableReason, setUnbillableReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { categories, map, refresh: refreshCategories } = useTimeCategories();

  // Billing-audit chips for the picked ticket's existing entries; adopt
  // prefills the form, never auto-submits (V6 — suggestion-grade).
  const audit = useBillingAuditForTicket(ticketId);

  // Default category: ticket-context suggestion, else first category.
  useEffect(() => {
    if (categories === null || categories.length === 0) return;
    setWorkType((current) => {
      if (current !== "" && categories.some((c) => c.slug === current)) return current;
      const ticket = items.find((t) => t.id === ticketId);
      return (
        suggestCategorySlug(
          { boardName: ticket?.board_name ?? null, clientName: ticket?.client_name ?? null },
          categories,
          map,
        ) ?? categories[0]!.slug
      );
    });
  }, [categories, map, ticketId, items]);

  const selectedCategory = categories?.find((c) => c.slug === workType);

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
    if (!billable && unbillableReason === "") {
      setError("Pick a reason for the non-billable time.");
      return;
    }
    setBusy(true);
    try {
      const { error: apiError } = await api.api.POST("/api/v1/time-entries", {
        body: {
          ticket_id: ticketId,
          started_at: started.toISOString(),
          ended_at: new Date(started.getTime() + mins * 60_000).toISOString(),
          billable,
          notes: notes === "" ? null : notes,
          work_type: workType === "" ? null : workType,
          unbillable_reason: billable ? null : unbillableReason,
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
          className="rounded-[var(--m-radius-container)] px-2 py-1 text-xs"
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
            className="rounded-[var(--m-radius-container)] px-2 py-1 text-xs"
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
            className="rounded-[var(--m-radius-container)] px-2 py-1 text-xs"
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
            className="rounded-[var(--m-radius-container)] px-2 py-1 text-xs"
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
          className="rounded-[var(--m-radius-container)] px-2 py-1 text-xs"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
        />
      </div>
      <div className="flex flex-col gap-1">
        <label className="text-xs" htmlFor="log-category" style={{ color: "var(--m-text-secondary)" }}>
          Category
        </label>
        <div className="flex items-center justify-between rounded-[var(--m-radius-container)] px-2 py-1" style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}>
          <select
            id="log-category"
            value={workType}
            onChange={(e) => setWorkType(e.target.value)}
            className="w-full cursor-pointer border-0 bg-transparent text-xs outline-none"
            style={{ color: "var(--m-text)" }}
            data-testid="quicklog-category"
          >
            {(categories ?? []).length === 0 && <option value="">Loading categories…</option>}
            {(categories ?? []).map((c) => (
              <option key={c.slug} value={c.slug}>
                {c.label}
                {c.defaultBillable ? "" : " (non-billable)"}
              </option>
            ))}
          </select>
          <span aria-hidden className="text-xs" style={{ color: "var(--m-text-muted)" }}>⌄</span>
        </div>
        {audit !== null && (
          <button
            type="button"
            className="mt-1 w-fit cursor-pointer rounded-full border px-2 py-0.5 text-[10px]"
            style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)", color: "var(--m-text-secondary)" }}
            data-testid="quicklog-audit-chip"
            onClick={() => {
              if (audit === "billable_false") {
                setBillable(false);
                setUnbillableReason((r) => (r === "" ? "internal" : r));
              } else {
                setBillable(true);
                setUnbillableReason("");
              }
              void refreshCategories();
            }}
          >
            ✦ Audit: {audit === "billable_false" ? "looks non-billable — add a reason?" : "looks billable"}
          </button>
        )}
      </div>
      <div className="flex flex-col gap-1">
        <label className="text-xs" htmlFor="log-billable" style={{ color: "var(--m-text-secondary)" }}>
          Billable
        </label>
        <div className="flex items-center justify-between rounded-[var(--m-radius-container)] px-2 py-1" style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}>
          <select
            id="log-billable"
            value={billable ? "yes" : "no"}
            onChange={(e) => {
              setBillable(e.target.value === "yes");
              if (e.target.value === "yes") setUnbillableReason("");
            }}
            className="w-full cursor-pointer border-0 bg-transparent text-xs outline-none"
            style={{ color: "var(--m-text)" }}
          >
            <option value="yes">● Yes</option>
            <option value="no">○ No</option>
          </select>
          <span aria-hidden className="text-xs" style={{ color: "var(--m-text-muted)" }}>⌄</span>
        </div>
      </div>
      {!billable && (
        <div className="flex flex-col gap-1">
          <label className="text-xs" htmlFor="log-unbillable-reason" style={{ color: "var(--m-text-secondary)" }}>
            Why not billable?
          </label>
          <div className="flex items-center justify-between rounded-[var(--m-radius-container)] px-2 py-1" style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}>
            <select
              id="log-unbillable-reason"
              value={unbillableReason}
              onChange={(e) => setUnbillableReason(e.target.value)}
              className="w-full cursor-pointer border-0 bg-transparent text-xs outline-none"
              style={{ color: "var(--m-text)" }}
              data-testid="quicklog-unbillable-reason"
            >
              <option value="">Pick a reason…</option>
              {[
                ["warranty", "Covered by warranty"],
                ["courtesy", "Courtesy / goodwill"],
                ["in_retainer_block", "Inside a retainer block"],
                ["sla_covered", "Covered by the SLA"],
                ["internal", "Internal work"],
                ["mistake", "Mistake — do not bill"],
              ].map(([code, label]) => (
                <option key={code} value={code}>{label}</option>
              ))}
            </select>
            <span aria-hidden className="text-xs" style={{ color: "var(--m-text-muted)" }}>⌄</span>
          </div>
        </div>
      )}
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
