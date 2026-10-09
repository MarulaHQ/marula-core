// /today — the daily landing surface, drawn to the surface pack
// (brand/marula-mockups/marula-surface-pack/web/today/): a greeting header
// with New ticket and Quick log, the five attention cards, the two-column
// day body (Your day | Yesterday, On call, Travel), then the grouped work
// queue below. The pack draws the day; the queue keeps the work reachable
// and anchors from the attention cards. Quick-log is the pack's dialog in
// full: Day / Started at / For with pre-fill, work-type chips with a dashed
// Suggested chip that never auto-submits (V6 — suggestion-grade), the
// billable toggle card, and a footer that names what to fix.
// Non-purpose: no board management here; stop/pause/wrap-up stay on the
// docked work bar; approval DECISIONS stay on /approvals (this surface only
// counts the queue and deep-links).
"use client";

import { friendlyMutationText } from "../../lib/error-copy";
import type { components } from "@marula-core/sdk";
import { Button, Card, Dialog, DialogActions, DialogContent, EmptyState, Kbd, Spinner, Tag } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { BoardNav } from "../../components/board-nav";
import { OnboardingResumeBanner } from "../../components/onboarding/resume-banner";
import { DayPackPanel } from "../../components/today/day-pack-panel";
import { YourDayPanel } from "../../components/today/day-surface";
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
const LAST_VISIT_KEY = "today:last-visit-at";

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
  return <Icon name="play" size={11} />;
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
  const [boardHref, setBoardHref] = useState<string | null>(null);
  const [lastVisit, setLastVisit] = useState<number | null>(null);
  // The pack's clock ("now 11:41") ticks by the minute — everything that
  // renders a time-of-day keys off this one value.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(id);
  }, []);
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

  // The pack's New ticket [C] routes to the default board, where the
  // new-ticket dialog lives; nav is one cheap cached call.
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      try {
        const nav = await api.api.GET("/api/v1/boards/nav");
        if (cancelled || nav.error !== undefined) return;
        const pick = nav.data.find((e) => e.is_default) ?? nav.data[0];
        if (pick !== undefined) setBoardHref(`/boards/${pick.id}`);
      } catch {
        // The button simply stays a queue link when nav is unavailable.
        if (!cancelled) setBoardHref("/tickets");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  // "Changed since you looked": record the visit after the read so the next
  // landing counts only tickets touched since this one.
  useEffect(() => {
    if (auth.status !== "authed" || items === null) return;
    try {
      const prior = Number(window.localStorage.getItem(LAST_VISIT_KEY) ?? "0");
      setLastVisit(Number.isFinite(prior) && prior > 0 ? prior : null);
      window.localStorage.setItem(LAST_VISIT_KEY, String(Date.now()));
    } catch {
      // Private mode: the card reads "up to date" rather than guessing.
      setLastVisit(null);
    }
  }, [auth.status, items]);

  // Yesterday card's Log time lands here, same as the strip did.
  useEffect(() => {
    const open = () => setLogOpen(true);
    window.addEventListener("marula:open-quick-log", open);
    return () => window.removeEventListener("marula:open-quick-log", open);
  }, []);

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
    ? gap.days.find((d) => d !== null && d.day === new Date(now - 86_400_000).toISOString().slice(0, 10)) ?? gap.days[0]
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

  // The five attention cards (pack §2): each is a button that opens the
  // filtered queue. Zero states keep the same geometry with a quiet line.
  const atRisk = grouped.today.filter((t) => {
    const due = t.sla_resolution_due ?? t.due_at;
    return due !== null && due !== undefined && Date.parse(due) > now;
  });
  const riskMins = atRisk
    .map((t) => {
      const due = t.sla_resolution_due ?? t.due_at;
      return due !== null && due !== undefined ? Math.round((Date.parse(due) - now) / 60_000) : null;
    })
    .filter((m): m is number => m !== null)
    .sort((a, b) => a - b);
  const changed =
    lastVisit !== null ? all.filter((t) => t.updated_at !== undefined && Date.parse(t.updated_at) > lastVisit) : [];
  const firstName = (auth.me?.display_name ?? "").split(/\s+/)[0] ?? "";
  const hour = new Date(now).getHours();
  const greeting = hour < 12 ? "Good morning" : hour < 17 ? "Good afternoon" : "Good evening";
  const yesterdayLabel = new Date(now - 86_400_000).toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
  });

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
          {/* Pack header: the date over the greeting, New ticket / Quick log
              to the right with their kbd chips. */}
          <header className="m-rise flex flex-wrap items-center justify-between gap-3">
            <div className="grid gap-0.5">
              <p
                className="text-[10px] font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
                {new Date(now).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}
              </p>
              <h1 className="text-2xl font-semibold tracking-tight">
                {firstName !== "" ? `${greeting}, ${firstName}` : greeting}
              </h1>
            </div>
            <div className="flex items-center gap-2">
              {boardHref !== null && (
                <a
                  href={boardHref}
                  className="flex items-center gap-1.5 rounded-[var(--m-radius-container)] px-2.5 py-1.5 text-xs font-medium hover:underline"
                  style={{ border: "1px solid var(--m-border)", color: "var(--m-text)" }}
                >
                  <Icon name="plus" size={14} /> New ticket <Kbd tone="muted">C</Kbd>
                </a>
              )}
              <Button size="sm" onClick={() => setLogOpen(true)}>
                <Icon name="clock" size={14} /> Quick log <Kbd tone="muted">L</Kbd>
              </Button>
            </div>
          </header>

          {/* Wave 26 onboarding: sticky entry affordance above the attention strip */}
          <OnboardingResumeBanner />

          {/* The five attention cards (pack): label + figure + one line, red
              border only for genuine breaches, all green when the day is
              clear. Each is a button that opens the filtered queue. */}
          <div className="m-rise grid gap-3 sm:grid-cols-2 xl:grid-cols-5" style={{ animationDelay: "30ms" }}>
            <AttentionCard
              label="Yours today"
              dot="accent"
              value={grouped.today.length + grouped.overdue.length}
              line={
                grouped.today.length + grouped.overdue.length === 0
                  ? "Nothing waiting on you"
                  : `${grouped.today.length} due today`
              }
              anchor="overdue"
            />
            <AttentionCard
              label="Changed since you looked"
              dot={changed.length > 0 ? "accent" : "clear"}
              value={changed.length}
              line={
                lastVisit !== null
                  ? `Since you were last here at ${new Date(lastVisit).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}`
                  : "You are up to date"
              }
              anchor="due-today"
            />
            <AttentionCard
              label="At risk"
              dot={riskMins.length > 0 ? "warning" : "clear"}
              value={riskMins.length}
              line={
                riskMins.length === 0
                  ? "Nothing close to its target"
                  : riskMins.length === 1
                    ? `${windowLabel(riskMins[0]!)} left`
                    : `${windowLabel(riskMins[0]!)} and ${windowLabel(riskMins[1]!)} left`
              }
              anchor="due-today"
            />
            <AttentionCard
              label="Breached"
              dot={grouped.overdue.length > 0 ? "danger" : "clear"}
              value={grouped.overdue.length}
              line={
                grouped.overdue.length === 0
                  ? "No SLA has been missed"
                  : `${grouped.overdue[0]!.number} by ${breachText(grouped.overdue[0]!, now)}`
              }
              tone={grouped.overdue.length > 0 ? "breach" : "plain"}
              anchor="overdue"
            />
            <AttentionCard
              label="Not logged yesterday"
              dot={day === undefined || !day.has_gap ? "clear" : "warning"}
              value={day === undefined ? 0 : day.unaccounted_minutes}
              format={fmtMins}
              line={
                day === undefined
                  ? "No time expected yesterday"
                  : day.has_gap
                    ? `${yesterdayLabel}: ${fmtMins(day.logged_minutes)} logged of ${fmtMins(day.expected_minutes)}`
                    : `${yesterdayLabel} is complete. ${fmtMins(day.expected_minutes)} logged`
              }
              onClick={day !== undefined && day.has_gap ? () => setLogOpen(true) : undefined}
            />
          </div>

          {/* Wave 32 lane 8: the tech's day pack — stops, what to grab, from where */}
          <DayPackPanel
            date={localDateIso()}
            meUserId={auth.me?.user_id ?? ""}
            meName={auth.me?.display_name ?? ""}
          />

          {/* The pack's two-column day body: Your day beside Yesterday, On
              call and Travel. One dispatch call feeds the schedule and the
              rota, so they can never disagree about the same day. */}
          <YourDayPanel now={now} />

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

          {/* The grouped queue. The pack draws the day; the queue keeps the
              work reachable and the attention cards anchor into it. The
              approvals count for approvers rides the same quiet band. */}
          {items === null ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : all.length === 0 ? (
            <EmptyState message="Nothing assigned to you yet. Tickets appear here highest-priority first." />
          ) : (
            <>
              {isApprover && approvalsWaiting !== null && approvalsWaiting > 0 && (
                <a
                  href="/approvals"
                  className="m-rise flex items-center gap-2 rounded-[var(--m-radius-container)] px-3 py-2 text-xs hover:underline"
                  style={{ background: "var(--m-surface-2)", color: "var(--m-text)" }}
                >
                  <span
                    className="h-1.5 w-1.5 rounded-full"
                    style={{ background: "var(--m-accent)" }}
                    aria-hidden="true"
                  />
                  <span className="font-medium">
                    {approvalsWaiting === APPROVALS_QUEUE_CAP ? `${approvalsWaiting}+` : approvalsWaiting} time{" "}
                    {approvalsWaiting === 1 ? "entry" : "entries"} waiting for your approval
                  </span>
                  <Icon name="arrow-r" size={13} />
                </a>
              )}
              {BUCKETS.map((bucket, bi) => {
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
                          className="row-hover grid h-[var(--m-density-row)] min-w-0 grid-cols-[auto_auto_minmax(0,1fr)_auto_auto] items-center gap-2 pl-1 pr-1 text-xs sm:grid-cols-[auto_auto_minmax(0,1fr)_auto_auto_auto_auto]"
                        >
                          <a
                            href={`/tickets/${t.id}`}
 className="tabular-nums"
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
 className="text-right tabular-nums"
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
              })}
            </>
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

/** One attention card (pack §2): a coloured dot with the label, the tabular
 * figure, one honest line underneath. Breach gets the red border; clear
 * cards keep the geometry with a muted line. */
function AttentionCard({
  label,
  value,
  dot,
  format,
  line,
  anchor,
  href,
  onClick,
  tone = "plain",
}: {
  label: string;
  value: number;
  dot: "accent" | "warning" | "danger" | "clear";
  format?: (n: number) => string;
  line: string;
  anchor?: string;
  href?: string;
  onClick?: () => void;
  tone?: "plain" | "breach";
}) {
  const dotColor =
    dot === "danger"
      ? "var(--m-status-danger-fg)"
      : dot === "warning"
        ? "var(--m-status-warning-fg)"
        : dot === "accent"
          ? "var(--m-accent)"
          : "var(--m-text-muted)";
  const breached = tone === "breach";
  const body = (
    <>
      <span className="flex items-center gap-1.5">
        <span
          className="h-1.5 w-1.5 shrink-0 rounded-full"
          style={{ background: dotColor }}
          aria-hidden="true"
        />
        <span
          className="text-[10px] font-semibold uppercase tracking-wide"
          style={{ color: breached ? "var(--m-status-danger-fg)" : "var(--m-text-muted)" }}
        >
          {label}
        </span>
      </span>
      <span
        className="text-xl font-semibold tabular-nums"
        style={{ color: breached ? "var(--m-status-danger-fg)" : "var(--m-text)" }}
      >
        {format !== undefined ? format(value) : value}
      </span>
      <span className="truncate text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        {line}
      </span>
    </>
  );
  const cardStyle: React.CSSProperties = breached
    ? { borderColor: "var(--m-status-danger-fg)" }
    : {};
  if (href !== undefined) {
    return (
      <Card
        className="m-rise grid gap-0.5 p-3 transition-shadow duration-100 hover:shadow-md"
        style={cardStyle}
      >
        <a href={href} className="grid gap-0.5 outline-offset-4">
          {body}
        </a>
      </Card>
    );
  }
  if (onClick !== undefined) {
    return (
      <Card className="m-rise p-3 transition-shadow duration-100 hover:shadow-md" style={cardStyle}>
        <button type="button" onClick={onClick} className="grid gap-0.5 text-left outline-offset-4">
          {body}
        </button>
      </Card>
    );
  }
  return (
    <Card
      className="m-rise p-3"
      style={{ ...cardStyle, ...(anchor !== undefined ? { scrollMarginTop: "1rem" } : {}) }}
    >
      <a href={anchor !== undefined ? `#${anchor}` : undefined} className="grid gap-0.5 outline-offset-4">
        {body}
      </a>
    </Card>
  );
}

/** "09:00" — a stored instant in the clock the user works in, never a raw
 * UTC string: the form frames entries in local time and so must the copy. */
function localClock(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "--:--";
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** "1:00" — the entry length as the pack's footer button writes it. */
function durationClock(mins: number): string {
  if (!Number.isFinite(mins) || mins <= 0) return "0:00";
  return `${Math.floor(mins / 60)}:${String(Math.round(mins % 60)).padStart(2, "0")}`;
}

/** "709m" or "11h 49m" — one at-risk window label: sub-hour windows read
 * as minutes, longer ones as hours and minutes. */
function windowLabel(mins: number): string {
  if (!Number.isFinite(mins) || mins <= 0) return "0m";
  if (mins < 60) return `${mins}m`;
  return fmtMins(mins);
}

/** "Mon 28 Sep (yesterday)" — the quick-log dialog's fixed day. */
function yesterdayLabel(): string {
  const y = new Date(Date.now() - 86_400_000);
  return `${y.toLocaleDateString(undefined, { weekday: "short" })} ${y.getDate()} ${y.toLocaleString(undefined, { month: "short" })} (yesterday)`;
}

/** The pack's kbd hint chips (C / L) ride the header buttons as kit `Kbd`. */

/** "by 1h 05m" — how far past its target the worst breached ticket is. */
function breachText(t: MyWorkItem, now: number): string {
  const due = t.sla_resolution_due ?? t.due_at;
  if (due === null || due === undefined) return "a while";
  const mins = Math.max(1, Math.round((now - Date.parse(due)) / 60_000));
  if (mins < 60) return `by ${mins}m`;
  return `by ${Math.floor(mins / 60)}h ${String(mins % 60).padStart(2, "0")}m`;
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
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              {ticket.number}
            </span>
          )}
          <span className="truncate">{ticket?.title ?? "Ticket in focus"}</span>
        </a>
      </div>
      <span
 className="ml-auto text-sm font-semibold tabular-nums"
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

/// The kit's dialog shell for the quick-log form; the pack's anatomy (title,
/// subtitle, footer with fix count) rides DialogContent's header/description
/// and DialogActions.
function QuickLogDialog({
  items,
  onClose,
}: {
  items: MyWorkItem[];
  onClose: () => void;
}) {
  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent title="Quick log" description="Time you forgot to log. Nothing is submitted until you choose a work type.">
        <QuickLogForm items={items} onDone={onClose} />
      </DialogContent>
    </Dialog>
  );
}

/// The pack's quick-log dialog. Time you forgot to log: pre-filled for the
/// common case (yesterday 09:00 for an hour), a work-type chip row whose
/// suggested chip is advice and never auto-submits (V6), billable as a
/// toggle card, errors as blush lines that name the fix, and a footer that
/// counts the things to fix.
function QuickLogForm({
  items,
  onDone,
}: {
  items: MyWorkItem[];
  onDone: () => void;
}) {
  const [ticketId, setTicketId] = useState(items[0]?.id ?? "");
  const [forMinutes, setForMinutes] = useState("60");
  const [startTime, setStartTime] = useState("09:00");
  const [notes, setNotes] = useState("");
  const [workType, setWorkType] = useState("");
  const [workTypeChosen, setWorkTypeChosen] = useState(false);
  const [billable, setBillable] = useState(true);
  const [unbillableReason, setUnbillableReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { categories, map, refresh: refreshCategories } = useTimeCategories();

  // Yesterday's entry windows, for the pack's overlap line: "That overlaps
  // #12345 from 08:00 to 09:30. Yesterday's gap is 13:00 to 14:15."
  const [dayEntries, setDayEntries] = useState<components["schemas"]["TimeEntry"][]>([]);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/time-entries", {
          params: { query: { since: `${yesterdayIso()}T00:00:00Z`, limit: 100 } },
        });
        if (!cancelled) {
          setDayEntries(
            (res.data ?? []).filter((e) => e.started_at.slice(0, 10) === yesterdayIso()),
          );
        }
      } catch {
        // No overlap check without the windows; the server still validates.
        if (!cancelled) setDayEntries([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Billing-audit chips for the picked ticket's existing entries; adopt
  // prefills the form, never auto-submits (V6 — suggestion-grade).
  const audit = useBillingAuditForTicket(ticketId);

  // The suggested work type is derived, never selected: the dashed chip
  // offers it, the helper names its sources, the primary button stays
  // disabled until a person chooses (pack: "Choose a work type").
  const suggested =
    categories === null || categories.length === 0
      ? null
      : suggestCategorySlug(
          {
            boardName: items.find((t) => t.id === ticketId)?.board_name ?? null,
            clientName: items.find((t) => t.id === ticketId)?.client_name ?? null,
          },
          categories,
          map,
        ) ?? categories[0]!.slug;

  const mins = Number.parseInt(forMinutes, 10);
  const validMins = Number.isFinite(mins) && mins > 0;
  const startedOk = !Number.isNaN(new Date(`${yesterdayIso()}T${startTime}:00`).getTime());
  const overlap = useMemo(() => {
    if (!validMins || !startedOk) return null;
    const started = new Date(`${yesterdayIso()}T${startTime}:00`);
    const ended = new Date(started.getTime() + mins * 60_000);
    const hit = dayEntries.find((e) => {
      const s = new Date(e.started_at).getTime();
      const en = new Date(e.ended_at).getTime();
      return s < ended.getTime() && en > started.getTime();
    });
    if (hit === undefined) return null;
    return { hit, started, ended };
  }, [dayEntries, validMins, startedOk, mins, startTime]);
  const fixes: string[] = [];
  if (!workTypeChosen) fixes.push("choose a work type");
  if (!validMins) fixes.push("set how long the work took");
  if (!startedOk) fixes.push("set when it started");
  if (!billable && unbillableReason === "") fixes.push("pick why the time is not billable");
  // The pack's error state keeps the primary reading "Log 1:00" with the
  // warning beside it — only an unfillable form blocks the submit; the
  // overlap names its fix in the footer instead.
  const canSubmit = fixes.length === 0 && workTypeChosen && ticketId !== "";

  const fixCount = fixes.length + (overlap !== null ? 1 : 0);
  const dayLabel = yesterdayLabel();

  async function submit() {
    if (ticketId === "" || !validMins || !startedOk) {
      setError("Pick a ticket, a start time and a positive length.");
      return;
    }
    if (!billable && unbillableReason === "") {
      setError("Pick a reason for the non-billable time.");
      return;
    }
    setBusy(true);
    try {
      const day = yesterdayIso();
      const started = new Date(`${day}T${startTime}:00`);
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
          className="rounded-[var(--m-radius-container)] px-2 py-1.5 text-xs"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
          data-testid="quicklog-ticket"
        >
          {items.map((t) => (
            <option key={t.id} value={t.id}>
              {t.number} — {t.title}
              {t.client_name !== null && t.client_name !== undefined ? `, ${t.client_name}` : ""}
            </option>
          ))}
        </select>
      </div>

      <div className="grid grid-cols-3 gap-2">
        <div className="flex flex-col gap-1">
          <label className="text-xs" htmlFor="log-date" style={{ color: "var(--m-text-secondary)" }}>
            Day
          </label>
          <input
            id="log-date"
            readOnly
            value={dayLabel}
            className="rounded-[var(--m-radius-container)] px-2 py-1.5 text-xs"
            style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
          />
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs" htmlFor="log-start" style={{ color: "var(--m-text-secondary)" }}>
            Started at
          </label>
          <input
            id="log-start"
            type="time"
            value={startTime}
            onChange={(e) => setStartTime(e.target.value)}
            className="rounded-[var(--m-radius-container)] px-2 py-1.5 text-xs"
            style={{
              border: `1px solid ${startedOk ? "var(--m-border)" : "var(--m-status-warning-fg)"}`,
              background: "var(--m-surface)",
              color: "var(--m-text)",
            }}
            data-testid="quicklog-start"
          />
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs" htmlFor="log-minutes" style={{ color: "var(--m-text-secondary)" }}>
            For
          </label>
          <input
            id="log-minutes"
            type="number"
            min={1}
            value={forMinutes}
            onChange={(e) => setForMinutes(e.target.value)}
            className="rounded-[var(--m-radius-container)] px-2 py-1.5 text-xs"
            style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
            data-testid="quicklog-minutes"
          />
        </div>
      </div>

      {overlap !== null && (
        <p
          className="flex items-start gap-1.5 rounded-[var(--m-radius-container)] px-2 py-1.5 text-[11px]"
          style={{
            border: "1px solid var(--m-status-warning-fg)",
            background: "var(--m-status-warning-bg)",
            color: "var(--m-status-warning-fg)",
          }}
          role="alert"
          data-testid="quicklog-overlap"
        >
          <Icon name="clock" size={12} />
          <span>
            That overlaps {overlapTicketLabel(overlap.hit, items)} from{" "}
            {localClock(overlap.hit.started_at)} to {localClock(overlap.hit.ended_at)}.{" "}
            {gapHint(dayEntries, overlap.started, overlap.ended)}
          </span>
        </p>
      )}

      <p
        className="flex items-start gap-1.5 rounded-[var(--m-radius-container)] px-2 py-1.5 text-[11px]"
        style={{ background: "var(--m-bg-subtle)", color: "var(--m-text-secondary)" }}
      >
        <Icon name="clock" size={12} />
        <span>
          Pre-filled for the common case: yesterday at 09:00 for an hour. Change whatever is not right.
        </span>
      </p>

      <div className="flex flex-col gap-1.5">
        <span className="text-xs font-medium" style={{ color: "var(--m-text-secondary)" }}>
          Work type
        </span>
        <div className="flex flex-wrap gap-1.5" data-testid="quicklog-work-types">
          {categories === null ? (
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              Loading categories…
            </span>
          ) : (
            categories.map((c) => {
              const isSuggested = c.slug === suggested;
              const selected = workType === c.slug;
              return (
                <button
                  key={c.slug}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => {
                    setWorkType(c.slug);
                    setWorkTypeChosen(true);
                    if (!c.defaultBillable) setBillable(false);
                    void refreshCategories();
                  }}
                  className="rounded-full px-2.5 py-1 text-[11px] font-medium"
                  style={{
                    border: `1px ${isSuggested && !selected ? "dashed" : "solid"} var(--m-border)`,
                    background: selected ? "var(--m-accent)" : "var(--m-surface)",
                    color: selected ? "var(--m-text)" : "var(--m-text-secondary)",
                  }}
                >
                  {c.label}
                  {isSuggested && !selected && (
                    <span className="ml-1 text-[9px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                      Suggested
                    </span>
                  )}
                  {selected && <Icon name="check" size={11} />}
                </button>
              );
            })
          )}
        </div>
        <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          {workTypeChosen
            ? `Chosen by you${audit !== null ? "" : "."}`
            : suggested !== null
              ? "Suggested from the ticket's board and client. Choose it, or pick another."
              : "Choose the work type that fits."}
        </p>
      </div>

      <div
        className="flex items-center justify-between rounded-[var(--m-radius-container)] px-2.5 py-2"
        style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}
      >
        <div className="grid gap-0">
          <span className="text-xs font-medium" style={{ color: "var(--m-text)" }}>
            Billable
          </span>
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            This time can go on an invoice.
          </span>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={billable}
          aria-label="Billable"
          onClick={() => {
            setBillable((b) => !b);
            if (!billable) setUnbillableReason("");
          }}
          className="relative h-5 w-9 rounded-full transition-colors duration-100"
          style={{ background: billable ? "var(--m-accent)" : "var(--m-border)" }}
          data-testid="quicklog-billable"
        >
          <span
            className="absolute top-0.5 h-4 w-4 rounded-full transition-transform duration-100"
            style={{
              background: "var(--m-surface)",
              border: "1px solid var(--m-border)",
              transform: `translateX(${billable ? 18 : 2}px)`,
            }}
          />
        </button>
      </div>

      {!billable && (
        <div className="flex flex-col gap-1">
          <label className="text-xs" htmlFor="log-unbillable-reason" style={{ color: "var(--m-text-secondary)" }}>
            Why not billable?
          </label>
          <div
            className="flex items-center justify-between rounded-[var(--m-radius-container)] px-2 py-1.5"
            style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)" }}
          >
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

      <div className="flex flex-col gap-1">
        <label className="text-xs" htmlFor="log-notes" style={{ color: "var(--m-text-secondary)" }}>
          What did you do? <span style={{ color: "var(--m-text-muted)" }}>Optional</span>
        </label>
        <textarea
          id="log-notes"
          value={notes}
          rows={2}
          placeholder="For example, fixed the Finance shared mailbox permissions"
          onChange={(e) => setNotes(e.target.value)}
          className="rounded-[var(--m-radius-container)] px-2 py-1.5 text-xs"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-surface)", color: "var(--m-text)" }}
        />
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

      {error !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
          {error}
        </p>
      )}

      <div
        className="flex items-center justify-between gap-2 border-t pt-3"
        style={{ borderColor: "var(--m-border)" }}
      >
        <span className="text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
          {fixCount > 0 ? (
            <span style={{ color: "var(--m-status-warning-fg)" }}>
              {fixCount === 1 ? "1 thing to fix" : `${fixCount} things to fix`}
              {overlap !== null ? " — shorten the entry or move it into the gap" : ""}
            </span>
          ) : (
            `${dayLabel.replace(" (yesterday)", "")}, ${startTime} to ${endTimeFrom(startTime, mins)}`
          )}
        </span>
        <div className="flex gap-2">
          <Button size="sm" variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button
            size="sm"
            disabled={busy || !canSubmit}
            onClick={() => void submit()}
            data-testid="quicklog-submit"
          >
            {busy
              ? "Saving…"
              : workTypeChosen && canSubmit
                ? `Log ${validMins ? durationClock(mins) : ""}`
                : "Choose a work type"}
          </Button>
        </div>
      </div>
    </div>
  );
}

/** "10:00" — the entry's end clock line for the dialog footer. */
function endTimeFrom(start: string, minutes: number): string {
  const started = new Date(`${yesterdayIso()}T${start}:00`);
  if (Number.isNaN(started.getTime()) || !Number.isFinite(minutes)) return "";
  return new Date(started.getTime() + minutes * 60_000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** "MAR-100" — the overlapping entry's ticket, matched against the queue
 * items so the line names the ticket like the pack does. */
function overlapTicketLabel(
  hit: components["schemas"]["TimeEntry"],
  items: MyWorkItem[],
): string {
  const byId = items.find((t) => t.id === hit.ticket_id);
  if (byId !== undefined) return byId.number;
  return "an entry";
}

/** "Yesterday's gap is 13:00 to 14:15." — the nearest free window on the
 * day's timeline, so the fix is one glance away. */
function gapHint(
  entries: components["schemas"]["TimeEntry"][],
  overlapStart: Date,
  overlapEnd: Date,
): string {
  const windows: { from: number; to: number }[] = [];
  const dayStart = new Date(`${yesterdayIso()}T08:00:00`).getTime();
  const dayEnd = new Date(`${yesterdayIso()}T17:00:00`).getTime();
  let cursor = dayStart;
  const sorted = [...entries].sort((a, b) => a.started_at.localeCompare(b.started_at));
  for (const e of sorted) {
    const s = new Date(e.started_at).getTime();
    const en = new Date(e.ended_at).getTime();
    if (Number.isNaN(s) || Number.isNaN(en)) continue;
    if (en <= dayStart || s >= dayEnd) continue;
    if (s - cursor >= 30 * 60_000) windows.push({ from: cursor, to: Math.min(s, dayEnd) });
    cursor = Math.max(cursor, en);
  }
  if (dayEnd - cursor >= 30 * 60_000) windows.push({ from: cursor, to: dayEnd });
  const open = windows.find((w) => w.to - w.from >= overlapEnd.getTime() - overlapStart.getTime());
  if (open === undefined) return "No free hour left on yesterday's timeline — shorten the entry or let it overlap.";
  const fmt = (ms: number) =>
    new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
  return `Yesterday's gap is ${fmt(open.from)} to ${fmt(open.to)}.`;
}
