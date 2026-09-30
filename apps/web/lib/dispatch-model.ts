// Shared dispatch board model — Wave 41 lane 6 (p41-ui-dispatch).
//
// Purpose: the pure logic the pack fixes in DESIGN-NOTES.md §B3–§B6, in one
// testable module: the frozen time axis (07:00–18:00, 15-minute snapping),
// travel minutes between zones, utilisation counting overlapped time once by
// default, presence lines, collision detection, and the overlap-policy drop
// evaluator (§B5b) whose five outcomes drive the ghost, chip and reason card.
//
// Frozen data mirrors the pack exactly (clock 11:41 Tue 29 Sep 2026, timer
// 00:25:05) so lane 15 can diff captures against `web/dispatch` states.
//
// Non-purpose: no rendering, no network I/O. The page maps server shapes onto
// BoardJob/BoardTech and renders from the derived values here.

export const D0 = 420; // board start 07:00
export const D1 = 1080; // board end 18:00
export const SPAN = D1 - D0; // minutes across the board (11h)
export const CAP = 480; // a working day, minutes
export const NOW = 701; // 11:41, frozen

export type JobMode = "remote" | "site";
export type Pri = "P1" | "P2" | "P3" | "P4";

export interface BoardJob {
  jid: string;
  tk: number;
  title: string;
  client: string;
  clientZone: number;
  tech: string;
  s: number; // minutes from midnight
  d: number; // duration minutes
  mode: JobMode;
  pri: Pri;
  /** Set when an overlap was approved on purpose (the reason key). */
  ov?: OverlapReason;
  part?: string;
  prop?: boolean;
  num?: number;
  ri?: number;
}

export type OverlapReason = "parallel" | "waiting" | "handle";

export const REASON_LABEL: Record<OverlapReason, string> = {
  parallel: "Remote in parallel",
  waiting: "Waiting on the client",
  handle: "I'll handle it",
};

export interface BoardAbsence {
  jid: string;
  tech: string;
  s: number;
  d: number;
  title: string;
}

/** An absence as the collision walker sees it: the same shape plus the marker
 *  it sorts last on and the `tk: 0` the say-line reads. */
export type AbsenceItem = BoardAbsence & { abs: true; tk: 0 };

export interface BoardTech {
  id: string;
  name: string;
  role: string;
  sk: string[];
}

export interface QueueTicket {
  tk: number;
  title: string;
  client: string;
  clientZone: number;
  pri: Pri;
  est: number;
  mode: JobMode;
  need: string;
  /** SLA minutes left, frozen; null = no SLA. */
  sla: number | null;
}

export type Policy = "never" | "remote" | "ask";

export interface DropEval {
  ok: boolean;
  kind: "free" | "overlap" | "blocked";
  hits: BoardJob[];
  allRemote: boolean;
  sites: boolean;
  gap: number;
  msg: string;
  blockers?: { kind: "policy" | "limit" };
}

export const hm = (m: number): string =>
  `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;

export const dur = (m: number): string => {
  if (m >= 60) {
    const h = Math.floor(m / 60);
    const r = m % 60;
    return r ? `${h}h ${String(r).padStart(2, "0")}m` : `${h}h`;
  }
  return `${m}m`;
};

export const pct = (m: number): number => ((m - D0) / (D1 - D0)) * 100;

export const tm = (s: string): number => {
  const [h, m] = s.split(":");
  return Number(h) * 60 + Number(m);
};

export const overlaps = (a: { s: number; d: number }, b: { s: number; d: number }): boolean =>
  a.s < b.s + b.d && b.s < a.s + a.d;

/** Travel before an on-site job: 12 min plus 10 per zone apart, nothing for
 *  remote work and nothing between two jobs at the same client (§B3). */
export function travelMin(
  j: Pick<BoardJob, "s" | "mode" | "clientZone" | "client">,
  list: Array<Pick<BoardJob, "s" | "d" | "mode" | "clientZone" | "client">>,
): number {
  if (j.mode !== "site") return 0;
  const prev = list
    .filter((x) => x.s + x.d <= j.s)
    .sort((a, b) => b.s + b.d - (a.s + a.d))[0];
  let pz = 1;
  let pc: string | null = null;
  if (prev && prev.mode === "site") {
    pz = prev.clientZone;
    pc = prev.client;
  }
  if (pc === j.client) return 0;
  return 12 + 10 * Math.abs(pz - j.clientZone);
}
export interface Utilisation {
  min: number;
  pctv: number;
}

/** Busy (job + travel) minutes over 480. Overlapped time counts once by
 * default; `countFull` counts it in full (the settings switch). */
export function utilisation(
  jobs: BoardJob[],
  countFull: boolean,
): Utilisation {
  const sorted = [...jobs].sort((a, b) => a.s - b.s);
  const iv = sorted.map((j) => [j.s - travelMin(j, sorted), j.s + j.d] as const);
  let m = 0;
  if (!countFull) {
    iv.sort((a, b) => a[0] - b[0]);
    let cs: number | null = null;
    let ce = 0;
    for (const [a, b] of iv) {
      if (cs === null) {
        cs = a;
        ce = b;
      } else if (a <= ce) {
        ce = Math.max(ce, b);
      } else {
        m += ce - cs;
        cs = a;
        ce = b;
      }
    }
    if (cs !== null) m += ce - cs;
  } else {
    for (const [a, b] of iv) m += b - a;
  }
  return { min: m, pctv: Math.round((m / CAP) * 100) };
}

export interface Presence {
  dot: string;
  txt: string;
}

const DOT_OK = "var(--m-status-success-fg)";
const DOT_OFF = "var(--m-input-line)";

/** Presence line in the lane header: Remote with the ticket, On site with the
 * client, Free with the next job time, or Off today (§B3). */
export function presenceOf(
  t: BoardTech,
  jobs: BoardJob[],
  absences: BoardAbsence[],
): Presence {
  const mine = jobs.filter((j) => j.tech === t.id);
  const ab = absences.find((a) => a.tech === t.id && a.s <= NOW && NOW < a.s + a.d);
  if (ab) return { dot: DOT_OFF, txt: "Off today" };
  const live = mine.find((j) => j.s <= NOW && NOW < j.s + j.d);
  if (live)
    return {
      dot: DOT_OK,
      txt: live.mode === "site" ? `On site · ${live.client}` : `Remote · #${live.tk}`,
    };
  const nx = mine.filter((j) => j.s > NOW).sort((a, b) => a.s - b.s)[0];
  return { dot: DOT_OK, txt: nx ? `Free · next ${hm(nx.s)}` : "Free" };
}

export interface Collision {
  tech: string;
  a: BoardJob | AbsenceItem;
  b: BoardJob | AbsenceItem;
  s: number;
  e: number;
}

type Item = BoardJob | AbsenceItem;

const isAbs = (x: Item): x is AbsenceItem =>
  (x as AbsenceItem).abs === true;

/** Exported for the render layer, which branches on the same marker when it
 *  phrases a collision. */
export const isAbsItem = isAbs;

/** Collisions nobody chose: an unapproved overlap between two jobs, or either
 * side an absence. Approved overlaps (`.ov` on either side) never appear. */
export function collisionsOf(
  jobs: Item[],
  byTech: (tech: string) => BoardTech | undefined,
): Collision[] {
  const out: Collision[] = [];
  const groups = new Map<string, Item[]>();
  for (const j of jobs) {
    const g = groups.get(j.tech) ?? [];
    g.push(j);
    groups.set(j.tech, g);
  }
  for (const [tech, list] of groups) {
    if (!byTech(tech)) continue;
    for (let i = 0; i < list.length; i++) {
      for (let k = i + 1; k < list.length; k++) {
        const a = list[i];
        const b = list[k];
        if (!a || !b) continue;
        if (!overlaps(a, b)) continue;
        if (!isAbs(a) && !isAbs(b) && ((a as BoardJob).ov || (b as BoardJob).ov))
          continue;
        out.push({
          tech,
          a,
          b,
          s: Math.max(a.s, b.s),
          e: Math.min(a.s + a.d, b.s + b.d),
        });
      }
    }
  }
  out.sort(
    (x, y) =>
      Number(isAbs(x.a) || isAbs(x.b)) - Number(isAbs(y.a) || isAbs(y.b)),
  );
  return out;
}

/** Approved-overlap windows in a lane, merged when adjacent. */
export function approvedWindows(jobs: BoardJob[]): Array<{ s: number; e: number }> {
  const out: Array<{ s: number; e: number }> = [];
  for (let i = 0; i < jobs.length; i++) {
    for (let k = i + 1; k < jobs.length; k++) {
      const a = jobs[i];
      const b = jobs[k];
      if (!a || !b) continue;
      if (overlaps(a, b) && (a.ov || b.ov))
        out.push({ s: Math.max(a.s, b.s), e: Math.min(a.s + a.d, b.s + b.d) });
    }
  }
  out.sort((a, b) => a.s - b.s);
  const merged: Array<{ s: number; e: number }> = [];
  for (const w of out) {
    const l = merged[merged.length - 1];
    if (l && w.s <= l.e + 45) l.e = Math.max(l.e, w.e);
    else merged.push({ ...w });
  }
  return merged;}

/** Lane rows: jobs stacked into rows so approved overlaps split the lane
 * (§B4 Stacked). Approved pairs share a lane's two rows; nothing else does.
 * Returns the row count and stamps the row index on each job. */
export function stackRows(jobs: BoardJob[]): number {
  const rows: BoardJob[][] = [];
  for (const j of jobs) {
    let r = 0;
    for (; r < rows.length; r++) {
      if ((rows[r] ?? []).every((k) => !(overlaps(j, k) && (k.ov || j.ov)))) break;
    }
    if (r === rows.length) rows.push([]);
    rows[r]?.push(j);
    rowIndex.set(j, r);
  }
  return Math.max(1, rows.length);
}

/** Row index stamped by `stackRows`, read by the block renderer. */
export const rowIndex = new WeakMap<BoardJob, number>();

/** The §B5b drop evaluator. Five outcomes: free (gold ghost), overlap
 * (neutral chip + reason card), cross-site overlap (sharper chip), blocked
 * (blush hatch, release rejected), plus the policy/limit blockers. */
export function evalDrop(
  opts: {
    jid: string;
    tech: string;
    techName: string;
    s: number;
    d: number;
    mode: JobMode;
    clientZone: number;
    client: string;
  },
  laneJobs: BoardJob[],
  absences: BoardAbsence[],
  policy: Policy,
  limit: number,
): DropEval {
  const { jid, tech, techName, s, d, mode, clientZone, client } = opts;
  if (s < D0 || s + d > D1)
    return { ok: false, kind: "blocked", hits: [], allRemote: false, sites: false, gap: 0, msg: "Outside board hours" };
  const ab = absences.find((a) => a.tech === tech && s < a.s + a.d && a.s < s + d);
  if (ab)
    return {
      ok: false,
      kind: "blocked",
      hits: [],
      allRemote: false,
      sites: false,
      gap: 0,
      msg: `${techName} is off until ${hm(ab.s + ab.d)}`,
    };
  const hits = laneJobs.filter((j) => j.jid !== jid && s < j.s + j.d && j.s < s + d);
  if (!hits.length)
    return { ok: true, kind: "free", hits: [], allRemote: false, sites: false, gap: 0, msg: "" };
  const h0 = hits[0]!;
  const rng = `#${h0.tk} (${hm(h0.s)} to ${hm(h0.s + h0.d)})`;
  const allRemote = mode === "remote" && hits.every((h) => h.mode === "remote");
  if (policy === "never")
    return {
      ok: false, kind: "blocked", hits, allRemote, sites: false, gap: 0,
      msg: `Overlaps ${rng}. Overlapping work is switched off in Dispatch settings`,
    };
  if (policy === "remote" && !allRemote)
    return {
      ok: false, kind: "blocked", hits, allRemote, sites: false, gap: 0,
      msg: `Overlaps ${rng}. Only remote work can run in parallel`,
    };
  const pts = [s, ...hits.map((h) => h.s).filter((t) => t > s)];
  let mx = 1;
  for (const t of pts) {
    const c = 1 + hits.filter((h) => h.s <= t && t < h.s + h.d).length;
    if (c > mx) mx = c;
  }
  if (mx > limit)
    return {
      ok: false, kind: "blocked", hits, allRemote, sites: false, gap: 0,
      msg: `${techName} would have ${mx} jobs at once. Your limit is ${limit}`,
    };
  const sh = hits.find((h) => h.mode === "site" && mode === "site" && h.client !== client);
  const sites = !!sh;
  const gap = sh ? 12 + 10 * Math.abs(sh.clientZone - clientZone) : 0;
  return {
    ok: true,
    kind: "overlap",
    hits,
    allRemote,
    sites,
    gap,
    msg: sites ? `Different sites, ${gap} min apart` : `Overlaps ${rng}`,
  };
}

/** The status class of a block: done (before now), live (running now) or sched. */
export function statusOf(j: { s: number; d: number }): "done" | "live" | "sched" {
  if (j.s + j.d <= NOW) return "done";
  if (j.s <= NOW) return "live";
  return "sched";
}
