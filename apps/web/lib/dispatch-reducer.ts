// Dispatch board drag-state machine (TASKLIST 2.19).
//
// Purpose: a PURE reducer owning everything interactive about the
// dispatch board so the drag-and-drop logic is testable headlessly:
// which queue/board item is being dragged, where the dashed ghost
// preview sits and whether that placement would violate availability or
// overlap a booking (the conflict badge), optimistic drop state with
// snapshot rollback on typed rejection, live reconciliation of
// appointment created/updated/cancelled events, and presence dots.
//
// Non-purpose: no network I/O (the page drives fetches and maps results
// onto actions), no rendering. All instants are ISO strings; weekly
// availability TEMPLATES are expanded to the concrete day inside
// findConflict via lib/zoned-time so the UI needs no timezone logic.

export interface AvailabilityWindow {
  weekday: number;
  start_local: string;
  end_local: string;
  timezone: string;
}

export interface Exclusion {
  id: string;
  starts_at: string;
  ends_at: string;
}

export interface Block {
  id: string;
  user_id: string;
  ticket_id: string | null;
  ticket_number: string | null;
  client_name: string | null;
  title: string;
  starts_at: string;
  ends_at: string;
  status: string;
  /** Travel placeholder rendered before the block face (minutes). */
  travel_minutes?: number;
}

/** Loose structural mirror of TechnicianView (keeps tests SDK-free). */
export interface Tech {
  user_id: string;
  display_name: string;
  available_minutes: number;
  booked_minutes: number;
  utilization_percent: number;
  appointments: Block[];
  availability_windows: AvailabilityWindow[];
  exclusions: Exclusion[];
}

export interface QueueItem {
  id: string;
  number: string;
  title: string;
  priority: string;
  due_at: string | null;
  sla_resolution_due: string | null;
}

export interface BoardState {
  technicians: Tech[];
  unassigned: QueueItem[];
  drag: {
    source: "queue" | "board";
    id: string;
    title: string;
    durationMin: number;
  } | null;
  /** Dashed preview target; conflict recomputed on every move. */
  ghost: {
    userId: string;
    startIso: string;
    endIso: string;
    conflict: Conflict | null;
  } | null;
  /** Optimistic writes awaiting confirmation keyed by appointment id
   * (moves) or temp id (queue assigns), holding the rollback snapshot. */
  pending: Record<
    string,
    { kind: "move" | "assign"; before?: Block; block?: Block }
  >;
  error: { key: string; message: string } | null;
  presence: string[];
  /** aria-live announcement (most recent wins). */
  announcement: string;
}

export type Conflict =
  | { kind: "overlap"; blockerTitle: string }
  | { kind: "outside_availability" };

export type DispatchAction =
  | { type: "loaded"; technicians: Tech[]; unassigned: QueueItem[] }
  | { type: "presence"; userIds: string[] }
  | {
      type: "drag_start";
      source: "queue" | "board";
      id: string;
      title: string;
      /** Drop-span length: board drags keep their booking length; queue
       * drops default to 60 minutes. */
      durationMin?: number;
    }
  | { type: "drag_end" }
  | { type: "ghost_move"; userId: string; startIso: string; endIso: string }
  | { type: "ghost_clear" }
  | {
      type: "drop_move";
      apptId: string;
      userId: string;
      startsAt: string;
      endsAt: string;
    }
  | {
      type: "drop_assign";
      tempId: string;
      ticket: QueueItem;
      userId: string;
      startsAt: string;
      endsAt: string;
    }
  | { type: "confirm"; key: string }
  | { type: "reject"; key: string; message: string }
  | { type: "reconcile_upsert"; block: Block }
  | { type: "reconcile_remove"; apptId: string };

const MIN = 60_000;

// --- placement helpers -------------------------------------------------------

function sortBlocks(blocks: Block[]): Block[] {
  return [...blocks].sort(
    (a, b) => Date.parse(a.starts_at) - Date.parse(b.starts_at),
  );
}

function unionMinutes(spans: readonly (readonly [number, number])[]): number {
  const sorted = [...spans].sort((a, b) => a[0] - b[0]);
  let total = 0;
  let curEnd = Number.NEGATIVE_INFINITY;
  for (const [s, e] of sorted) {
    if (e <= s) continue;
    if (s > curEnd) {
      total += e - s;
      curEnd = e;
    } else if (e > curEnd) {
      total += e - curEnd;
      curEnd = e;
    }
  }
  return Math.round(total / MIN);
}

function techWithStats(tech: Tech, appointments: Block[]): Tech {
  const booked = unionMinutes(
    appointments.map(
      (a) => [Date.parse(a.starts_at), Date.parse(a.ends_at)] as const,
    ),
  );
  return {
    ...tech,
    appointments: sortBlocks(appointments),
    booked_minutes: booked,
    utilization_percent:
      tech.available_minutes > 0
        ? Math.round((booked * 100) / tech.available_minutes)
        : 0,
  };
}

/** Removes a block wherever it currently sits (id-keyed, all columns). */
export function withoutBlock(techs: Tech[], blockId: string): Tech[] {
  return techs.map((t) => ({
    ...t,
    appointments: t.appointments.filter((a) => a.id !== blockId),
  }));
}

/** Removes any existing copy of the block, then lands it on its owner. */
export function withBlock(techs: Tech[], block: Block): Tech[] {
  const stripped = withoutBlock(techs, block.id);
  const owner = stripped.find((t) => t.user_id === block.user_id);
  if (!owner) return stripped;
  return stripped.map((t) =>
    t.user_id === block.user_id
      ? techWithStats(t, [...t.appointments, block])
      : t,
  );
}

// --- conflict detection ------------------------------------------------------

/**
 * Why would the server reject / flag this placement?
 *
 * Precedence: overlap beats outside-availability because the schema's
 * EXCLUDE constraint HARD-blocks overlaps while booking outside working
 * hours is legal-but-flagged (the board's amber treatment). Availability
 * comes from expanding the technician's weekly templates around the
 * placement date (in each template's own timezone), minus exclusions.
 */
export function findConflict(
  tech: Pick<Tech, "appointments" | "availability_windows" | "exclusions">,
  startIso: string,
  endIso: string,
  ignoreIds: string[] = [],
): Conflict | null {
  const blocker = tech.appointments.find(
    (a) =>
      !ignoreIds.includes(a.id) &&
      a.status !== "cancelled" &&
      spansOverlap(startIso, endIso, a.starts_at, a.ends_at),
  );
  if (blocker) return { kind: "overlap", blockerTitle: blocker.title };

  const excluded = tech.exclusions.some((x) =>
    spansOverlap(startIso, endIso, x.starts_at, x.ends_at),
  );
  if (excluded) return { kind: "outside_availability" };

  const covered = coveredSpans(tech, startIso);
  const uncovered = !coversSpans(covered, [
    [Date.parse(startIso), Date.parse(endIso)],
  ]);
  if (uncovered) return { kind: "outside_availability" };
  return null;
}

function spansOverlap(
  aStart: string,
  aEnd: string,
  bStart: string,
  bEnd: string,
): boolean {
  return (
    Date.parse(aStart) < Date.parse(bEnd) && Date.parse(bStart) < Date.parse(aEnd)
  );
}

/**
 * Expands weekly availability templates into ABSOLUTE spans covering the
 * day containing `referenceIso`, honoring each template's own timezone.
 * Weekday values follow the API convention (0=Monday .. 6=Sunday).
 */
export function coveredSpans(
  tech: Pick<Tech, "availability_windows">,
  referenceIso: string,
): Array<readonly [number, number]> {
  return tech.availability_windows.flatMap((w) => {
    const localDay = zonedDayInfo(referenceIso, w.timezone);
    if (!localDay || localDay.weekday !== w.weekday) return [];
    const [sh, sm] = w.start_local.split(":").map(Number);
    const [eh, em] = w.end_local.split(":").map(Number);
    const start = zonedInstant(
      w.timezone,
      localDay.y,
      localDay.m,
      localDay.d,
      sh ?? 0,
      sm ?? 0,
    );
    const end = zonedInstant(
      w.timezone,
      localDay.y,
      localDay.m,
      localDay.d,
      eh ?? 0,
      em ?? 0,
    );
    if (start === null || end === null || end <= start) return [];
    return [[start, end] as const];
  });
}

/** True when every given span lies fully inside at least one cover span. */
export function coversSpans(
  coverSpans: Array<readonly [number, number]>,
  required: Array<readonly [number, number]>,
): boolean {
  return required.every(([rs, re]) =>
    coverSpans.some(([cs, ce]) => cs <= rs && re <= ce),
  );
}

// --- timezone plumbing (Intl-based, no external deps) -------------------------

function tzOffsetMs(tz: string, at: Date): number | null {
  try {
    const dtf = new Intl.DateTimeFormat("en-US", {
      timeZone: tz,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hourCycle: "h23",
    });
    const parts = dtf.formatToParts(at);
    const get = (k: string) =>
      Number(parts.find((p) => p.type === k)?.value ?? "NaN");
    const asUtc = Date.UTC(
      get("year"),
      get("month") - 1,
      get("day"),
      get("hour"),
      get("minute"),
      get("second"),
    );
    return asUtc - at.getTime();
  } catch {
    return null;
  }
}

/**
 * Absolute epoch ms for a WALL-CLOCK time in `tz` (DST-aware two-pass
 * correction; ambiguous/gap instants resolve to the earlier offset).
 */
export function zonedInstant(
  tz: string,
  y: number,
  m: number,
  d: number,
  hh: number,
  mm: number,
): number | null {
  const naive = Date.UTC(y, m - 1, d, hh, mm);
  const off1 = tzOffsetMs(tz, new Date(naive));
  if (off1 === null) return null;
  const off2 = tzOffsetMs(tz, new Date(naive - off1));
  if (off2 === null) return null;
  return naive - off2;
}

/** Calendar date + API weekday (Mon=0..Sun=6) of an instant in `tz`. */
export function zonedDayInfo(
  iso: string,
  tz: string,
): { y: number; m: number; d: number; weekday: number } | null {
  try {
    const parts = new Intl.DateTimeFormat("en-US", {
      timeZone: tz,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      weekday: "short",
    }).formatToParts(new Date(iso));
    const get = (k: string) =>
      parts.find((p) => p.type === k)?.value ?? "";
    const names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const weekday = names.indexOf(get("weekday"));
    return {
      y: Number(get("year")),
      m: Number(get("month")),
      d: Number(get("day")),
      weekday,
    };
  } catch {
    return null;
  }
}

// --- reducer ----------------------------------------------------------------

export function initialDispatchState(): BoardState {
  return {
    technicians: [],
    unassigned: [],
    drag: null,
    ghost: null,
    pending: {},
    error: null,
    presence: [],
    announcement: "",
  };
}

export function dispatchReducer(
  state: BoardState,
  action: DispatchAction,
): BoardState {
  switch (action.type) {
    case "loaded":
      return {
        ...state,
        technicians: action.technicians,
        unassigned: action.unassigned,
      };

    case "presence":
      return { ...state, presence: action.userIds };

    case "drag_start":
      return {
        ...state,
        error: null,
        drag: {
          source: action.source,
          id: action.id,
          title: action.title,
          durationMin: action.durationMin ?? 60,
        },
        announcement:
          action.source === "board"
            ? `Picked up ${action.title}. Choose a slot, then press Enter to drop or Escape to cancel.`
            : `Assigning ${action.title}. Pick a technician and time.`,
      };

    case "drag_end":
      return { ...state, drag: null, ghost: null };

    case "ghost_move": {
      if (!state.drag) return state;
      const tech = state.technicians.find((t) => t.user_id === action.userId);
      if (!tech) return state;
      const ignoreIds = state.drag.source === "board" ? [state.drag.id] : [];
      const conflict = findConflict(
        tech,
        action.startIso,
        action.endIso,
        ignoreIds,
      );
      return {
        ...state,
        ghost: {
          userId: action.userId,
          startIso: action.startIso,
          endIso: action.endIso,
          conflict,
        },
        announcement: conflict
          ? conflict.kind === "overlap"
            ? `Conflicts with ${conflict.blockerTitle}`
            : "Outside working hours"
          : `Drop target ${tech.display_name}, no conflict`,
      };
    }

    case "ghost_clear":
      return { ...state, ghost: null };

    case "drop_move": {
      const before = state.technicians
        .flatMap((t) => t.appointments)
        .find((a) => a.id === action.apptId);
      if (!before) return state;
      const moved: Block = {
        ...before,
        user_id: action.userId,
        starts_at: action.startsAt,
        ends_at: action.endsAt,
      };
      return {
        ...state,
        ghost: null,
        drag: null,
        pending: {
          ...state.pending,
          [action.apptId]: { kind: "move", before },
        },
        technicians: withBlock(
          withoutBlock(state.technicians, action.apptId),
          moved,
        ),
        announcement: `Moved ${moved.title}; confirming`,
      };
    }

    case "drop_assign": {
      const block: Block = {
        id: action.tempId,
        user_id: action.userId,
        ticket_id: action.ticket.id,
        ticket_number: action.ticket.number,
        client_name: action.ticket.title,
        title: `${action.ticket.number} ${action.ticket.title}`,
        starts_at: action.startsAt,
        ends_at: action.endsAt,
        status: "scheduled",
      };
      return {
        ...state,
        ghost: null,
        drag: null,
        pending: {
          ...state.pending,
          [action.tempId]: { kind: "assign", block },
        },
        unassigned: state.unassigned.filter((t) => t.id !== action.ticket.id),
        technicians: withBlock(state.technicians, block),
        announcement: `Assigned ${block.title}; confirming`,
      };
    }

    case "confirm": {
      const entry = state.pending[action.key];
      if (!entry) return state;
      const rest = { ...state.pending };
      delete rest[action.key];
      // Assign optimism used a TEMP block id: clear it here and let the
      // authoritative appointment (the drop response or a reload) land via
      // reconcile_upsert under its real id.
      const technicians =
        entry.kind === "assign"
          ? withoutBlock(state.technicians, action.key)
          : state.technicians;
      return {
        ...state,
        pending: rest,
        technicians,
        error: null,
        announcement: "Confirmed",
      };
    }

    case "reject": {
      const entry = state.pending[action.key];
      if (!entry) {
        return { ...state, error: { key: action.key, message: action.message } };
      }
      let technicians = state.technicians;
      let unassigned = state.unassigned;
      if (entry.kind === "move" && entry.before) {
        // Same appointment id throughout: strip wherever it landed, put
        // the snapshot back where it came from.
        technicians = withBlock(
          withoutBlock(state.technicians, entry.before.id),
          entry.before,
        );
      } else if (entry.kind === "assign" && entry.block) {
        technicians = withoutBlock(state.technicians, entry.block.id);
        if (
          entry.block.ticket_id &&
          !unassigned.some((t) => t.id === entry.block!.ticket_id)
        ) {
          // Minimal queue restoration; authoritative order returns with
          // the next reload/reconcile pass.
          unassigned = [
            ...unassigned,
            {
              id: entry.block.ticket_id,
              number: entry.block.ticket_number ?? "",
              title: entry.block.client_name ?? "",
              priority: "p3",
              due_at: null,
              sla_resolution_due: null,
            },
          ];
        }
      }
      const rest = { ...state.pending };
      delete rest[action.key];
      return {
        ...state,
        pending: rest,
        technicians,
        unassigned,
        error: { key: action.key, message: action.message },
        announcement: `Rejected: ${action.message}`,
      };
    }

    case "reconcile_upsert": {
      // Local pending entries own their block until confirm/reject;
      // echoing server events for them would fight the optimistic copy.
      if (state.pending[action.block.id]) return state;
      if (
        !state.technicians.some((t) => t.user_id === action.block.user_id)
      ) {
        return state;
      }
      return { ...state, technicians: withBlock(state.technicians, action.block) };
    }

    case "reconcile_remove": {
      if (!state.technicians.some((t) =>
        t.appointments.some((a) => a.id === action.apptId),
      )) {
        return state;
      }
      return {
        ...state,
        technicians: withoutBlock(state.technicians, action.apptId),
      };
    }

    default:
      return state;
  }
}
