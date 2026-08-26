// /dispatch — the dispatch board (TASKLIST 2.19).
//
// Purpose: the dispatcher's capacity view per design/mockups/dispatch-
// board-light.png (+ dark parity via tokens): technician columns with
// capacity bars (Ember accent reserved for over-capacity warnings and
// the ghost preview), appointment blocks draggable between technicians
// and time slots with a dashed ghost placement preview, inline conflict
// badges backed by typed server rejections, an unassigned queue rail
// feeding drag sources with SLA countdowns, presence dots from the
// realtime hub, live reconciliation of appointment events, and full
// keyboard-reachable moves announced via aria-live.
//
// Non-purpose: no auto-assign heuristics, no availability editing (the
// calendar surface owns that), no travel RoutingProvider math — travel
// renders as hatched placeholders derived from `travel_minutes`. The
// hour axis is UTC because the API's dispatch day window is UTC; the
// status bar says so.

"use client";

import { Button } from "@meridian/ui";
import type { components } from "@meridian/sdk";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
} from "react";
import { AppHeader } from "../../components/app-header";
import type { Block, QueueItem, Tech } from "../../lib/dispatch-reducer";
import { useDispatchBoard } from "../../lib/use-dispatch";
import { useRequireAuth } from "../../lib/api";

type OnCall = components["schemas"]["OnCallBlock"];

const DAY_START_HOUR = 6;
const DAY_END_HOUR = 20;
const ROW_PX = 44;
const SNAP_MIN = 15;

function isoDay(offsetDays: number): string {
  const d = new Date();
  d.setUTCDate(d.getUTCDate() + offsetDays);
  return d.toISOString().slice(0, 10);
}

function isoOffset(date: string, days: number): string {
  const d = new Date(`${date}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

function addMinutesIso(iso: string, minutes: number): string {
  return new Date(Date.parse(iso) + minutes * 60_000).toISOString();
}

function minutesSinceDayStart(iso: string): number {
  const d = new Date(iso);
  return d.getUTCHours() * 60 + d.getUTCMinutes() - DAY_START_HOUR * 60;
}

function spanMinutes(b: Block): number {
  return Math.max(
    SNAP_MIN,
    Math.round((Date.parse(b.ends_at) - Date.parse(b.starts_at)) / 60000),
  );
}

function utcTimeLabel(iso: string): string {
  return new Date(iso).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "UTC",
    hour12: false,
  });
}

function slaCountdown(
  item: QueueItem,
): { label: string; warn: boolean } | null {
  const target = item.sla_resolution_due ?? item.due_at;
  if (!target) return null;
  const remaining = Date.parse(target) - Date.now();
  if (remaining <= 0) return { label: "SLA breach", warn: true };
  const h = Math.floor(remaining / 3_600_000);
  const m = Math.floor((remaining % 3_600_000) / 60_000);
  const label =
    h > 0 ? `${h}h ${m}m left` : `${m}:${String(Math.floor((remaining % 60_000) / 1000)).padStart(2, "0")} left`;
  return {
    label: `${item.sla_resolution_due ? "SLA" : "Due"} ${label}`,
    warn: remaining < 30 * 60_000,
  };
}

const PRIORITY_FILTERS = ["all", "p1", "p2", "p3", "p4"] as const;

export default function DispatchPage() {
  const auth = useRequireAuth();
  const board = useDispatchBoard(isoDay(0));
  const { state, dispatch, dropMove, dropAssign, rawView } = board;
  const [priorityFilter, setPriorityFilter] =
    useState<(typeof PRIORITY_FILTERS)[number]>("all");
  const [liveMessage, setLiveMessage] = useState("");

  useEffect(() => {
    if (state.announcement) setLiveMessage(state.announcement);
  }, [state.announcement]);

  // --- conflict banner count --------------------------------------------
  const conflictCount = useMemo(() => {
    let n = 0;
    for (const tech of state.technicians) {
      for (const a of tech.appointments) {
        if (conflictOf(tech, a)) n += 1;
      }
    }
    return n;
  }, [state.technicians]);

  const filteredQueue = useMemo(
    () =>
      state.unassigned.filter(
        (t) =>
          priorityFilter === "all" || t.priority === priorityFilter,
      ),
    [state.unassigned, priorityFilter],
  );

  const presentSet = useMemo(() => new Set(state.presence), [state.presence]);

  const overCapacity = state.technicians.filter(
    (t) => t.utilization_percent > 100,
  ).length;

  // ---- shared landing path for mouse AND keyboard drops -----------------
  const landRef = useRef<(userId: string, startsAt: string, endsAt: string) => void>(
    () => undefined,
  );

  // Keep latest interactive context for the keyboard engine without
  // resubscribing its window listener every render.
  const ctxRef = useRef<{
    techs: Tech[];
    drag: typeof state.drag;
    queue: QueueItem[];
  }>({ techs: [], drag: null, queue: [] });
  ctxRef.current = { techs: state.technicians, drag: state.drag, queue: state.unassigned };
  const ghostRef = useRef(state.ghost);
  ghostRef.current = state.ghost;

  useEffect(() => {
    if (auth.status !== "authed" || !state.drag) return;
    function seedGhost(): void {
      const { drag, techs } = ctxRef.current;
      if (!drag || techs.length === 0) return;
      if (drag.source === "board") {
        const all = techs.flatMap((t) => t.appointments);
        const b = all.find((a) => a.id === drag.id);
        if (b) {
          dispatch({
            type: "ghost_move",
            userId: b.user_id,
            startIso: b.starts_at,
            endIso: b.ends_at,
          });
          return;
        }
      }
      const firstTech = techs[0];
      if (!firstTech) return;
      const start = new Date(Date.now());
      start.setUTCMinutes(Math.floor(start.getUTCMinutes() / 30) * 30, 0, 0);
      dispatch({
        type: "ghost_move",
        userId: firstTech.user_id,
        startIso: start.toISOString(),
        endIso: addMinutesIso(start.toISOString(), 60),
      });
    }

    function onKeyDown(ev: WindowEventMap["keydown"]): void {
      const { drag, techs } = ctxRef.current;
      if (!drag) return;
      const g = ghostRef.current;
      if (ev.key === "Escape") {
        ev.preventDefault();
        dispatch({ type: "drag_end" });
        return;
      }
      if (ev.key === "Enter") {
        ev.preventDefault();
        if (g && !g.conflict) {
          landRef.current(g.userId, g.startIso, g.endIso);
        }
        return;
      }
      if (!ev.key.startsWith("Arrow")) return;
      ev.preventDefault();
      if (!g) {
        seedGhost();
        return;
      }
      const idx = techs.findIndex((t) => t.user_id === g.userId);
      let userId = g.userId;
      let startMs = Date.parse(g.startIso);
      const durationMin = Math.round(
        (Date.parse(g.endIso) - startMs) / 60000,
      );
      if (ev.key === "ArrowRight" && idx < techs.length - 1) {
        userId = techs[idx + 1]?.user_id ?? userId;
      } else if (ev.key === "ArrowLeft" && idx > 0) {
        userId = techs[idx - 1]?.user_id ?? userId;
      } else if (ev.key === "ArrowDown") {
        startMs += 30 * 60_000;
      } else if (ev.key === "ArrowUp") {
        startMs -= 30 * 60_000;
      }
      dispatch({
        type: "ghost_move",
        userId,
        startIso: new Date(startMs).toISOString(),
        endIso: addMinutesIso(new Date(startMs).toISOString(), durationMin),
      });
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [auth.status, state.drag]);

  if (auth.status !== "authed") {
    return (
      <div className="flex min-h-screen flex-col">
        <AppHeader />
        <p className="p-6 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          Signing in…
        </p>
      </div>
    );
  }

  const viewDate = board.date;
  const dayBaseMs = Date.parse(`${viewDate}T00:00:00Z`);
  const hoursAxis = Array.from(
    { length: DAY_END_HOUR - DAY_START_HOUR },
    (_, i) => DAY_START_HOUR + i,
  );

  function land(userId: string, startsAt: string, endsAt: string): void {
    const drag = state.drag;
    if (!drag) return;
    if (drag.source === "board") {
      void dropMove(drag.id, userId, startsAt, endsAt);
    } else {
      const item = state.unassigned.find((t) => t.id === drag.id);
      if (!item) return;
      void dropAssign(item, userId, startsAt, endsAt, 0);
    }
  }
  landRef.current = land;

  return (
    <div className="flex min-h-screen flex-col" style={{ background: "var(--m-canvas)" }}>
      <AppHeader />

      {overCapacity > 0 && (
        <div
          role="status"
          className="px-4 py-1 text-[11px]"
          style={{
            color: "var(--m-status-warning-fg)",
            borderBottom: "1px solid var(--m-border)",
            background: "var(--m-surface)",
          }}
        >
          {overCapacity} technician{overCapacity === 1 ? "" : "s"} over
          capacity — Ember bars mark who
        </div>
      )}

      <div className="flex flex-1 overflow-hidden">
        {/* TEAM rail */}
        <aside
          className="w-56 shrink-0 overflow-auto border-r p-3"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          aria-label="Team capacity"
        >
          <h2
            className="mb-2 text-[10px] font-semibold uppercase tracking-wide"
            style={{ color: "var(--m-text-muted)" }}
          >
            Team
          </h2>
          {state.technicians.map((t) => {
            const over = t.utilization_percent > 100;
            const widthPct = Math.min(t.utilization_percent, 130) / 1.3;
            const h = Math.floor(t.booked_minutes / 60);
            const m = t.booked_minutes % 60;
            return (
              <div key={t.user_id} className="mb-3">
                <div className="flex items-center gap-1.5">
                  <span
                    title={presentSet.has(t.user_id) ? "Online" : "Offline"}
                    className="inline-block h-1.5 w-1.5 shrink-0 rounded-full"
                    style={{
                      background: presentSet.has(t.user_id)
                        ? "var(--m-status-success-fg)"
                        : "var(--m-border-strong)",
                    }}
                  />
                  <span
                    className="truncate text-[12px]"
                    style={{ color: "var(--m-text)" }}
                  >
                    {t.display_name}
                  </span>
                  <span
                    className="ml-auto shrink-0 tabular-nums text-[10px]"
                    style={{
                      color: over
                        ? "var(--m-status-warning-fg)"
                        : "var(--m-text-muted)",
                    }}
                  >
                    {h}:{String(m).padStart(2, "0")}/
                    {Math.round(t.available_minutes / 60)}h
                  </span>
                </div>
                <div
                  className="mt-1 h-1 overflow-hidden rounded-full"
                  style={{ background: "var(--m-border)" }}
                  role="meter"
                  aria-valuenow={t.utilization_percent}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-label={`${t.display_name} utilization ${t.utilization_percent}%`}
                >
                  <div
                    className="h-full rounded-full"
                    style={{
                      width: `${widthPct}%`,
                      background: over
                        ? "var(--m-accent)"
                        : "var(--m-text-secondary)",
                      opacity: over ? 1 : 0.7,
                    }}
                  />
                </div>
              </div>
            );
          })}
          <Button variant="ghost" size="sm" onClick={() => void board.reload()}>
            Refresh
          </Button>
        </aside>

        {/* Board grid */}
        <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
          <div className="flex items-center justify-between px-4 py-2">
            <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
              Dispatch Board
              {state.drag && (
                <span className="ml-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  moving “{state.drag.title}” — arrows move · Enter drops · Esc cancels
                </span>
              )}
            </h1>
            <div className="flex items-center gap-1">
              <Button variant="ghost" size="sm" aria-label="Previous day" onClick={() => board.setDate(isoOffset(viewDate, -1))}>
                ←
              </Button>
              <span className="tabular-nums text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                {new Date(dayBaseMs).toLocaleDateString(undefined, {
                  weekday: "short",
                  day: "numeric",
                  month: "short",
                  timeZone: "UTC",
                })}
              </span>
              <Button variant="ghost" size="sm" aria-label="Next day" onClick={() => board.setDate(isoOffset(viewDate, 1))}>
                →
              </Button>
              <Button variant="secondary" size="sm" onClick={() => board.setDate(isoDay(0))}>
                Today
              </Button>
            </div>
          </div>

          {board.loadError && (
            <p role="alert" className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {board.loadError}
            </p>
          )}

          <div className="flex-1 overflow-auto px-4 pb-4">
            <div
              className="overflow-x-auto rounded-[var(--m-radius-control)] border"
              style={{ borderColor: "var(--m-border)", background: "var(--m-paper)" }}
            >
              <div className="min-w-[820px]">
                <div
                  className="grid border-b"
                  style={{
                    gridTemplateColumns: colTemplate(state.technicians.length),
                    borderColor: "var(--m-border)",
                  }}
                >
                  <div />
                  {state.technicians.map((t) => (
                    <div key={t.user_id} className="px-2 py-2 text-center">
                      <div className="truncate text-[12px] font-medium" style={{ color: "var(--m-text)" }}>
                        {t.display_name}
                      </div>
                      <div className="tabular-nums text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                        {t.utilization_percent}% booked
                      </div>
                    </div>
                  ))}
                </div>

                <div
                  className="grid"
                  style={{ gridTemplateColumns: colTemplate(state.technicians.length) }}
                >
                  <div>
                    {hoursAxis.map((h) => (
                      <div
                        key={h}
                        className="pr-2 text-right text-[10px]"
                        style={{
                          height: ROW_PX,
                          transform: "translateY(-6px)",
                          color: "var(--m-text-muted)",
                        }}
                      >
                        {String(h).padStart(2, "0")}:00
                      </div>
                    ))}
                  </div>

                  {state.technicians.map((tech) => (
                    <Column
                      key={tech.user_id}
                      userId={tech.user_id}
                      dayBaseMs={dayBaseMs}
                      appointments={tech.appointments}
                      exclusions={tech.exclusions}
                      onCall={
                        rawView?.technicians.find(
                          (r) => r.user_id === tech.user_id,
                        )?.on_call ?? []
                      }
                      conflicts={techConflictIds(tech)}
                      ghost={
                        state.ghost?.userId === tech.user_id
                          ? state.ghost
                          : null
                      }
                      isDragged={(id) =>
                        state.drag?.source === "board" && state.drag.id === id
                      }
                      dragDurationMin={state.drag?.durationMin ?? 60}
                      error={state.error}
                      onProposeSlot={(startsAt, endsAt) =>
                        dispatch({
                          type: "ghost_move",
                          userId: tech.user_id,
                          startIso: startsAt,
                          endIso: endsAt,
                        })
                      }
                      onDropSlot={land}
                      onPickUp={(b) =>
                        dispatch({
                          type: "drag_start",
                          source: "board",
                          id: b.id,
                          title: b.title,
                          durationMin: spanMinutes(b),
                        })
                      }
                    />
                  ))}
                </div>
              </div>
            </div>
          </div>
        </main>

        {/* Unassigned queue rail */}
        <aside
          className="flex w-72 shrink-0 flex-col border-l"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          aria-label="Unassigned queue"
        >
          <div className="flex items-center justify-between px-3 py-2">
            <h2
              className="text-[10px] font-semibold uppercase tracking-wide"
              style={{ color: "var(--m-text-muted)" }}
            >
              Unassigned ({filteredQueue.length})
            </h2>
            <Button variant="ghost" size="sm" aria-label="Refresh queue" onClick={() => void board.reload()}>
              ⟳
            </Button>
          </div>
          <div className="flex gap-1 px-3 pb-2">
            {PRIORITY_FILTERS.map((p) => (
              <button
                key={p}
                type="button"
                aria-pressed={priorityFilter === p}
                onClick={() => setPriorityFilter(p)}
                className="rounded-[2px] border px-1.5 py-0.5 text-[10px] uppercase"
                style={{
                  borderColor:
                    priorityFilter === p ? "var(--m-accent)" : "var(--m-border)",
                  color:
                    priorityFilter === p
                      ? "var(--m-accent)"
                      : "var(--m-text-muted)",
                  background: "transparent",
                }}
              >
                {p}
              </button>
            ))}
          </div>
          <div className="flex-1 space-y-2 overflow-auto px-3 pb-3">
            {filteredQueue.map((item) => (
              <QueueCard
                key={item.id}
                item={item}
                dragged={state.drag?.id === item.id && state.drag.source === "queue"}
                onPickUp={() =>
                  dispatch({
                    type: "drag_start",
                    source: "queue",
                    id: item.id,
                    title: `${item.number} ${item.title}`,
                  })
                }
                onCancel={() => dispatch({ type: "drag_end" })}
              />
            ))}
            {filteredQueue.length === 0 && (
              <p className="pt-2 text-center text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Queue clear.
              </p>
            )}
          </div>
        </aside>
      </div>

      {/* Status bar */}
      <footer
        className="flex items-center gap-4 border-t px-4 py-1 text-[11px]"
        style={{
          borderColor: "var(--m-border)",
          background: "var(--m-surface)",
          color: "var(--m-text-muted)",
        }}
      >
        <span>Queue: {state.unassigned.length}</span>
        <span>Conflicts: {conflictCount}</span>
        <span>Presence: {state.presence.length} online</span>
        <span>Auto-assign: Off</span>
        <span>Time zone: UTC axis</span>
        {auth.me && <span>User: {auth.me.display_name}</span>}
      </footer>

      <div aria-live="polite" role="status" className="sr-only">
        {liveMessage}
      </div>
    </div>
  );
}

function colTemplate(n: number): string {
  return `56px repeat(${Math.max(n, 1)}, minmax(120px, 1fr))`;
}

/** Per-tech ids of blocks that violate availability/exclusions. */
function techConflictIds(tech: Tech): Set<string> {
  const out = new Set<string>();
  for (const a of tech.appointments) {
    if (conflictOf(tech, a)) out.add(a.id);
  }
  return out;
}

type ConflictLike = { kind: string; blockerTitle?: string };

function conflictOf(
  tech: Pick<Tech, "appointments" | "availability_windows" | "exclusions">,
  a: Block,
): ConflictLike | null {
  // Banner-level cheap checks mirroring the reducer module (its tests pin
  // the exact semantics): overlap vs OTHER live blocks first, then
  // exclusion bites.
  const others = tech.appointments.filter(
    (x) => x.id !== a.id && x.status !== "cancelled",
  );
  const overlapBlocker = others.find(
    (x) =>
      Date.parse(x.starts_at) < Date.parse(a.ends_at) &&
      Date.parse(a.starts_at) < Date.parse(x.ends_at),
  );
  if (overlapBlocker) return { kind: "overlap", blockerTitle: overlapBlocker.title };
  const excluded = tech.exclusions.some(
    (x) =>
      Date.parse(x.starts_at) < Date.parse(a.ends_at) &&
      Date.parse(a.starts_at) < Date.parse(x.ends_at),
  );
  if (excluded) return { kind: "outside_availability" };
  return null;
}

// --- column ---------------------------------------------------------------

function Column(props: {
  userId: string;
  dayBaseMs: number;
  dragDurationMin: number;
  appointments: Block[];
  exclusions: Array<{ id: string; starts_at: string; ends_at: string }>;
  onCall: OnCall[];
  conflicts: Set<string>;
  ghost: { startIso: string; endIso: string; conflict: ConflictLike | null } | null;
  isDragged: (id: string) => boolean;
  error: { key: string; message: string } | null;
  onProposeSlot: (startsAt: string, endsAt: string) => void;
  onDropSlot: (userId: string, startsAt: string, endsAt: string) => void;
  onPickUp: (b: Block) => void;
}) {
  const columnRef = useRef<HTMLDivElement>(null);
  const lastStart = useRef<string | null>(null);

  const slotFromY = useCallback(
    (clientY: number): string => {
      const el = columnRef.current!;
      const rect = el.getBoundingClientRect();
      const y = clientY - rect.top + el.scrollTop;
      const rawMin = (y / ROW_PX) * 60 + DAY_START_HOUR * 60;
      const snapped =
        Math.max(DAY_START_HOUR * 60, Math.round(rawMin / SNAP_MIN) * SNAP_MIN) -
        DAY_START_HOUR * 60;
      return new Date(props.dayBaseMs + snapped * 60_000).toISOString();
    },
    [props.dayBaseMs],
  );

  return (
    <div
      ref={columnRef}
      className="relative border-l"
      style={{
        height: ROW_PX * (DAY_END_HOUR - DAY_START_HOUR),
        borderColor: "var(--m-border)",
      }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        const startsAt = slotFromY(e.clientY);
        if (lastStart.current !== startsAt) {
          lastStart.current = startsAt;
          props.onProposeSlot(
            startsAt,
            addMinutesIso(startsAt, props.dragDurationMin),
          );
        }
      }}
      onDrop={(e) => {
        e.preventDefault();
        const startsAt = slotFromY(e.clientY);
        props.onDropSlot(
          props.userId,
          startsAt,
          addMinutesIso(startsAt, props.dragDurationMin),
        );
        lastStart.current = null;
      }}
    >
      {/* hour gridlines */}
      {hoursList().map((i) => (
        <div
          key={i}
          className="border-t"
          style={{ height: ROW_PX, borderColor: "var(--m-border)" }}
        />
      ))}

      {/* time-off + on-call shading */}
      {props.exclusions.map((x) => (
        <Shade
          key={x.id}
          top={topFor(x.starts_at, props.dayBaseMs)}
          height={heightFor(x.starts_at, x.ends_at)}
          style={{
            background:
              "repeating-linear-gradient(45deg, transparent 0 5px, var(--m-border-strong) 5px 6px)",
            opacity: 0.5,
          }}
        />
      ))}
      {props.onCall.map((oc) => (
        <Shade
          key={oc.id}
          top={topFor(oc.starts_at, props.dayBaseMs)}
          height={heightFor(oc.starts_at, oc.ends_at)}
          style={{ background: "var(--m-status-neutral-bg)" }}
          label={`On call · ${oc.category}`}
        />
      ))}

      {/* ghost preview */}
      {props.ghost && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-1 rounded-[2px] px-1"
          style={{
            top: topFor(props.ghost.startIso, props.dayBaseMs),
            height: heightFor(props.ghost.startIso, props.ghost.endIso),
            border: "2px dashed var(--m-accent)",
            background: "transparent",
            color: "var(--m-accent)",
          }}
        >
          <span className="text-[10px] font-medium">
            {utcTimeLabel(props.ghost.startIso)}
            {props.ghost.conflict
              ? props.ghost.conflict.kind === "overlap"
                ? ` — conflict: ${props.ghost.conflict.blockerTitle}`
                : " — outside working hours"
              : ""}
          </span>
        </div>
      )}

      {/* bookings */}
      {props.appointments.map((a) => {
        const conflicted = props.conflicts.has(a.id);
        return (
          <BookedBlock
            key={a.id}
            block={a}
            dayBaseMs={props.dayBaseMs}
            conflicted={conflicted}
            dragging={props.isDragged(a.id)}
            inlineError={
              props.error && props.error.key === a.id ? props.error.message : null
            }
            onPickUp={props.onPickUp}
          />
        );
      })}
    </div>
  );
}

function hoursList(): number[] {
  return Array.from({ length: DAY_END_HOUR - DAY_START_HOUR }, (_, i) => i);
}

function topFor(iso: string, dayBaseMs: number): number {
  const min = (Date.parse(iso) - dayBaseMs) / 60000 - DAY_START_HOUR * 60;
  return Math.max(0, min) * (ROW_PX / 60);
}

function heightFor(startIso: string, endIso: string): number {
  const min = Math.round((Date.parse(endIso) - Date.parse(startIso)) / 60000);
  return Math.max(min, SNAP_MIN) * (ROW_PX / 60) - 2;
}

function Shade(props: {
  top: number;
  height: number;
  style: React.CSSProperties;
  label?: string;
}) {
  return (
    <div
      aria-label={props.label}
      className="pointer-events-none absolute inset-x-0 overflow-hidden"
      style={{ ...props.style, top: props.top, height: props.height }}
    />
  );
}

// --- booked block ---------------------------------------------------------

function BookedBlock(props: {
  block: Block;
  dayBaseMs: number;
  conflicted: boolean;
  dragging: boolean;
  inlineError: string | null;
  onPickUp: (b: Block) => void;
}) {
  const { block: b, dayBaseMs } = props;
  const top = topFor(b.starts_at, dayBaseMs);
  const travelMin = b.travel_minutes ?? 0;
  const travelPx = Math.min(travelMin, 45) * (ROW_PX / 60) - 2;
  return (
    <>
      {travelMin > 0 && travelPx > 0 && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-1"
          style={{
            top: Math.max(0, top - travelPx),
            height: travelPx,
            borderRadius: 2,
            border: "1px dashed var(--m-border-strong)",
            background:
              "repeating-linear-gradient(135deg, transparent 0 4px, var(--m-border-strong) 4px 5px)",
          }}
          title={`${travelMin}m travel`}
        />
      )}
      <button
        type="button"
        aria-label={`${b.title}, ${utcTimeLabel(b.starts_at)} to ${utcTimeLabel(b.ends_at)}. Press Enter to pick up${props.conflicted ? ", currently conflicting with availability" : ""}.`}
        onClick={() => props.onPickUp(b)}
        className="absolute inset-x-1 cursor-grab overflow-auto px-1 py-0.5 text-left focus-visible:ring-2"
        style={{
          top,
          height: heightFor(b.starts_at, b.ends_at),
          borderRadius: 2,
          background: props.conflicted
            ? "var(--m-status-warning-bg)"
            : "var(--m-surface)",
          border: props.conflicted
            ? "1px solid var(--m-status-warning-fg)"
            : "1px solid var(--m-border-strong)",
          opacity: props.dragging ? 0.4 : 1,
          boxShadow: props.dragging ? "var(--m-shadow-overlay)" : undefined,
        }}
        draggable
        onDragStart={(e) => {
          e.dataTransfer.effectAllowed = "move";
          e.dataTransfer.setData("text/plain", b.id);
          props.onPickUp(b);
        }}
        onDragEnd={undefined}
      >
        <span
          className="block truncate text-[11px] font-medium leading-tight"
          style={{
            color: props.conflicted
              ? "var(--m-status-warning-fg)"
              : "var(--m-text)",
          }}
        >
          {b.ticket_number ? `${b.ticket_number} ` : ""}
          {props.conflicted ? "⚠ " : ""}
          {b.title}
        </span>
        <span
          className="block truncate text-[10px]"
          style={{ color: "var(--m-text-secondary)" }}
        >
          {utcTimeLabel(b.starts_at)}–{utcTimeLabel(b.ends_at)}
          {b.client_name ? ` · ${b.client_name}` : ""}
        </span>
        {props.inlineError && (
          <span
            className="block truncate text-[10px]"
            style={{ color: "var(--m-status-danger-fg)" }}
          >
            {props.inlineError}
          </span>
        )}
      </button>
    </>
  );
}

// --- queue card -----------------------------------------------------------

function QueueCard(props: {
  item: QueueItem;
  dragged: boolean;
  onPickUp: () => void;
  onCancel: () => void;
}) {
  const sla = slaCountdown(props.item);
  const p1 = props.item.priority === "p1";
  return (
    <button
      type="button"
      draggable
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("text/plain", props.item.id);
        props.onPickUp();
      }}
      onDragEnd={props.onCancel}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          props.onPickUp();
        } else if (e.key === "Escape") {
          props.onCancel();
        }
      }}
      className={`w-full cursor-grab rounded-[2px] border p-2 text-left${props.dragged ? " opacity-40" : ""}`}
      style={{
        borderColor: props.dragged ? "var(--m-accent)" : "var(--m-border)",
        background: "var(--m-paper)",
      }}
      aria-label={`Queue item ${props.item.number}: ${props.item.title}. Priority ${props.item.priority}.${sla ? ` ${sla.label}` : ""}. Press Enter to pick up, then choose a technician.`}
    >
      <span className="flex items-center gap-1.5">
        <span
          className="rounded-[2px] px-1 py-0 text-[9px] uppercase tabular-nums"
          style={{
            border: `1px solid ${p1 ? "var(--m-accent)" : "var(--m-border-strong)"}`,
            color: p1 ? "var(--m-accent)" : "var(--m-text-secondary)",
          }}
        >
          {props.item.priority.toUpperCase()}
        </span>
        <span className="truncate text-[11px] font-medium" style={{ color: "var(--m-text)" }}>
          {props.item.number}
        </span>
        {sla && (
          <span
            className="ml-auto shrink-0 tabular-nums text-[10px]"
            style={{
              color: sla.warn
                ? "var(--m-status-warning-fg)"
                : "var(--m-text-muted)",
            }}
          >
            {sla.label}
          </span>
        )}
      </span>
      <span
        className="mt-0.5 block truncate text-[11px]"
        style={{ color: "var(--m-text-secondary)" }}
      >
        {props.item.title}
      </span>
    </button>
  );
}
