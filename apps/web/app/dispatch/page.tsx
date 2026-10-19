// /dispatch — the dispatch board, rebuilt to the Marula surface pack (Wave 41
// lane 6, p41-ui-dispatch).
//
// Purpose: the day board of DESIGN-NOTES.md §B — twelve technician lanes with
// utilisation bars and presence, the 288px unassigned queue with priority
// filter and select-all, the blush collision banner, hatched travel and
// absences, the dragged/caret ghost family, a full keyboard engine (arrows,
// Enter, +/-, S, U, Esc), the Week capacity matrix, the Unassigned data-surface
// view with the core kit's Columns/Filter/Export toolbar, the AI plan editor
// (352px right panel, not inverted) behind the org's ai.features_enabled grant,
// the overlap decision family (§B5b) and the overlap-policy settings card.
// Density steps compact + dense are first-class states here (§B9) — this is the
// surface where they earn their keep.
//
// Non-purpose: no auto-assign heuristics beyond the plan rows the user
// confirms; drag places onto the frozen demo board when the live API has no
// dispatch day; touch dragging is explicitly not implemented (§D).
"use client";

import "./dispatch.css";
import { Badge, ColumnChooser, DensitySwitch, ExportControl, type ChoosableColumn as ChooserColumn } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
} from "react";

import {
  CAP,
  D0,
  D1,
  NOW,
  REASON_LABEL,
  approvedWindows,
  collisionsOf,
  dur,
  evalDrop,
  hm,
  isAbsItem,
  pct,
  presenceOf,
  rowIndex,
  stackRows,
  statusOf,
  tm,
  travelMin,
  utilisation,
  type BoardAbsence,
  type BoardJob,
  type BoardTech,
  type DropEval,
  type OverlapReason,
  type Policy,
  type Pri,
  type QueueTicket,
} from "../../lib/dispatch-model";
import {
  FROZEN_CLOCK,
  FROZEN_DATE_LINE,
  ME_TECH,
  SHORT,
  TECHS,
  ZONES,
  seedAbsences,
  seedJobs,
  seedQueue,
  seedWithCollision,
} from "../../lib/dispatch-seed";
import { api, useRequireAuth } from "../../lib/api";
import { RoutingStrip } from "../../components/routing-strip";

type OnCall = components["schemas"]["OnCallBlock"];

const PRIORITY_FILTERS = ["all", "P1", "P2", "P3+"] as const;

/** Grid widths for the Unassigned table's columns, in the pack's order. */
const COLW: Record<string, string> = {
  pri: "60px",
  id: "84px",
  title: "minmax(280px,1fr)",
  client: "170px",
  est: "90px",
  need: "150px",
  where: "110px",
  sla: "170px",
};

type View = "day" | "week" | "queue";

interface DragState {
  kind: "new" | "move" | "resize";
  jid?: string;
  tk: number;
  title: string;
  dur: number;
  mode: "remote" | "site";
  client: string;
  clientZone: number;
  lane: number | null;
  start: number;
  ek?: string;
  sites?: boolean;
  gap?: number;
  allRemote?: boolean;
  hits?: BoardJob[];
  plan?: boolean;
  msg?: string;
  origin?: { tech: string; s: number };
  ekEval?: DropEval;
}

interface PlanRow {
  tk: number;
  title: string;
  client: string;
  pri: Pri;
  tech: string;
  s: number;
  d: number;
  mode: "remote" | "site";
  ov?: OverlapReason;
  part?: string;
  why: string;
  num: number;
}

interface PlanState {
  rows: PlanRow[];
  n: number;
  scopeLabel: string;
  failed: string | null;
}

function PriChip({ p }: { p: Pri }) {
  return <span className={`dp-pri dp-pri-${p.toLowerCase()}`}>{p}</span>;
}

// Wave41 lane 20: the dispatch board draws its named glyphs from the kit's
// one icon door (the mockups' own set) instead of local copies.
function AlertIcon({ size = 16 }: { size?: number }) {
  return <Icon name="alert" size={size} className="shrink-0" />;
}

function StackIcon({ size = 14 }: { size?: number }) {
  return <Icon name="layers" size={size} className="shrink-0" />;
}

function LocationIcon({ size = 13 }: { size?: number }) {
  return <Icon name="pin" size={size} className="shrink-0" />;
}

function MonitorIcon({ size = 13 }: { size?: number }) {
  return <Icon name="monitor" size={size} className="shrink-0" />;
}

export default function DispatchBoardPage() {
  const auth = useRequireAuth();
  const [density, setDensityState] = useState<"comfortable" | "compact" | "dense">("comfortable");
  const [ai, setAi] = useState(false);
  const [view, setView] = useState<View>("day");
  const [jobs, setJobs] = useState<BoardJob[]>(seedJobs);
  const [absences, setAbsences] = useState<BoardAbsence[]>(seedAbsences);
  const [queue, setQueue] = useState<QueueTicket[]>(seedQueue);
  const [qsel, setQsel] = useState<Set<number>>(new Set());
  const [qfilter, setQfilter] = useState<(typeof PRIORITY_FILTERS)[number]>("all");
  const [chips, setChips] = useState<Array<{ k: string; v: string }>>([]);
  // Unassigned column set: the picker's own order, and the header's order.
  const [cols, setCols] = useState<ChooserColumn[]>([
    { key: "pri", label: "P", visible: true, required: true },
    { key: "id", label: "ID", visible: true, required: true },
    { key: "title", label: "Ticket", visible: true },
    { key: "client", label: "Client", visible: true },
    { key: "est", label: "Est.", visible: true },
    { key: "need", label: "Needs", visible: true },
    { key: "where", label: "Where", visible: true },
    { key: "sla", label: "SLA", visible: true },
  ]);
  const [sel, setSel] = useState<string | null>(null);
  const [pop, setPop] = useState<string | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [kb, setKb] = useState<{ on: boolean; lane: number; t: number; picked: DragState | null }>({
    on: false,
    lane: 0,
    t: 706,
    picked: null,
  });
  const [plan, setPlan] = useState<PlanState | null>(null);
  const [scope, setScope] = useState<{ type: "selection" | "queue" | "filter" | "range"; range: "day" | "week" | "weeks" }>({
    type: "selection",
    range: "day",
  });
  const [flash, setFlash] = useState<number | null>(null);
  const [ovc, setOvc] = useState<{ g: DragState } | null>(null);
  const [settings, setSettings] = useState(false);
  const [policy, setPolicy] = useState<Policy>("ask");
  const [limit, setLimit] = useState(3);
  const [once, setOnce] = useState(true);
  const [toastMsg, setToastMsg] = useState<string | null>(null);
  const [live, setLive] = useState("");

  const techById = useMemo(() => new Map(TECHS.map((t) => [t.id, t])), []);
  const byTech = useCallback((id: string) => techById.get(id), [techById]);

  const toast = useCallback((m: string) => {
    setToastMsg(m);
    setLive(m);
    window.setTimeout(() => setToastMsg((cur) => (cur === m ? null : cur)), 2800);
  }, []);

  // `?demo=collision` swaps in the pack's state-07 day, where one job overruns
  // into the next. The banner is only meaningful when the day actually has a
  // double booking nobody chose, and the default day deliberately has none.
  useEffect(() => {
    if (new URLSearchParams(window.location.search).get("demo") === "collision") {
      setJobs(seedWithCollision());
    }
  }, []);

  useEffect(() => {
    document.documentElement.setAttribute("data-density", density);
    return () => {
      document.documentElement.removeAttribute("data-density");
    };
  }, [density]);

  // The AI plan editor is gated on the org's own `ai.features_enabled` setting:
  // no grant, no trace (§B7 — the button only exists where AI is switched on).
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/settings/org");
      if (cancelled) return;
      const row = (data ?? []).find((v) => v.key === "ai.features_enabled");
      setAi(row?.value === true);
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  // --- derived model -----------------------------------------------------
  const allItems = useMemo(
    () => [...jobs, ...planRowsAsJobs(plan), ...absences.map((a) => ({ ...a, abs: true as const, tk: 0 as const }))],
    [jobs, plan, absences],
  );

  const collisions = useMemo(() => collisionsOf(allItems, byTech), [allItems, byTech]);

  const filteredQueue = useMemo(
    () =>
      queue.filter((q) =>
        qfilter === "all" ? true : qfilter === "P3+" ? q.pri === "P3" || q.pri === "P4" : q.pri === qfilter,
      ),
    [queue, qfilter],
  );

  const lanes = useMemo(
    () =>
      TECHS.filter((t) =>
        chips.every((c) =>
          c.k === "Skill" ? t.sk.includes(c.v) : c.k === "Role" ? t.role === c.v : c.k === "Availability" ? freeNow(t, jobs, absences) : true,
        ),
      ),
    [chips, jobs, absences],
  );

  // --- drag machinery ----------------------------------------------------
  const laneElsRef = useRef<Array<HTMLDivElement | null>>([]);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const pdRef = useRef<{ kind: DragState["kind"]; tk?: number; jid?: string; x: number; y: number; started: boolean } | null>(null);
  const dragRef = useRef<DragState | null>(null);
  dragRef.current = drag;
  const stateRef = useRef({ jobs, absences, policy, limit, once, lanes, plan, ovc, sel, qsel, view, queue });
  stateRef.current = { jobs, absences, policy, limit, once, lanes, plan, ovc, sel, qsel, view, queue };

  const geo = useCallback((i: number) => {
    const el = laneElsRef.current[i];
    const laneVar = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--dp-lane")) || 88;
    const blkVar = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--dp-blk")) || 68;
    if (!el) return { t: 0, h: laneVar, b: blkVar };
    return { t: el.offsetTop - 36, h: el.offsetHeight, b: el.offsetHeight - (laneVar - blkVar) };
  }, []);

  const commit = useCallback(
    (g: DragState, reason?: OverlapReason) => {
      const st = stateRef.current;
      const tech = st.lanes[g.lane ?? 0]?.id;
      if (!tech) return;
      const ov = g.ek === "overlap" ? reason : undefined;
      if (g.kind === "new") {
        const q = st.queue.find((x) => x.tk === g.tk);
        if (!q) return;
        setJobs((js) => [
          ...js,
          {
            jid: `j${Date.now()}`,
            tk: q.tk,
            title: q.title,
            client: q.client,
            clientZone: q.clientZone,
            tech,
            s: g.start,
            d: g.dur,
            mode: q.mode,
            pri: q.pri,
            ov,
          },
        ]);
        setQueue((qs) => qs.filter((x) => x.tk !== q.tk));
        setQsel((s) => {
          const n = new Set(s);
          n.delete(q.tk);
          return n;
        });
        toast(ov ? `Placed #${q.tk} with ${SHORT[q.client] ?? q.client}, double-booked: ${REASON_LABEL[ov].toLowerCase()}` : `Placed #${q.tk} at ${hm(g.start)}`);
      } else if (g.jid && g.jid.startsWith("pr")) {
        const ri = Number(g.jid.slice(2));
        setPlan((p) => {
          if (!p || !p.rows[ri]) return p;
          return { ...p, rows: patchRow(p.rows, ri, g.kind === "resize" ? { d: g.dur } : { s: g.start, tech, ov: undefined }) };
        });
        toast("Plan updated");
      } else {
        setJobs((js) =>
          js.map((j) => {
            if (j.jid !== g.jid) return j;
            if (g.kind === "resize") return { ...j, d: g.dur, ov: undefined };
            return { ...j, tech, s: g.start, ov };
          }),
        );
        const who = st.lanes[g.lane ?? 0]?.name ?? "";
        toast(
          ov
            ? `Double-booked on purpose: ${REASON_LABEL[ov].toLowerCase()}`
            : g.kind === "resize"
              ? `#${g.tk} is now ${dur(g.dur)}`
              : `Moved #${g.tk} to ${who.split(" ")[0] ?? ""} at ${hm(g.start)}`,
        );
      }
    },
    [toast],
  );

  const reject = useCallback(
    (g: DragState) => {
      const lane = g.lane ?? 0;
      setFlash(lane);
      window.setTimeout(() => setFlash(null), 1400);
      toast(`Not placed. ${g.msg ?? ""}.`);
    },
    [toast],
  );

  const finishDrop = useCallback(
    (g: DragState) => {
      if (g.lane == null) return;
      if (g.ek === "blocked" && !g.plan) {
        reject(g);
        return;
      }
      if (g.ek === "overlap" && !g.plan && !g.ekEval?.sites) {
        setOvc({ g: { ...g } });
        return;
      }
      commit(g);
    },
    [commit, reject],
  );

  const mkGhost = useCallback(
    (o: Partial<DragState> & { tk: number; kind: DragState["kind"] }, lane: number, start: number): DragState => {
      const st = stateRef.current;
      const t = st.lanes[lane];
      const g: DragState = {
        kind: o.kind,
        jid: o.jid,
        tk: o.tk,
        title: o.title ?? "",
        dur: o.dur ?? 60,
        mode: o.mode ?? "remote",
        client: o.client ?? "",
        clientZone: o.clientZone ?? 1,
        lane,
        start,
      };
      if (!t) return { ...g, ek: "free" };
      const ev = evalDrop(
        { jid: g.jid ?? "x", tech: t.id, techName: t.name, s: start, d: g.dur, mode: g.mode, clientZone: g.clientZone, client: g.client },
        [
          ...st.jobs.filter((j) => j.tech === t.id),
          ...planRowsAsJobs(st.plan).filter((j) => j.tech === t.id),
        ],
        st.absences,
        st.policy,
        st.limit,
      );
      return {
        ...g,
        ek: ev.kind,
        sites: ev.sites,
        gap: ev.gap,
        hits: ev.hits,
        allRemote: ev.allRemote,
        plan: !!g.jid?.startsWith("pr"),
        msg: ev.msg,
        ekEval: ev,
      };
    },
    [],
  );

  const updateDragOff = useRef<number | null>(null);

  const updateDrag = useCallback(
    (x: number, y: number) => {
      const g = dragRef.current;
      if (!g) return;
      const s = slotAt(x, y, laneElsRef.current);
      const proxy = document.getElementById("dp-proxy");
      if (proxy) {
        proxy.style.left = `${x + 14}px`;
        proxy.style.top = `${y + 10}px`;
        proxy.style.display = s || g.kind !== "new" ? "none" : "block";
      }
      if (!s) {
        setDrag((cur) => (cur ? { ...cur, lane: null } : cur));
        return;
      }
      const ls = stateRef.current.lanes;
      let lane = s.lane;
      let start = g.start;
      let d = g.dur;
      if (g.kind === "resize") {
        const orig = g.origin;
        lane = orig ? ls.findIndex((t) => t.id === orig.tech) : lane;
        lane = lane < 0 ? 0 : lane;
        const origS = orig?.s ?? g.start;
        d = Math.max(15, Math.min(D1 - origS, snap5(s.min - origS)));
        start = origS;
      } else if (g.kind === "move") {
        start = snap5(s.min - (updateDragOff.current ?? 0));
      } else {
        start = snap5(s.min - g.dur / 2);
      }
      setDrag(mkGhost({ ...g, dur: d }, lane, start));
    },
    [mkGhost],
  );

  // pointer handlers are attached once
  useEffect(() => {
    if (auth.status !== "authed") return;
    function onDown(e: PointerEvent): void {
      if (e.button !== 0) return;
      const target = e.target as HTMLElement;
      const rz = target.closest<HTMLElement>("[data-rsz]");
      const blk = target.closest<HTMLElement>("[data-j]");
      const qc = target.closest<HTMLElement>("[data-q]");
      if (target.closest("input,select,button") && !rz) return;
      if (rz && blk) pdRef.current = { kind: "resize", jid: blk.dataset.j, x: e.clientX, y: e.clientY, started: false };
      else if (blk) pdRef.current = { kind: "move", jid: blk.dataset.j, x: e.clientX, y: e.clientY, started: false };
      else if (qc) pdRef.current = { kind: "new", tk: Number(qc.dataset.q), x: e.clientX, y: e.clientY, started: false };
    }
    function onMove(e: PointerEvent): void {
      const pd = pdRef.current;
      if (!pd) return;
      if (!pd.started) {
        if (Math.hypot(e.clientX - pd.x, e.clientY - pd.y) < 4) return;
        pd.started = true;
        const st = stateRef.current;
        if (pd.kind === "new") {
          const q = st.queue.find((x) => x.tk === pd.tk);
          if (!q) return;
          setDrag({ kind: "new", tk: q.tk, title: q.title, dur: q.est, mode: q.mode, client: q.client, clientZone: q.clientZone, lane: null, start: 0 });
          const p = document.createElement("div");
          p.id = "dp-proxy";
          p.className = "dp-proxy";
          p.textContent = `#${q.tk} ${q.title}`;
          document.body.appendChild(p);
        } else {
          const j = st.jobs.find((x) => x.jid === pd.jid) ?? planRowsAsJobs(st.plan).find((x) => x.jid === pd.jid);
          if (!j) return;
          const s = slotAt(pd.x, pd.y, laneElsRef.current);
          updateDragOff.current = s ? s.min - j.s : 0;
          setDrag({
            kind: pd.kind,
            jid: j.jid,
            tk: j.tk,
            title: j.title,
            dur: j.d,
            mode: j.mode,
            client: j.client,
            clientZone: j.clientZone,
            origin: { tech: j.tech, s: j.s },
            lane: null,
            start: j.s,
          });
          setSel(null);
        }
      }
      updateDrag(e.clientX, e.clientY);
    }
    function onUp(): void {
      const pd = pdRef.current;
      pdRef.current = null;
      document.getElementById("dp-proxy")?.remove();
      if (!pd) return;
      if (pd.started) {
        const g = dragRef.current;
        setDrag(null);
        if (g && g.lane != null) finishDrop(g);
      } else if (pd.kind !== "new" && pd.jid) {
        setSel((cur) => (cur === pd.jid ? null : (pd.jid ?? null)));
      }
    }
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
    };
  }, [auth.status, finishDrop, updateDrag]);

  // --- keyboard engine ---------------------------------------------------
  useEffect(() => {
    if (auth.status !== "authed") return;
    function onKey(e: KeyboardEvent): void {
      const st = stateRef.current;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const ae = document.activeElement as HTMLElement | null;
      if (st.ovc && /^[1-3]$/.test(e.key)) {
        const g = st.ovc.g;
        const rs = g.sites ? ["waiting", "handle"] : g.allRemote ? ["parallel", "waiting", "handle"] : ["waiting", "handle"];
        const r = rs[Number(e.key) - 1];
        if (r) {
          setOvc(null);
          commit(g, r as OverlapReason);
        }
        return;
      }
      if (e.key === "Escape" && (st.ovc || settings)) {
        setOvc(null);
        setSettings(false);
        return;
      }
      if (e.key === "Escape" && pop) {
        setPop(null);
        return;
      }
      if (ae && /INPUT|SELECT|TEXTAREA/.test(ae.tagName)) return;
      if (ae && ae.classList.contains("dp-qcard") && (e.key === "Enter" || e.key === " ")) {
        e.preventDefault();
        const tk = Number(ae.dataset.q ?? "0");
        const q = st.queue.find((x) => x.tk === tk);
        if (!q) return;
        const at = Math.ceil((NOW + 1) / 15) * 15;
        setKb({
          on: true,
          lane: 0,
          t: at,
          picked: { kind: "new", tk: q.tk, title: q.title, dur: q.est, mode: q.mode, client: q.client, clientZone: q.clientZone, lane: 0, start: at },
        });
        toast(`Picked up #${q.tk}. Use the arrow keys, then Enter to drop.`);
        scrollRef.current?.focus();
        return;
      }
      if (!(ae && (ae === scrollRef.current || ae.closest("[data-dscroll]")))) return;
      setKb((k) => {
        const ls = st.lanes;
        const n = ls.length;
        if (!n || !k.on) return { ...k, on: true };
        const step = e.shiftKey ? 60 : 15;
        const nc = { ...k, t: k.t, lane: k.lane };
        let used = true;
        const laneJobs = (id: string) => [...st.jobs, ...planRowsAsJobs(st.plan)].filter((j) => j.tech === id);
        const blockAt = () => {
          const t = ls[nc.lane];
          if (!t) return null;
          return laneJobs(t.id).find((j) => j.s <= nc.t && nc.t < j.s + j.d) ?? null;
        };
        switch (e.key) {
          case "ArrowRight":
            nc.t = Math.min(D1 - 15, nc.t + step);
            break;
          case "ArrowLeft":
            nc.t = Math.max(D0, nc.t - step);
            break;
          case "ArrowDown":
            nc.lane = Math.min(n - 1, nc.lane + 1);
            break;
          case "ArrowUp":
            nc.lane = Math.max(0, nc.lane - 1);
            break;
          case "Enter":
          case " ": {
            if (nc.picked) {
              const g = mkGhost(nc.picked, nc.lane, nc.t);
              commit(g);
              toast(`Dropped #${g.tk} at ${hm(g.start)}`);
              return { on: k.on, lane: k.lane, t: k.t, picked: null };
            }
            const b = blockAt();
            if (b && !(b as { prop?: boolean }).prop) {
              nc.picked = {
                kind: "move",
                jid: b.jid,
                tk: b.tk,
                title: b.title,
                dur: b.d,
                mode: b.mode,
                client: b.client,
                clientZone: b.clientZone,
                origin: { tech: b.tech, s: b.s },
                lane: nc.lane,
                start: b.s,
              };
              setLive(`Picked up #${b.tk}`);
            }
            break;
          }
          case "Escape":
            if (nc.picked) return { on: k.on, lane: k.lane, t: k.t, picked: null };
            return { ...nc, on: false };
          case "+":
          case "=":
          case "]":
            if (nc.picked) nc.picked = { ...nc.picked, dur: Math.min(240, nc.picked.dur + 15) };
            else {
              const b = blockAt();
              if (b) resizeJob(b, 15);
            }
            break;
          case "-":
          case "[":
            if (nc.picked) nc.picked = { ...nc.picked, dur: Math.max(15, nc.picked.dur - 15) };
            else {
              const b = blockAt();
              if (b) resizeJob(b, -15);
            }
            break;
          case "s":
          case "S": {
            const b = blockAt();
            if (b) splitJob(b.jid);
            break;
          }
          case "u":
          case "U": {
            const b = blockAt();
            if (b && !b.jid.startsWith("pr")) unassignJob(b.jid);
            break;
          }
          default:
            used = false;
        }
        if (used) setLive(`${ls[nc.lane]?.name ?? ""} ${hm(nc.t)}`);
        return nc;
      });
      e.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [auth.status, commit, mkGhost, pop, settings, toast]);

  function resizeJob(b: BoardJob, delta: number): void {
    const nd = Math.max(15, b.d + delta);
    if (b.jid.startsWith("pr")) {
      setPlan((p) => {
        if (!p || !p.rows[b.ri ?? 0]) return p;
        return { ...p, rows: patchRow(p.rows, b.ri ?? 0, { d: nd }) };
      });
      return;
    }
    const t = TECHS.find((x) => x.id === b.tech);
    const ev = evalDrop(
      { jid: b.jid, tech: b.tech, techName: t?.name ?? "", s: b.s, d: nd, mode: b.mode, clientZone: b.clientZone, client: b.client },
      jobs.filter((j) => j.tech === b.tech && j.jid !== b.jid),
      absences,
      policy,
      limit,
    );
    if (!ev.ok) {
      toast(`Not resized. ${ev.msg}.`);
      return;
    }
    if (ev.kind === "overlap") {
      toast(`That would overlap #${ev.hits[0]?.tk ?? ""}. Drag the edge to decide.`);
      return;
    }
    setJobs((js) => js.map((j) => (j.jid === b.jid ? { ...j, d: nd, ov: undefined } : j)));
    toast(`#${b.tk} is now ${dur(nd)}`);
  }

  function splitJob(jid: string): void {
    const j = jobs.find((x) => x.jid === jid) ?? planRowsAsJobs(plan).find((x) => x.jid === jid);
    if (!j) return;
    if (j.d < 30) {
      toast("Too short to split");
      return;
    }
    const half = Math.max(15, snap5(j.d / 2));
    if (j.jid.startsWith("pr")) {
      setPlan((p) => {
        if (!p) return p;
        const i0 = j.ri ?? 0;
        const r = p.rows[i0];
        if (!r) return p;
        const r2: PlanRow = { ...r, s: r.s + half, d: r.d - half, part: "2/2" };
        const rows = [...p.rows];
        rows[i0] = { ...r, d: half, part: "1/2" };
        rows.splice(i0 + 1, 0, r2);
        rows.forEach((rr, k) => (rr.num = k + 1));
        return { ...p, rows };
      });
      setSel(jid);
      toast("Split into two visits. Drag either part where it needs to go.");
      return;
    }
    setJobs((js) => {
      const jj = js.find((x) => x.jid === jid);
      if (!jj) return js;
      const b: BoardJob = { ...jj, jid: `j${Date.now()}`, s: jj.s + half, d: jj.d - half, part: "2/2" };
      return [...js.map((x) => (x.jid === jid ? { ...x, d: half, part: "1/2" } : x)), b];
    });
    setSel(null);
    toast("Split into two visits. Drag either part where it needs to go.");
  }

  function unassignJob(jid: string): void {
    const j = jobs.find((x) => x.jid === jid);
    if (!j) return;
    setJobs((js) => js.filter((x) => x.jid !== jid));
    setQueue((qs) =>
      qs.some((q) => q.tk === j.tk)
        ? qs
        : [...qs, { tk: j.tk, title: j.title, client: j.client, clientZone: j.clientZone, pri: j.pri, est: j.d, mode: j.mode, need: "Service desk", sla: null }],
    );
    setSel(null);
    toast(`#${j.tk} is back in the queue`);
  }

  function autoSlot(techId: string, d: number): number | null {
    const laneJobs = jobs.filter((j) => j.tech === techId);
    const t = TECHS.find((x) => x.id === techId);
    if (!t) return null;
    for (let s = Math.max(Math.ceil((NOW + 1) / 15) * 15, D0); s + d <= 1050; s += 15) {
      const ev = evalDrop({ jid: "x", tech: techId, techName: t.name, s, d, mode: "remote", clientZone: 1, client: "" }, laneJobs, absences, policy, limit);
      if (ev.kind === "free") return s;
    }
    return null;
  }

  if (auth.status !== "authed") {
    return (
      <div className="flex min-h-full flex-col">
        <p className="p-6 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          Signing in…
        </p>
      </div>
    );
  }

  // plan issue calculation for the panel
  const planIssues: Array<DropEval | null> = plan
    ? plan.rows.map((r, i) => {
        const t = TECHS.find((x) => x.id === r.tech);
        if (!t) return null;
        const ev = evalDrop(
          { jid: `pr${i}`, tech: r.tech, techName: t.name, s: r.s, d: r.d, mode: r.mode, clientZone: ZONES[r.client] ?? 1, client: r.client },
          [...jobs.filter((j) => j.tech === r.tech), ...planRowsAsJobs(plan).filter((j, k) => k !== i && j.tech === r.tech)],
          absences,
          policy,
          limit,
        );
        if (ev.kind === "free") return null;
        if (ev.ok && ev.kind === "overlap" && (r.ov || ev.hits.every((h) => h.ov))) return null;
        return ev;
      })
    : [];

  const nBlocked = planIssues.filter((e) => e?.kind === "blocked").length;
  const nOverlap = planIssues.filter((e) => e?.kind === "overlap").length;
  const nOv = plan?.rows.filter((r) => r.ov).length ?? 0;

  const scopeList = (): QueueTicket[] => {
    if (scope.type === "selection") return queue.filter((q) => qsel.has(q.tk));
    if (scope.type === "queue") return queue;
    if (scope.type === "filter") return filteredQueue;
    return scope.range === "day" ? queue.filter((q) => q.sla !== null && q.sla <= 1440) : queue;
  };

  function propose(): void {
    const l = scopeList();
    if (!l.length) return;
    const L =
      scope.type === "selection"
        ? "From your selection"
        : scope.type === "queue"
          ? "From the unassigned queue"
          : scope.type === "filter"
            ? "From the current filter"
            : `Due ${scope.range === "day" ? "today" : scope.range === "week" ? "this week" : "in the next two weeks"}`;
    setPlan({ rows: makePlan(l, jobs, absences), n: l.length, scopeLabel: L, failed: null });
    setPop(null);
    setSel(null);
    setView("day");
  }

  function confirmPlan(): void {
    const p = plan;
    if (!p) return;
    setJobs((js) => [
      ...js,
      ...p.rows.map((r, i) => ({
        jid: `j${Date.now()}-${i}`,
        tk: r.tk,
        title: r.title,
        client: r.client,
        clientZone: ZONES[r.client] ?? 1,
        tech: r.tech,
        s: r.s,
        d: r.d,
        mode: r.mode,
        pri: r.pri,
        part: r.part,
        ov: r.ov,
      })),
    ]);
    setQueue((qs) => qs.filter((q) => !p.rows.some((r) => r.tk === q.tk)));
    setQsel((s) => {
      const n = new Set(s);
      p.rows.forEach((r) => n.delete(r.tk));
      return n;
    });
    const n = p.rows.length;
    setPlan(null);
    toast(`Plan confirmed. ${n} placement${n > 1 ? "s" : ""} booked. Client confirmations are ready for you to send.`);
  }

  // ---- render pieces ----------------------------------------------------
  const renderHeader = (
    <div className="dp-head">
      <h1>Dispatch</h1>
      <nav className="dp-subnav" aria-label="Dispatch sections">
        <Link href="/dispatch" aria-current="page">
          Board
        </Link>
        <Link href="/dispatch/planning">Planning</Link>
        <Link href="/dispatch/travel">Travel</Link>
        <Link href="/dispatch/scheduling">Scheduling</Link>
      </nav>
      <span className="dp-sp" />
      <div className="dp-datenav">
        <button type="button" className="dp-ib" aria-label="Previous day" onClick={() => toast("This demo loads today")}>
          ←
        </button>
        <span className="lbl">{view === "week" ? "Mon 28 Sep to Fri 2 Oct" : FROZEN_DATE_LINE}</span>
        <button type="button" className="dp-ib" aria-label="Next day" onClick={() => toast("This demo loads today")}>
          →
        </button>
        <button type="button" className="dp-btn" style={{ height: 36 }} onClick={() => toast("This demo loads today")}>
          Today
        </button>
      </div>
      <button type="button" className="dp-btn primary" onClick={() => toast("New job: pick a ticket, a technician and a time")}>
        New job
      </button>
    </div>
  );

  // The Unassigned view already carries Plan with AI in its selection bar, so
  // the toolbar button would be a second identical call to action in one view.
  const showPlanWithAI = ai && view === "day";

  const renderToolbar = (
    <div className="dp-tb">
      <div className="dp-seg dp-views" role="group" aria-label="Views">
        <button type="button" aria-pressed={view === "day"} onClick={() => setView("day")}>
          Day
        </button>
        <button type="button" aria-pressed={view === "week"} onClick={() => setView("week")}>
          Week
        </button>
        <button type="button" aria-pressed={view === "queue"} onClick={() => setView("queue")}>
          Unassigned<span className="cnt"> {queue.length}</span>
        </button>
      </div>
      <div className="dp-chips">
        {chips.map((c, i) => (
          <span key={`${c.k}-${c.v}`} className="dp-chip">
            <span className="k">{c.k} is</span>
            <span className="v">{c.v}</span>
            <button type="button" aria-label={`Remove filter ${c.k} ${c.v}`} onClick={() => setChips((cs) => cs.filter((_, k) => k !== i))}>
              ×
            </button>
          </span>
        ))}
        <button type="button" className="dp-chip add" onClick={() => setPop(pop === "filter" ? null : "filter")}>
          + Add filter
        </button>
      </div>
      {view === "queue" ? (
        <>
          <span className="dp-sp" style={{ flex: 1 }} />
          {/* The pack's right group reads Columns, Filter, Export
              (`DESIGN-NOTES-tickets.md` §2 law 1). The chooser therefore sits
              before the Filter button rather than after it. */}
          <ColumnChooser
            columns={cols}
            onChange={(next) => {
              setCols(next);
              const shown = next.filter((c) => c.visible).length;
              toast(`Columns: ${shown} of ${next.length} shown`);
            }}
          />
        </>
      ) : null}
      <button type="button" className="dp-btn" aria-expanded={pop === "filter"} onClick={() => setPop(pop === "filter" ? null : "filter")}>
        Filter{chips.length ? <span className="dp-kbd" style={{ border: "none" }}>{chips.length}</span> : null}
      </button>
      {showPlanWithAI && (
        <div className="dp-anchor">
          <button type="button" className={plan ? "dp-btn" : "dp-btn primary"} aria-expanded={pop === "scope"} onClick={() => setPop(pop === "scope" ? null : "scope")}>
            Plan with AI
          </button>
        </div>
      )}
      {/* The scope chooser hangs off the toolbar in either view that offers
          Plan with AI, so the queue selection bar and the day toolbar reach
          the same dialog rather than one of them opening nothing. */}
      {ai && pop === "scope" && (
        <div className="dp-anchor" style={{ marginLeft: "auto" }}>
          {renderScope()}
        </div>
      )}
      {view === "queue" ? (
        <ExportControl
          columns={cols.filter((c) => c.visible).map((c) => c.key)}
          rowCount={filteredQueue.length}
          filters={qfilter === "all" ? undefined : { priority: qfilter }}
          onExport={(req) => toast(`Exporting ${req.rowCount} unassigned ticket${req.rowCount === 1 ? "" : "s"} as ${req.format.toUpperCase()}`)}
        />
      ) : null}
      <button type="button" className="dp-ib" aria-label="Dispatch settings" onClick={() => setSettings(true)}>
        <Icon name="settings" size={18} />
      </button>
    </div>
  );

  function renderScope(): React.JSX.Element {
    const l = scopeList();
    const hrs = l.reduce((a, q) => a + q.est, 0);
    return (
      <div className="dp-scope" role="dialog" aria-label="Plan with AI: choose the scope">
        <div className="hd">
          <div>
            <h2>Plan with AI</h2>
            <div className="sub">Pick what to place. You review every placement.</div>
          </div>
        </div>
        <div className="opts">
          <label className="o" data-on={scope.type === "selection"}>
            <input type="radio" name="scope" checked={scope.type === "selection"} onChange={() => setScope({ ...scope, type: "selection" })} />
            <span>
              <b>The selected tickets ({qsel.size})</b>
              <small>What you ticked in the queue</small>
            </span>
          </label>
          <label className="o" data-on={scope.type === "queue"}>
            <input type="radio" name="scope" checked={scope.type === "queue"} onChange={() => setScope({ ...scope, type: "queue" })} />
            <span>
              <b>Everything in the unassigned queue ({queue.length})</b>
              <small>All of today&apos;s open requests</small>
            </span>
          </label>
          <label className="o" data-on={scope.type === "filter"}>
            <input type="radio" name="scope" checked={scope.type === "filter"} onChange={() => setScope({ ...scope, type: "filter" })} />
            <span>
              <b>Everything matching the current filter ({filteredQueue.length})</b>
              <small>Priority {qfilter === "all" ? "any" : qfilter}</small>
            </span>
          </label>
          <label className="o" data-on={scope.type === "range"}>
            <input type="radio" name="scope" checked={scope.type === "range"} onChange={() => setScope({ ...scope, type: "range" })} />
            <span style={{ flex: 1 }}>
              <b>A range of time</b>
              <small>Everything due inside it</small>
              <span className="dp-seg" role="group" aria-label="Range" style={{ marginTop: 8, display: "inline-flex" }}>
                {([["day", "The day"], ["week", "The week"], ["weeks", "Next 2 weeks"]] as const).map(([k, t]) => (
                  <button key={k} type="button" aria-pressed={scope.range === k} style={{ height: 28 }} onClick={() => setScope({ type: "range", range: k })}>
                    {t}
                  </button>
                ))}
              </span>
            </span>
          </label>
        </div>
        <div className="prev">
          <b>{l.length} ticket{l.length === 1 ? "" : "s"}</b>, about {dur(hrs)} of work.
          {l.length ? ` ${l.slice(0, 3).map((q) => `#${q.tk}`).join(", ")}${l.length > 3 ? ` and ${l.length - 3} more` : ""}.` : " Nothing in this scope yet."}
          <br />
          <span style={{ color: "var(--m-text-muted)" }}>They stay in the queue until you confirm the plan.</span>
        </div>
        <div className="ft">
          <span className="match">Nothing is booked or sent yet.</span>
          <button type="button" className="dp-btn" onClick={() => setPop(null)}>
            Cancel
          </button>
          <button type="button" className="dp-btn primary" disabled={!l.length} onClick={propose}>
            Propose a plan
          </button>
        </div>
      </div>
    );
  }

  const first = collisions[0];
  const renderBanner =
    view === "day" && first ? (
      <div className="dp-banner" role="status" data-testid="collision-banner">
        <AlertIcon size={18} />
        <span>
          <b>
            {collisions.length} collision{collisions.length > 1 ? "s" : ""} today.
          </b>{" "}
          {collisions.slice(0, 2).map((x) => collisionSay(x)).join(". ")}.
        </span>
        <span className="dp-sp" />
        <button type="button" className="dp-btn" onClick={() => setSel(first.a.jid || first.b.jid)}>
          Review first
        </button>
        {!isAbsItem(first.a) && !isAbsItem(first.b) && (
          <button
            type="button"
            className="dp-btn"
            onClick={() => {
              const x = collisions.find((y) => !isAbsItem(y.a) && !isAbsItem(y.b));
              if (!x) return;
              const later = x.b.s >= x.a.s ? x.b : x.a;
              setJobs((js) => js.map((j) => (j.jid === later.jid ? { ...j, ov: "handle" } : j)));
              toast("Kept. It stays on the board as a double booking you chose.");
            }}
          >
            Keep it
          </button>
        )}
      </div>
    ) : null;

  // ---- day board --------------------------------------------------------
  const renderBoard = () => {
    const showQueue = !plan;
    return (
      <div className="dp-board" data-testid="dispatch-board-root">
        {showQueue && (
          <aside className="dp-q" aria-label="Unassigned queue">
            <div className="dp-qh">
              <div className="dp-qt">
                Unassigned<span className="cnt"> {queue.length}</span>
                <span style={{ flex: 1 }} />
                <button
                  type="button"
                  className="dp-linkbtn"
                  style={{ fontSize: 13 }}
                  onClick={() => setQsel((s) => (s.size ? new Set() : new Set(filteredQueue.map((q) => q.tk))))}
                >
                  {qsel.size ? "Clear" : "Select all"}
                </button>
              </div>
              <div className="dp-qf" role="group" aria-label="Priority">
                {PRIORITY_FILTERS.map((k) => (
                  <button key={k} type="button" aria-pressed={qfilter === k} onClick={() => setQfilter(k)}>
                    {k === "all" ? "All" : k}
                  </button>
                ))}
              </div>
            </div>
            <div className="dp-qlist">
              {filteredQueue.map((q) => renderQCard(q))}
              {filteredQueue.length === 0 && (
                <div className="dp-hint2" style={{ padding: 12 }}>
                  Nothing unassigned. The board is fully placed.
                </div>
              )}
            </div>
            <div className="dp-qfoot">Drag onto a lane, or press Enter on a card and place it with the arrow keys.</div>
          </aside>
        )}
        <div
          className="dp-scroll"
          data-dscroll
          tabIndex={0}
          role="application"
          aria-label="Dispatch board. Arrow keys move the cursor, Enter picks up or drops, S splits, Escape cancels."
          ref={scrollRef}
        >
          <div className="dp-inner">
            <div className="dp-ruler">
              <div className="rl">Technician</div>
              <div className="rt">
                {Array.from({ length: 11 }, (_, k) => (
                  <span key={k} className="hr" style={{ left: `${pct(D0 + k * 60)}%` }}>
                    {hm(D0 + k * 60)}
                  </span>
                ))}
                <span className="now" style={{ left: `${pct(NOW)}%` }}>
                  {FROZEN_CLOCK}
                </span>
              </div>
            </div>
            {lanes.map((t, i) => renderLane(t, i))}
            <div className="dp-nowline" style={{ left: `calc(176px + (100% - 176px) * ${pct(NOW) / 100})` }} />
            <div className="dp-ov">{renderOverlayContent()}</div>
          </div>
        </div>
        {plan && renderPlanPanel()}
      </div>
    );
  };

  function renderQCard(q: QueueTicket): React.JSX.Element {
    const isSel = qsel.has(q.tk);
    const dragging = drag?.tk === q.tk && drag.kind === "new";
    return (
      <div
        key={q.tk}
        className={`dp-qcard${isSel ? " sel" : ""}${dragging ? " dragging" : ""}`}
        data-q={q.tk}
        tabIndex={0}
        role="button"
        aria-label={`#${q.tk} ${q.title}, unassigned, ${q.pri}, ${dur(q.est)}. Enter to pick up.`}
      >
        <input
          type="checkbox"
          checked={isSel}
          aria-label={`Select #${q.tk}`}
          onChange={() =>
            setQsel((s) => {
              const n = new Set(s);
              if (n.has(q.tk)) n.delete(q.tk);
              else n.add(q.tk);
              return n;
            })
          }
        />
        <div className="qb">
          <div className="q1">
            <PriChip p={q.pri} />
            <span className="dp-idc">#{q.tk}</span>
            {q.sla !== null && q.sla <= 60 ? (
              <span className="dp-pill dp-pill-bad" style={{ marginLeft: "auto" }}>
                <Icon name="clock" size={14} />
                {dur(q.sla)} left
              </span>
            ) : q.sla !== null ? (
              <span className="dp-idc" style={{ marginLeft: "auto", fontSize: 13 }}>
                {dur(q.sla)}
              </span>
            ) : null}
          </div>
          <div className="q2">{q.title}</div>
          {density === "comfortable" && (
            <div className="q3">
              {SHORT[q.client] ?? q.client}
              <span>·</span>
              {dur(q.est)}
              <span>·</span>
              {q.mode === "site" ? <LocationIcon /> : <MonitorIcon />}
              {q.mode === "site" ? "On site" : "Remote"}
              <span>·</span>
              {q.need}
            </div>
          )}
        </div>
        <span className="grip" aria-hidden="true">
          <Icon name="grip" size={16} />
        </span>
      </div>
    );
  }

  /** The drop outline belongs to the lane row, so it is part of the row's
   *  className rather than a classList toggle from render — a render-phase DOM
   *  write is a side effect React is free to discard, and the outline would
   *  then survive a drag that has already moved on. */
  function dropTone(i: number): "" | "drop-ok" | "drop-bad" {
    const g = drag;
    if (!g || g.lane !== i) return "";
    const bad = g.ek === "blocked" && !g.plan;
    return bad || (g.ek === "overlap" && !!g.sites) ? "drop-bad" : "drop-ok";
  }

  function renderLane(t: BoardTech, i: number): React.JSX.Element {
    const laneJobs = [...jobs, ...planRowsAsJobs(plan)].filter((j) => j.tech === t.id).sort((a, b) => a.s - b.s);
    const u = utilisation(laneJobs, !once);
    const p = presenceOf(t, jobs, absences);
    const over = u.pctv > 100;
    const nr = stackRows(laneJobs);
    const laneHeight = `calc(var(--dp-lane) + var(--dp-xr) * ${nr - 1})`;
    const blkHeight = `calc(var(--dp-blk) + var(--dp-xr) * ${nr - 1})`;
    const absencesHere = absences.filter((a) => a.tech === t.id);
    const collsHere = collisions.filter((x) => x.tech === t.id);
    const approved = approvedWindows(laneJobs);
    return (
      <div
        key={t.id}
        className={`dp-lane${flash === i ? " flash" : ""}${dropTone(i) ? ` ${dropTone(i)}` : ""}`}
        data-tech={t.id}
        style={{ height: laneHeight }}
        ref={(el) => {
          laneElsRef.current[i] = el;
        }}
      >
        <div className="dp-lh">
          <div className="l1">
            <span className={`av${t.id === ME_TECH ? " me" : ""}`} title={t.id === ME_TECH ? `${t.name} (you)` : undefined}>
              {t.id}
            </span>
            <span className="nm">{t.name}</span>
            {nr > 1 && (
              <span className="x2" title="Double-booked on purpose">
                ×{nr}
              </span>
            )}
          </div>
          <div className="l2">
            <span className="pd" style={{ background: p.dot }} />
            {p.txt}
          </div>
          <div className="l3">
            <span className="dp-ub" aria-hidden="true">
              {over ? (
                <>
                  <i style={{ width: `${(100 / u.pctv) * 100}%`, background: "var(--m-bark)" }} />
                  <i style={{ width: `${(1 - 100 / u.pctv) * 100}%`, background: "var(--m-status-due-fg)" }} />
                </>
              ) : (
                <i style={{ width: `${u.pctv}%`, background: "var(--m-bark)" }} />
              )}
            </span>
            <span className={`dp-pc${over ? " over" : ""}`}>
              {over && <AlertIcon size={13} />}
              {u.pctv}%
            </span>
          </div>
          {over && <div className="ovr">Over by {dur(u.min - CAP)}</div>}
        </div>
        <div className="dp-lt">
          <span className="off" style={{ left: 0, width: `${pct(480)}%` }} />
          <span className="off" style={{ left: `${pct(1020)}%`, right: 0 }} />
          {Array.from({ length: 10 }, (_, k) => (
            <span key={k} className="hl" style={{ left: `${pct(480 + k * 60)}%` }} />
          ))}
          <span className="past" style={{ width: `${pct(NOW)}%` }} />
          {laneJobs.map((j) => renderBlock(j, nr, blkHeight))}
          {absencesHere.map((a) => (
            <div key={a.jid} className="dp-blk abs" style={{ left: `${pct(a.s)}%`, width: `${(a.d / (D1 - D0)) * 100}%` }}>
              <div className="dp-bt">
                <Icon name="lock" size={14} />
                {a.title}
              </div>
              <div className="dp-mt">
                {hm(a.s)} to {hm(a.s + a.d)}
              </div>
            </div>
          ))}
          {collsHere.map((x, k) => (
            <div key={k} className="dp-collide" style={{ left: `${pct(x.s)}%`, width: `${((x.e - x.s) / (D1 - D0)) * 100}%` }} />
          ))}
          {approved.map((w, k) => (
            <span key={k} className="dp-stkb" style={{ left: `${pct(w.s)}%` }}>
              {nr} at once
            </span>
          ))}
        </div>
      </div>
    );
  }

  function renderBlock(j: BoardJob, nr: number, blkHeight: string): React.JSX.Element {
    const st = j.prop ? "prop" : statusOf(j);
    const cf = collisions.some((x) => x.a.jid === j.jid || x.b.jid === j.jid);
    const gap = 4;
    const r = rowIndex.get(j) ?? 0;
    // A single block is centred in its lane, the way the pack draws it. It used
    // to sit at the top of the lane (an absolutely positioned box with no `top`
    // takes its static position), which left the block hugging the ruler and
    // the lane's own vertical rhythm broken. Stacked blocks carry their own row
    // maths and must not also be translated.
    const pos: React.CSSProperties =
      nr > 1
        ? {
            top: `calc((var(--dp-lane) - var(--dp-blk)) / 2 + ${r} * ((var(--dp-blk) - ${gap * (nr - 1)}px) / ${nr} + ${gap}px))`,
            height: `calc((var(--dp-blk) - ${gap * (nr - 1)}px) / ${nr})`,
          }
        : { top: "50%", transform: "translateY(-50%)", height: blkHeight };
    const travel = travelMin(j, jobs.filter((x) => x.tech === j.tech).sort((a, b) => a.s - b.s));
    return (
      <div key={j.jid}>
        {travel > 0 && (
          <div className="dp-trv" style={{ left: `${pct(j.s - travel)}%`, width: `${(travel / (D1 - D0)) * 100}%` }} title={`Travel ${travel} min`}>
            <Icon name="car" size={12} />
            <span>{travel}m</span>
          </div>
        )}
        <div
          className={`dp-blk p${j.pri.slice(1).toLowerCase()} ${st}${nr > 1 ? " stk" : ""}${sel === j.jid ? " sel" : ""}${cf ? " conf" : ""}`}
          data-j={j.jid}
          style={{ left: `${pct(j.s)}%`, width: `${(j.d / (D1 - D0)) * 100}%`, ...pos }}
          role="button"
          tabIndex={-1}
          aria-label={`#${j.tk} ${j.title}, ${SHORT[j.client] ?? j.client}, ${hm(j.s)} to ${hm(j.s + j.d)}${cf ? ", conflict" : ""}${j.ov ? `. Double-booked on purpose: ${REASON_LABEL[j.ov].toLowerCase()}` : ""}`}
        >
          <span className="dp-bar" />
          <div className="dp-bt">
            {j.prop && <span className="dp-num">{j.num}</span>}
            <span className="pid">#{j.tk}</span>
            <span className="dp-cl">{SHORT[j.client] ?? j.client}</span>
            {cf && (
              <span className="dp-warn">
                <AlertIcon size={14} />
              </span>
            )}
            <span className="dp-ti">{j.title}</span>
          </div>
          <div className="dp-tt">{j.title}</div>
          <div className="dp-mt">
            {j.mode === "site" ? <LocationIcon /> : <MonitorIcon />}
            {j.prop ? "Proposed" : j.mode === "site" ? "On site" : "Remote"} · {dur(j.d)}
            {j.part ? ` · part ${j.part}` : ""}
            {st === "done" ? " · done" : ""}
          </div>
          <span className="dp-rsz" data-rsz="1" />
        </div>
      </div>
    );
  }

  function renderOverlayContent(): React.JSX.Element {
    const parts: React.JSX.Element[] = [];
    const g = drag;
    if (g && g.lane != null && lanes[g.lane]) {
      const q = geo(g.lane);
      const isOv = g.ek === "overlap";
      const bad = g.ek === "blocked" && !g.plan;
      const cls = bad ? "bad" : isOv ? `ov${g.sites ? " sites" : ""}` : "ok";
      const chCls = bad ? "bad" : isOv ? (g.sites ? "ovs" : "ovn") : "ok";
      const gh = isOv ? Math.floor((q.b - 4) / 2) : q.b;
      const gt = q.t + (q.h - q.b) / 2 + (isOv ? q.b - gh : 0);
      const label = bad ? g.msg : isOv ? `${g.msg}. Double-book?` : `${hm(g.start)} to ${hm(g.start + g.dur)} · ${dur(g.dur)}`;
      parts.push(
        <div key="ghost" className={`dp-ghost ${cls}`} style={{ left: `${pct(g.start)}%`, width: `${(g.dur / (D1 - D0)) * 100}%`, top: gt, height: gh }}>
          {isOv ? `#${g.tk}` : `#${g.tk} ${g.title}`}
        </div>,
      );
      const cl = Math.max(2, Math.min(pct(g.start), 56));
      parts.push(
        <div key="chip" className={`dp-gchip ${chCls}`} style={{ left: `${cl}%`, top: g.lane === 0 ? q.t + q.h - 6 : q.t - 2 }}>
          {bad || g.sites ? <AlertIcon size={14} /> : isOv ? <StackIcon /> : null}
          {label}
        </div>,
      );
    }
    if (kb.on && kb.picked) {
      const q = geo(kb.lane);
      parts.push(
        <div key="kcur" className="dp-kcur" style={{ left: `${pct(kb.t)}%`, width: `${(15 / (D1 - D0)) * 100}%`, top: q.t, height: q.h }} />,
      );
    }
    if (sel) {
      const jb = [...jobs, ...planRowsAsJobs(plan)].find((x) => x.jid === sel);
      const li = jb ? lanes.findIndex((t) => t.id === jb.tech) : -1;
      if (jb && li >= 0) {
        const q = geo(li);
        parts.push(
          <div
            key="btb"
            className="dp-btb"
            style={{
              left: `${Math.min(pct(jb.s), 60)}%`,
              top: li === 0 ? q.t + q.h - 4 : Math.max(-4, q.t - 6),
              ...(li === 0 ? {} : { transform: "translateY(-100%)" }),
            }}
          >
            {jb.ov && (
              <span className="dp-pill dp-pill-neu" style={{ alignSelf: "center", margin: "0 4px" }}>
                <StackIcon />
                {REASON_LABEL[jb.ov]}
              </span>
            )}
            <button type="button" onClick={() => splitJob(sel)}>
              <Icon name="split" size={16} />
              Split
            </button>
            {!jb.prop && (
              <button type="button" onClick={() => unassignJob(sel)}>
                <Icon name="userx" size={16} />
                Unassign
              </button>
            )}
            <button type="button" onClick={() => toast(`Opening #${jb.tk} in the workbench`)}>
              Open
            </button>
          </div>,
        );
      }
    }
    if (ovc) {
      const g2 = ovc.g;
      const lane = g2.lane ?? 0;
      const q = geo(lane);
      const tech = lanes[lane];
      if (tech) {
        const below = lane < 5;
        const f = g2.hits?.[0];
        const rs: OverlapReason[] = g2.sites ? ["waiting", "handle"] : g2.allRemote ? ["parallel", "waiting", "handle"] : ["waiting", "handle"];
        parts.push(
          <div
            key="ovc"
            className="dp-ovc"
            role="dialog"
            aria-label={`Double-book ${tech.name}?`}
            data-testid="overlap-card"
            style={{ left: `${Math.max(0, Math.min(pct(g2.start) - 4, 58))}%`, top: below ? q.t + q.h + 10 : Math.max(0, q.t - 262) }}
          >
            <b style={{ fontSize: 15 }}>Double-book {tech.name} at {hm(g2.start)}?</b>
            <div className="dp-why">
              #{g2.tk}{" "}
              {g2.sites
                ? `overlaps #${f?.tk ?? ""} at a different site, ${g2.gap} min apart by road.`
                : `overlaps #${f?.tk ?? ""} (${f ? `${hm(f.s)} to ${hm(f.s + f.d)}` : ""}).`}{" "}
              Both stay on the board, stacked. Nothing is sent to the client.
            </div>
            <div className="dp-rs2">
              {rs.map((r, i2) => (
                <button key={r} type="button" onClick={() => { setOvc(null); commit(g2, r); }}>
                  <span className="dp-kbd">{i2 + 1}</span>
                  {REASON_LABEL[r]}
                </button>
              ))}
            </div>
            <div className="dp-links">
              <button type="button" className="dp-linkbtn" style={{ color: "var(--m-text-muted)", fontWeight: 500 }} onClick={() => { setOvc(null); setSettings(true); }}>
                Change the overlap policy
              </button>
              <span style={{ flex: 1 }} />
              <button type="button" className="dp-btn" style={{ height: 32, fontSize: 13 }} onClick={() => setOvc(null)}>
                Cancel <span className="dp-kbd" style={{ color: "inherit", borderColor: "currentColor", opacity: 0.8 }}>Esc</span>
              </button>
            </div>
          </div>,
        );
      }
    }
    if (kb.on) {
      parts.push(
        <div key="kbh" className="dp-kbhelp" data-testid="kb-help">
          <span className="dp-kbd">←</span>
          <span className="dp-kbd">→</span> time <span className="dp-kbd">↑</span>
          <span className="dp-kbd">↓</span> lane <span className="dp-kbd">+</span>
          <span className="dp-kbd">-</span> length <span className="dp-kbd">↵</span> drop <span className="dp-kbd">Esc</span> cancel
        </div>,
      );
    }
    return <div style={{ position: "absolute", inset: 0 }}>{parts}</div>;
  }

  function renderPlanPanel(): React.JSX.Element {
    if (!plan) return <></>;
    const planJobsForTravel = planRowsAsJobs(plan);
    const trv = plan.rows.reduce((a, r, i) => {
      const pj = planJobsForTravel[i];
      if (!pj) return a;
      return a + travelMin(pj, jobs.filter((x) => x.tech === r.tech).sort((x, y) => x.s - y.s));
    }, 0);
    const pillS = nBlocked ? (
      <span className="dp-pill dp-pill-bad">
        <AlertIcon size={14} />
        {nBlocked} conflict{nBlocked > 1 ? "s" : ""}
      </span>
    ) : nOverlap ? (
      <span className="dp-pill dp-pill-bad">
        <StackIcon />
        {nOverlap} to decide
      </span>
    ) : (
      <span className="dp-pill dp-pill-ok">No conflicts</span>
    );
    return (
      <aside className="dp-plan" role="dialog" aria-label="Review the proposed plan" data-testid="plan-panel">
        <div className="ph2">
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <h2>
              Plan for {plan.n} ticket{plan.n === 1 ? "" : "s"}
            </h2>
            <button type="button" className="dp-ib-sm" aria-label="Discard plan" onClick={() => { setPlan(null); toast("Plan discarded. Nothing changed."); }}>
              ×
            </button>
          </div>
          <div style={{ fontSize: 13, color: "var(--m-text-muted)" }}>{plan.scopeLabel}. Nothing is booked until you confirm.</div>
          <div className="sum">
            <span className="dp-pill dp-pill-neu">{plan.rows.length} placed</span>
            {pillS}
            {nOv > 0 && (
              <span className="dp-pill dp-pill-neu">
                <StackIcon />
                {nOv} on purpose
              </span>
            )}
            <span className="dp-pill dp-pill-neu">{trv} min travel</span>
          </div>
        </div>
        {plan.failed && (
          <div className="dp-fail" role="alert" data-testid="plan-fail">
            <div>
              <b>Nothing was booked.</b> {plan.failed}
            </div>
            <div className="row">
              <button type="button" className="dp-btn" onClick={() => toast("Opening the first conflict")}>
                Show the conflict
              </button>
              <button type="button" className="dp-btn" onClick={() => { setPlan({ ...plan, failed: null }); toast("Checked again. Still needs your call on the conflict."); }}>
                Retry
              </button>
            </div>
          </div>
        )}
        <div className="rows">
          {plan.rows.map((r, i) => {
            const e = planIssues[i];
            const bad = !!e;
            const blocked = e?.kind === "blocked";
            return (
              <div key={`${r.tk}-${i}`} className={`dp-prow${bad ? " bad" : ""}`}>
                <div className="r1">
                  <span className="num">{r.num}</span>
                  <span className="t" title={r.title}>
                    #{r.tk} {r.title}
                  </span>
                  <PriChip p={r.pri} />
                  <button
                    type="button"
                    className="dp-ib-sm"
                    style={{ border: "none" }}
                    aria-label={`Remove #${r.tk} from the plan`}
                    onClick={() => {
                      setPlan((p) => {
                        if (!p) return p;
                        const rows = p.rows.filter((_, k) => k !== i);
                        rows.forEach((rr, k) => (rr.num = k + 1));
                        return rows.length ? { ...p, rows, n: Math.max(0, p.n - 1) } : null;
                      });
                      toast(`#${r.tk} stays in the queue`);
                    }}
                  >
                    ×
                  </button>
                </div>
                <div className="r2">
                  <select
                    className="dp-sel"
                    aria-label={`Technician for #${r.tk}`}
                    value={r.tech}
                    onChange={(ev) => setPlan((p) => (p ? { ...p, rows: patchRow(p.rows, i, { tech: ev.target.value, ov: undefined }) } : p))}
                  >
                    {TECHS.map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.name}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className="dp-ib-sm"
                    aria-label="15 minutes earlier"
                    onClick={() => {
                      const cur = plan.rows[i];
                      if (!cur) return;
                      setPlan((p) => (p ? { ...p, rows: patchRow(p.rows, i, { s: Math.max(D0, Math.min(D1 - cur.d, cur.s - 15)) }) } : p));
                    }}
                  >
                    −
                  </button>
                  <span className="tm">
                    {hm(r.s)} to {hm(r.s + r.d)}
                  </span>
                  <button
                    type="button"
                    className="dp-ib-sm"
                    aria-label="15 minutes later"
                    onClick={() => {
                      const cur = plan.rows[i];
                      if (!cur) return;
                      setPlan((p) => (p ? { ...p, rows: patchRow(p.rows, i, { s: Math.max(D0, Math.min(D1 - cur.d, cur.s + 15)) }) } : p));
                    }}
                  >
                    +
                  </button>
                </div>
                <div className="why">
                  {blocked ? `Conflict: ${e?.msg}` : e ? `${e.msg}. Keep the overlap?` : r.ov ? `${r.ov} approved` : r.why}
                  {e && !blocked && (
                    <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 8 }}>
                      {(e.sites ? ["waiting", "handle"] : e.allRemote ? ["parallel", "waiting", "handle"] : ["waiting", "handle"]).map((k) => (
                        <button
                          key={k}
                          type="button"
                          className="dp-btn"
                          style={{ height: 28, fontSize: 12, padding: "0 10px", background: "var(--m-surface)", color: "var(--m-text)" }}
                          onClick={() => setPlan((p) => (p ? { ...p, rows: patchRow(p.rows, i, { ov: k as OverlapReason }) } : p))}
                        >
                          {REASON_LABEL[k as OverlapReason]}
                        </button>
                      ))}
                    </div>
                  )}
                  {r.ov && !e && (
                    <div style={{ marginTop: 6, display: "flex", alignItems: "center", gap: 8 }}>
                      <span className="dp-pill dp-pill-neu">
                        <StackIcon />
                        2 at once · {REASON_LABEL[r.ov]}
                      </span>
                      <button type="button" className="dp-linkbtn" style={{ fontSize: 13 }} onClick={() => setPlan((p) => (p ? { ...p, rows: patchRow(p.rows, i, { ov: undefined }) } : p))}>
                        Undo
                      </button>
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
        <div className="pf">
          <div className="note">
            {plan.rows.some((r) => r.mode === "site")
              ? "Confirming books the visits. Confirmations for clients are prepared for you to send, never sent for you."
              : "Confirming books the work on the board."}
          </div>
          <div className="row">
            <button type="button" className="dp-btn" onClick={() => { setPlan(null); setSel(null); toast("Plan discarded. Nothing changed."); }}>
              Discard
            </button>
            <button type="button" className="dp-btn primary" disabled={nBlocked > 0 || nOverlap > 0} onClick={confirmPlan}>
              {nBlocked
                ? `Fix ${nBlocked} conflict${nBlocked > 1 ? "s" : ""} to confirm`
                : nOverlap
                  ? `Decide on ${nOverlap} overlap${nOverlap > 1 ? "s" : ""} to confirm`
                  : "Confirm plan"}
            </button>
          </div>
        </div>
      </aside>
    );
  }

  // ---- week view --------------------------------------------------------
  function renderWeek(): React.JSX.Element {
    const days = ["Mon 28", "Tue 29", "Wed 30", "Thu 1", "Fri 2"];
    // The pack's own week matrix: Tuesday is the real board's utilisation, the
    // rest is a deterministic spread with four fixed exceptions.
    const overMap = new Map([
      ["SK:4", 1.12],
      ["KB:2", 1.05],
      ["AM:3", 1.03],
      ["JT:4", 0],
      ["LZ:0", 0],
    ]);
    return (
      <div className="dp-wk" data-testid="week-view">
        <table>
          <thead>
            <tr>
              <th>Technician</th>
              {days.map((d, i) => (
                <th key={d} className={i === 1 ? "today" : ""}>
                  {d}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {lanes.map((t) => {
              const i = TECHS.indexOf(t);
              return (
                <tr key={t.id}>
                  <td className="tech">
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span className={`av${t.id === ME_TECH ? " me" : ""}`}>{t.id}</span>
                      <div>
                        <div style={{ fontWeight: 600 }}>{t.name}</div>
                        <div style={{ fontSize: 12, color: "var(--m-text-muted)" }}>{t.role}</div>
                      </div>
                    </div>
                  </td>
                  {days.map((d, k) => {
                    const key = `${t.id}:${k}`;
                    const p = k === 1 ? utilisation([...jobs, ...planRowsAsJobs(plan)].filter((j) => j.tech === t.id), !once).pctv / 100 : (overMap.get(key) ?? 0.5 + ((i * 7 + k * 13) % 42) / 100);
                    const over = p > 1;
                    const off = p === 0;
                    const jc = off ? 0 : Math.round(p * 5);
                    return (
                      <td key={d}>
                        <button
                          type="button"
                          className={`cell${over ? " over" : ""}${off ? " off" : ""}`}
                          aria-label={`${t.name} ${d}: ${off ? "off" : `${Math.round(p * 100)} percent`}`}
                          onClick={() => setView("day")}
                        >
                          <span className="c1">
                            <span>{off ? "Off" : `${Math.round(p * 100)}%`}</span>
                            {over && <AlertIcon size={15} />}
                          </span>
                          <span className="c2">
                            {off ? "Leave" : `${over ? `Over by ${dur(Math.round((p - 1) * CAP))} · ` : ""}${jc} jobs`}
                          </span>
                        </button>
                      </td>
                    );
                  })}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    );
  }

  // ---- unassigned table -------------------------------------------------
  function renderQueueTable(): React.JSX.Element {
    const vis = cols.filter((c) => c.visible);
    const grid = vis.map((c) => COLW[c.key] ?? "minmax(0,1fr)").join(" ");
    const selN = qsel.size;
    const cell = (q: QueueTicket, k: string): React.JSX.Element => {
      switch (k) {
        case "pri":
          return <PriChip p={q.pri} />;
        case "id":
          return <span className="dp-idc">#{q.tk}</span>;
        case "title":
          return <span style={{ fontWeight: 500 }}>{q.title}</span>;
        case "client":
          return <span>{q.client}</span>;
        case "est":
          return <span style={{ fontVariantNumeric: "tabular-nums" }}>{dur(q.est)}</span>;
        case "need":
          return <span>{q.need}</span>;
        case "where":
          return (
            <span className="dp-where">
              {q.mode === "site" ? <LocationIcon size={15} /> : <MonitorIcon size={15} />}
              {q.mode === "site" ? "On site" : "Remote"}
            </span>
          );
        case "sla":
          return q.sla === null ? (
            <span className="dp-hint2">No SLA</span>
          ) : q.sla <= 60 ? (
            <span className="dp-pill dp-pill-bad">
              <Icon name="clock" size={14} />
              {dur(q.sla)} left
            </span>
          ) : (
            <span style={{ fontVariantNumeric: "tabular-nums" }}>{dur(q.sla)}</span>
          );
        default:
          return <></>;
      }
    };
    return (
      <div className="dp-qwrap" data-testid="unassigned-table">
        {/* The selection bar replaces the column header, the way the pack draws
            it: one soft-gold band with the bulk actions, not a table row that
            pretends to be both. */}
        {selN > 0 ? (
          <div className="dp-selbar" data-testid="selection-bar">
            <input type="checkbox" checked aria-label="Deselect all" onChange={() => setQsel(new Set())} />
            <b>{selN} selected</b>
            <div className="dp-anchor">
              <button type="button" className="dp-btn" onClick={() => setPop(pop === "assign" ? null : "assign")}>
                Assign to…
              </button>
              {pop === "assign" && renderAssignMenu()}
            </div>
            <button
              type="button"
              className="dp-btn"
              onClick={() => {
                setView("day");
                toast("Drag the tickets onto a lane, or press Enter on a card and use the arrow keys.");
              }}
            >
              Place on the board
            </button>
            {ai && (
              <button type="button" className="dp-btn" onClick={() => setPop("scope")}>
                Plan with AI
              </button>
            )}
            <button type="button" className="dp-linkbtn" style={{ marginLeft: "auto" }} onClick={() => setQsel(new Set())}>
              Clear selection
            </button>
          </div>
        ) : null}
        {selN === 0 ? (
          <div className="dp-trow h" style={{ gridTemplateColumns: `44px ${grid}` }}>
            <div>
              <input type="checkbox" aria-label="Select all" onChange={() => setQsel(new Set(filteredQueue.map((q) => q.tk)))} />
            </div>
            {vis.map((c) => (
              <div key={c.key}>{c.label}</div>
            ))}
          </div>
        ) : null}
        {filteredQueue.map((q) => (
          <div key={q.tk} className={`dp-trow${qsel.has(q.tk) ? " sel" : ""}`} style={{ gridTemplateColumns: `44px ${grid}` }}>
            <div>
              <input
                type="checkbox"
                checked={qsel.has(q.tk)}
                aria-label={`Select #${q.tk}`}
                onChange={() =>
                  setQsel((s) => {
                    const n = new Set(s);
                    if (n.has(q.tk)) n.delete(q.tk);
                    else n.add(q.tk);
                    return n;
                  })
                }
              />
            </div>
            {vis.map((c) => (
              <div key={c.key}>{cell(q, c.key)}</div>
            ))}
          </div>
        ))}
        {filteredQueue.length === 0 && (
          <div className="dp-hint2" style={{ padding: 24 }}>
            Nothing unassigned.
          </div>
        )}
      </div>
    );
  }

  function renderAssignMenu(): React.JSX.Element {
    return (
      <div className="dp-menu">
        {TECHS.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => {
              const ids = [...qsel];
              let n = 0;
              for (const tk of ids) {
                const q = queue.find((x) => x.tk === tk);
                if (!q) continue;
                const slot = autoSlot(t.id, q.est);
                if (slot != null) {
                  setJobs((js) => [
                    ...js,
                    { jid: `j${Date.now()}-${tk}`, tk: q.tk, title: q.title, client: q.client, clientZone: q.clientZone, tech: t.id, s: slot, d: q.est, mode: q.mode, pri: q.pri },
                  ]);
                  setQueue((qs) => qs.filter((x) => x.tk !== q.tk));
                  n++;
                }
              }
              setQsel(new Set());
              setPop(null);
              toast(`Assigned ${n} ticket${n === 1 ? "" : "s"} to ${t.name} in the earliest free slots`);
            }}
          >
            <span className="av" style={{ width: 24, height: 24, borderRadius: 999, background: "var(--m-lime)", color: "var(--m-paper)", display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 12, fontWeight: 600 }}>
              {t.id}
            </span>
            {t.name}
          </button>
        ))}
      </div>
    );
  }

  // ---- footer -----------------------------------------------------------
  // The pack's board footer (DESIGN-NOTES.md §B2: legend, density control,
  // keyboard hints) worn on the template's own strip. `.m-surface-footer` is
  // what every other surface's footer carries, and the audit that sent this
  // lane here reads exactly that root to answer "is a footer present" — the
  // private `.dp-ft` name it used before was invisible to it. It is composed
  // from the strip's parts rather than `SurfaceFooter` because a board has no
  // rows to count and that component's count and load-more shape belongs to
  // list surfaces. The density control is the kit's `DensitySwitch` — the
  // pack's own footer control (§B9), identical to every ledger footer — where
  // this page used to hand-roll a text segment.
  const renderFooter = (
    <div className="m-surface-footer dp-ft" role="group" aria-label="Board footer" data-testid="dispatch-footer">
      <span className="dp-legend">
        <span>
          <LocationIcon size={14} /> On site
        </span>
        <span>
          <MonitorIcon size={14} /> Remote
        </span>
        <span>
          <i className="dp-hatchsw" /> Travel
        </span>
        <span>
          <StackIcon size={14} /> Double-booked
        </span>
        {ai && (
          <span>
            <i className="dp-propsw" /> Proposed
          </span>
        )}
      </span>
      <span className="m-surface-footer-gap" />
      <DensitySwitch value={density} onChange={setDensityState} label="Density" />
      <span className="dp-kbd">←</span>
      <span className="dp-kbd">→</span>
      <span className="dp-kbd">↑</span>
      <span className="dp-kbd">↓</span>
      <span>move</span>
      <span className="dp-kbd">↵</span>
      <span>pick up, drop</span>
      <span className="dp-kbd">S</span>
      <span>split</span>
      <span className="dp-kbd">Esc</span>
      <span>cancel</span>
    </div>
  );

  // ---- settings modal ---------------------------------------------------
  const renderSettings =
    settings === true ? (
      <div className="dp-scrim" onClick={(e) => { if (e.target === e.currentTarget) setSettings(false); }}>
        <div className="dp-modal" role="dialog" aria-modal="true" aria-labelledby="dp-st-title" data-testid="dispatch-settings">
          <div className="mh">
            <div>
              <h2 id="dp-st-title">Dispatch settings</h2>
              <p>Overlapping work. Applies to the whole team.</p>
            </div>
            <span style={{ flex: 1 }} />
            <button type="button" className="dp-ib" aria-label="Close" onClick={() => setSettings(false)}>
              ×
            </button>
          </div>
          <div className="mb">
            <div className="dp-lab" style={{ marginBottom: 8 }}>
              When two jobs overlap on one technician
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <label className="dp-optcard" data-on={policy === "never"}>
                <input type="radio" name="pol" checked={policy === "never"} onChange={() => setPolicy("never")} />
                <span>
                  <b>Never</b>
                  <small>One job at a time. A drop that overlaps is blocked, with the reason.</small>
                </span>
              </label>
              <label className="dp-optcard" data-on={policy === "remote"}>
                <input type="radio" name="pol" checked={policy === "remote"} onChange={() => setPolicy("remote")} />
                <span>
                  <b>Remote work only</b>
                  <small>Remote jobs can run in parallel. On-site jobs never overlap.</small>
                </span>
              </label>
              <label className="dp-optcard" data-on={policy === "ask"}>
                <input type="radio" name="pol" checked={policy === "ask"} onChange={() => setPolicy("ask")} />
                <span>
                  <b>Always ask me</b>
                  <small>Any overlap is allowed once the dispatcher says why. Both jobs stay on the board, stacked. This is the default.</small>
                </span>
              </label>
            </div>
            <div className="dp-oncelab">
              <b>Most jobs at once per technician</b>
              <span className="dp-stepper" role="group" aria-label="Limit">
                <button type="button" aria-label="Fewer" onClick={() => setLimit((l) => Math.max(2, l - 1))}>
                  −
                </button>
                <span className="val">{limit}</span>
                <button type="button" aria-label="More" onClick={() => setLimit((l) => Math.min(4, l + 1))}>
                  +
                </button>
              </span>
            </div>
            <div className="dp-oncelab">
              <b>Count overlapped time</b>
              <span className="dp-seg" role="group" aria-label="Count overlapped time">
                <button type="button" aria-pressed={once} style={{ height: 32 }} onClick={() => setOnce(true)}>
                  Once
                </button>
                <button type="button" aria-pressed={!once} style={{ height: 32 }} onClick={() => setOnce(false)}>
                  In full
                </button>
              </span>
            </div>
            <div className="dp-hint2">
              Counting once means two remote jobs side by side use the technician&apos;s time once, so they do not look overloaded. Travel
              between different sites is still added.
            </div>
          </div>
          <div className="mf">
            <span className="hint">Only admins can change this.</span>
            <button type="button" className="dp-btn primary" onClick={() => setSettings(false)}>
              Done
            </button>
          </div>
        </div>
      </div>
    ) : null;

  return (
    <div className="dp" style={{ display: "flex", flexDirection: "column", height: "100%", padding: "0 24px", minWidth: 0 }}>
      <p className="sr-only" aria-live="assertive" role="status">
        {live}
      </p>
      <RoutingStrip dayIso="2026-09-29" />
      {renderHeader}
      {renderToolbar}
      {renderBanner}
      <div className="dp-view">{view === "week" ? renderWeek() : view === "queue" ? renderQueueTable() : renderBoard()}</div>
      {renderFooter}
      {renderSettings}
      {toastMsg && (
        <div role="status" className="dp-ovc" style={{ position: "fixed", left: "50%", bottom: 84, transform: "translateX(-50%)", width: "auto", maxWidth: "calc(100vw - 32px)", background: "var(--m-ink)", color: "var(--m-cream)", zIndex: 100 }} data-testid="toast">
          {toastMsg}
        </div>
      )}
    </div>
  );
}

// ---- module-scope helpers ----------------------------------------------

function snap5(m: number): number {
  return Math.round(m / 15) * 15;
}

/** Narrowing-safe row patch: the row must exist, and the result is a whole
 *  PlanRow rather than a partial spread of a possibly-undefined one. */
function patchRow(rows: PlanRow[], i: number, patch: Partial<PlanRow>): PlanRow[] {
  const cur = rows[i];
  if (!cur) return rows;
  const next = [...rows];
  next[i] = { ...cur, ...patch };
  return next;
}

function slotAt(x: number, y: number, laneEls: Array<HTMLDivElement | null>): { lane: number; min: number } | null {
  if (!laneEls.length) return null;
  const lt = laneEls[0]?.querySelector<HTMLElement>(".dp-lt")?.getBoundingClientRect();
  if (!lt) return null;
  let li = -1;
  for (let i = 0; i < laneEls.length; i++) {
    const el = laneEls[i];
    if (!el) continue;
    const r = el.getBoundingClientRect();
    if (y >= r.top && y < r.bottom) {
      li = i;
      break;
    }
  }
  if (li < 0 || x < lt.left - 30 || x > lt.right + 30) return null;
  return { lane: li, min: D0 + ((x - lt.left) / lt.width) * (D1 - D0) };
}

function freeNow(t: BoardTech, jobs: BoardJob[], absences: BoardAbsence[]): boolean {
  const mine = jobs.filter((j) => j.tech === t.id);
  if (mine.some((j) => j.s <= NOW && NOW < j.s + j.d)) return false;
  if (absences.some((a) => a.tech === t.id && a.s <= NOW && NOW < a.s + a.d)) return false;
  return true;
}

function collisionSay(x: {
  tech: string;
  a: { tk?: number; s: number; d: number; title?: string; abs?: boolean };
  b: { tk?: number; s: number; d: number; title?: string; abs?: boolean };
}): string {
  const name = TECHS.find((t) => t.id === x.tech)?.name ?? x.tech;
  if (x.b.abs) return `${name} is off from ${hm(x.b.s)} (${(x.b.title ?? "off").replace(/^Off: /, "").toLowerCase()}), but #${x.a.tk} runs to ${hm(x.a.s + x.a.d)}`;
  if (x.a.abs) return `${name} is off from ${hm(x.a.s)}, but #${x.b.tk} runs to ${hm(x.b.s + x.b.d)}`;
  return `${name}: #${x.a.tk} runs into #${x.b.tk} at ${hm(x.b.s)}`;
}

function planRowsAsJobs(plan: PlanState | null): BoardJob[] {
  if (!plan) return [];
  return plan.rows.map((r, i) => ({
    jid: `pr${i}`,
    tk: r.tk,
    title: r.title,
    client: r.client,
    clientZone: ZONES[r.client] ?? 1,
    tech: r.tech,
    s: r.s,
    d: r.d,
    mode: r.mode,
    pri: r.pri,
    prop: true,
    num: r.num,
    part: r.part,
    ov: r.ov,
    ri: i,
  }));
}

function makePlan(list: QueueTicket[], jobs: BoardJob[], absences: BoardAbsence[]): PlanRow[] {
  const virt = jobs.slice();
  const rows: PlanRow[] = [];
  const order = [...list].sort((a, b) => (a.sla === null ? 9e9 : a.sla) - (b.sla === null ? 9e9 : b.sla));
  const from = Math.ceil((NOW + 1) / 15) * 15;
  for (const q of order) {
    let best: { score: number; tech: string; s: number; tv: number; match: boolean } | null = null;
    for (const t of TECHS) {
      if (t.role === "Projects") continue;
      if (q.mode === "site" && t.role === "Service desk") continue;
      const match = t.sk.includes(q.need);
      const tmp = virt.filter((j) => j.tech === t.id);
      for (let s = from; s + q.est <= 1050; s += 15) {
        const hit = tmp.some((j) => s < j.s + j.d && j.s < s + q.est) || absences.some((a) => a.tech === t.id && s < a.s + a.d && a.s < s + q.est);
        if (hit) continue;
        const probe: BoardJob = { jid: "probe", tk: q.tk, title: q.title, client: q.client, clientZone: q.clientZone, tech: t.id, s, d: q.est, mode: q.mode, pri: q.pri };
        const tv = travelMin(probe, tmp);
        const score = (match ? 0 : 1000) + s + tv * 2;
        if (!best || score < best.score) best = { score, tech: t.id, s, tv, match };
        break;
      }
    }
    if (best) {
      virt.push({ jid: "v", tk: q.tk, title: q.title, client: q.client, clientZone: q.clientZone, tech: best.tech, s: best.s, d: q.est, mode: q.mode, pri: q.pri });
      const t = TECHS.find((x) => x.id === best.tech);
      rows.push({
        tk: q.tk,
        title: q.title,
        client: q.client,
        pri: q.pri,
        tech: best.tech,
        s: best.s,
        d: q.est,
        mode: q.mode,
        why:
          (best.match ? `${q.need} skill, ` : `No ${q.need} skill is free today; ${t?.name.split(" ")[0] ?? "someone"} fits. `) +
          (q.mode === "site" ? `${best.tv} min travel` : "remote, no travel") +
          (q.sla !== null && q.sla <= 120 ? `, done before the SLA (${dur(q.sla)} left).` : "."),
        num: 0,
      });
    }
  }
  rows.sort((a, b) => a.s - b.s || a.tk - b.tk);
  rows.forEach((r, i) => (r.num = i + 1));
  return rows;
}
