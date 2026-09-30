// Pure-model tests for the dispatch board (Wave 41 lane 6).
//
// The overlap decision family is the load-bearing logic this lane adds:
// DESIGN-NOTES §B5b fixes five drop outcomes (free, overlap, cross-site
// overlap, blocked-by-policy, blocked-by-limit), §B3 fixes utilisation
// counting overlapped time once by default, §B4 fixes approved-overlap
// stacking, and §B6 fixes the hatched travel rule. These are the assertions
// the audit reads against the pack.
import { describe, expect, it } from "vitest";
import {
  approvedWindows,
  CAP,
  collisionsOf,
  dur,
  evalDrop,
  hm,
  pct,
  presenceOf,
  stackRows,
  statusOf,
  travelMin,
  utilisation,
  type AbsenceItem,
  type BoardJob,
  type BoardTech,
} from "./dispatch-model";
import { TECHS, seedAbsences, seedJobs, seedQueue } from "./dispatch-seed";

const tech = (id: string): BoardTech => TECHS.find((t) => t.id === id) as BoardTech;
const byTech = (id: string): BoardTech | undefined => TECHS.find((t) => t.id === id);

const job = (o: Partial<BoardJob> & Pick<BoardJob, "jid" | "tk" | "tech" | "s" | "d">): BoardJob => ({
  title: "Job",
  client: "Acme Corporation",
  clientZone: 0,
  mode: "remote",
  pri: "P3",
  ...o,
});

const absence = (o: Partial<AbsenceItem> & Pick<AbsenceItem, "jid" | "tech" | "s" | "d">): AbsenceItem => ({
  title: "Off: appointment",
  abs: true,
  tk: 0,
  ...o,
});

describe("dispatch time axis", () => {
  it("formats minutes as the pack does (07:00, 14:30, 1h 05m)", () => {
    expect(hm(420)).toBe("07:00");
    expect(hm(870)).toBe("14:30");
    expect(dur(65)).toBe("1h 05m");
    expect(dur(60)).toBe("1h");
    expect(dur(45)).toBe("45m");
  });

  it("maps minutes to the 07:00–18:00 percentage the CSS positions against", () => {
    expect(pct(420)).toBe(0);
    expect(pct(1080)).toBe(100);
    expect(pct(701)).toBeCloseTo(42.576, 2);
  });
});

describe("travel (§B6 hatched strips)", () => {
  it("adds nothing for remote work", () => {
    expect(travelMin(job({ jid: "a", tk: 1, tech: "TL", s: 600, d: 30, mode: "remote" }), [])).toBe(0);
  });

  it("adds 12 min plus 10 per zone, and nothing for the same client", () => {
    const prev = job({ jid: "p", tk: 1, tech: "TL", s: 480, d: 60, mode: "site", client: "Acme Corporation", clientZone: 0 });
    const far = job({ jid: "n", tk: 2, tech: "TL", s: 600, d: 30, mode: "site", client: "Kestrel Logistics", clientZone: 4 });
    expect(travelMin(far, [prev])).toBe(12 + 10 * 4);
    const same = job({ jid: "n2", tk: 3, tech: "TL", s: 600, d: 30, mode: "site", client: "Acme Corporation", clientZone: 0 });
    expect(travelMin(same, [prev])).toBe(0);
  });

  it("uses zone 1 as the home base when nothing precedes the job", () => {
    const first = job({ jid: "f", tk: 4, tech: "TL", s: 480, d: 30, mode: "site", client: "Acme Corporation", clientZone: 0 });
    expect(travelMin(first, [])).toBe(12 + 10);
  });
});

describe("utilisation (§B3 overlapped time counted once)", () => {
  it("counts two overlapping jobs once by default and in full when asked", () => {
    const a = job({ jid: "a", tk: 1, tech: "TL", s: 600, d: 60 });
    const b = job({ jid: "b", tk: 2, tech: "TL", s: 630, d: 60 });
    expect(utilisation([a, b], false).min).toBe(90);
    expect(utilisation([a, b], true).min).toBe(120);
  });

  it("adds travel before the block, so a site day reads longer than the sum", () => {
    const remote = job({ jid: "r", tk: 5, tech: "TL", s: 600, d: 60, mode: "remote" });
    expect(utilisation([remote], false).min).toBe(60);
    const site = job({ jid: "s", tk: 6, tech: "AM", s: 600, d: 60, mode: "site", client: "Acme Corporation", clientZone: 0 });
    // 12 + 10 from the home zone, so 82 minutes of the day is committed.
    expect(utilisation([site], false).min).toBe(60 + 22);
  });

  it("reports the over-capacity figure the pack prints (Over by 53m)", () => {
    const u = utilisation([job({ jid: "x", tk: 7, tech: "TL", s: 480, d: 480 })], false);
    expect(u.pctv).toBe(100);
    const over = utilisation([job({ jid: "y", tk: 8, tech: "TL", s: 480, d: 533 })], false);
    expect(over.pctv).toBeGreaterThan(100);
    expect(dur(over.min - 480)).toBe("53m");
  });
});

describe("presence (§B3)", () => {
  it("reads On site with the client while a site job runs", () => {
    const running = job({ jid: "a", tk: 1, tech: "TL", s: 690, d: 60, mode: "site", client: "Acme Corporation" });
    const p = presenceOf(tech("TL"), [running], []);
    expect(p.txt).toBe("On site · Acme Corporation");
  });

  it("reads Remote with the ticket number for remote work", () => {
    const running = job({ jid: "a", tk: 18421, tech: "TL", s: 690, d: 60, mode: "remote" });
    expect(presenceOf(tech("TL"), [running], []).txt).toBe("Remote · #18421");
  });

  it("reads Free with the next job time, and Off today when absent", () => {
    const later = job({ jid: "a", tk: 18421, tech: "TL", s: 810, d: 45, mode: "remote" });
    expect(presenceOf(tech("TL"), [later], []).txt).toBe("Free · next 13:30");
    expect(presenceOf(tech("JT"), [later], [absence({ jid: "ab", tech: "JT", s: 700, d: 240 })]).txt).toBe("Off today");
  });
});

describe("status of a block", () => {
  it("marks finished work done, the running block live, and the rest scheduled", () => {
    expect(statusOf({ s: 480, d: 60 })).toBe("done");
    expect(statusOf({ s: 690, d: 60 })).toBe("live");
    expect(statusOf({ s: 900, d: 60 })).toBe("sched");
  });
});

describe("collisions (§B4 banner counts only what nobody chose)", () => {
  it("reports an unapproved overlap", () => {
    const c = collisionsOf(
      [job({ jid: "a", tk: 1, tech: "TL", s: 600, d: 60 }), job({ jid: "b", tk: 2, tech: "TL", s: 630, d: 60 })],
      byTech,
    );
    expect(c).toHaveLength(1);
    expect(c[0]?.s).toBe(630);
    expect(c[0]?.e).toBe(660);
  });

  it("stays silent once either side carries an approved reason", () => {
    const c = collisionsOf(
      [
        job({ jid: "a", tk: 1, tech: "TL", s: 600, d: 60, ov: "parallel" }),
        job({ jid: "b", tk: 2, tech: "TL", s: 630, d: 60 }),
      ],
      byTech,
    );
    expect(c).toHaveLength(0);
  });

  it("always reports a collision with an absence, and sorts it last", () => {
    const c = collisionsOf(
      [
        job({ jid: "a", tk: 1, tech: "TL", s: 600, d: 60 }),
        job({ jid: "b", tk: 2, tech: "TL", s: 660, d: 60 }),
        absence({ jid: "ab", tech: "TL", s: 690, d: 120 }),
      ],
      byTech,
    );
    expect(c).toHaveLength(1);
    expect(c[0]?.b).toMatchObject({ abs: true });
  });

  it("ignores a technician it does not know (a filter hid the lane)", () => {
    expect(
      collisionsOf([job({ jid: "a", tk: 1, tech: "ZZ", s: 600, d: 60 }), job({ jid: "b", tk: 2, tech: "ZZ", s: 610, d: 60 })], byTech),
    ).toHaveLength(0);
  });
});

describe("stacking (§B4 approved overlaps split the lane into two rows)", () => {
  it("keeps one row when nothing is double-booked", () => {
    const jobs = [job({ jid: "a", tk: 1, tech: "TL", s: 600, d: 60 }), job({ jid: "b", tk: 2, tech: "TL", s: 660, d: 60 })];
    expect(stackRows(jobs)).toBe(1);
  });

  it("uses two rows for an approved overlap and labels the lane ×2", () => {
    const jobs = [
      job({ jid: "a", tk: 1, tech: "TL", s: 600, d: 60, ov: "parallel" }),
      job({ jid: "b", tk: 2, tech: "TL", s: 630, d: 60 }),
    ];
    expect(stackRows(jobs)).toBe(2);
    expect(approvedWindows(jobs)).toEqual([{ s: 630, e: 660 }]);
  });
});

describe("drop evaluation (§B5b, the five outcomes)", () => {
  const base = { jid: "x", tech: "TL", techName: "Tumi L.", s: 600, d: 60, mode: "remote" as const, clientZone: 0, client: "Acme Corporation" };

  it("is free on an empty slot", () => {
    const ev = evalDrop(base, [], [], "ask", 3);
    expect(ev.kind).toBe("free");
    expect(ev.ok).toBe(true);
  });

  it("blocks outside board hours", () => {
    expect(evalDrop({ ...base, s: 360 }, [], [], "ask", 3).kind).toBe("blocked");
    expect(evalDrop({ ...base, s: 1030, d: 60 }, [], [], "ask", 3).kind).toBe("blocked");
  });

  it("blocks a drop onto an absence and names when they are back", () => {
    const ev = evalDrop({ ...base, tech: "JT", techName: "Jordan T.", s: 870 }, [], [absence({ jid: "ab", tech: "JT", s: 840, d: 240 })], "ask", 3);
    expect(ev.kind).toBe("blocked");
    expect(ev.msg).toBe("Jordan T. is off until 18:00");
  });

  it("asks on an overlap under the default policy, and offers the two reasons", () => {
    const ev = evalDrop(base, [job({ jid: "h", tk: 18421, tech: "TL", s: 600, d: 60 })], [], "ask", 3);
    expect(ev.kind).toBe("overlap");
    expect(ev.ok).toBe(true);
    expect(ev.msg).toBe("Overlaps #18421 (10:00 to 11:00)");
  });

  it("blocks every overlap when the policy is never", () => {
    const ev = evalDrop(base, [job({ jid: "h", tk: 18421, tech: "TL", s: 600, d: 60 })], [], "never", 3);
    expect(ev.kind).toBe("blocked");
    expect(ev.msg).toContain("Overlapping work is switched off");
  });

  it("allows only remote work in parallel under the remote policy", () => {
    const remote = evalDrop(base, [job({ jid: "h", tk: 18421, tech: "TL", s: 600, d: 60, mode: "remote" })], [], "remote", 3);
    expect(remote.kind).toBe("overlap");
    expect(remote.allRemote).toBe(true);
    const site = evalDrop(
      { ...base, mode: "site" },
      [job({ jid: "h", tk: 18421, tech: "TL", s: 600, d: 60, mode: "site" })],
      [],
      "remote",
      3,
    );
    expect(site.kind).toBe("blocked");
    expect(site.msg).toContain("Only remote work can run in parallel");
  });

  it("flags a cross-site overlap with the road gap instead of a plain overlap", () => {
    const ev = evalDrop(
      { ...base, mode: "site", client: "Kestrel Logistics", clientZone: 4 },
      [job({ jid: "h", tk: 18433, tech: "TL", s: 615, d: 60, mode: "site", client: "Acme Corporation", clientZone: 0 })],
      [],
      "ask",
      3,
    );
    expect(ev.kind).toBe("overlap");
    expect(ev.sites).toBe(true);
    expect(ev.gap).toBe(12 + 10 * 4);
    expect(ev.msg).toBe("Different sites, 52 min apart");
  });

  it("blocks a third job at once once the limit is exceeded", () => {
    const lane = [
      job({ jid: "h1", tk: 1, tech: "TL", s: 600, d: 60 }),
      job({ jid: "h2", tk: 2, tech: "TL", s: 620, d: 60 }),
    ];
    expect(evalDrop({ ...base, s: 630 }, lane, [], "ask", 3).kind).toBe("overlap");
    const ev = evalDrop({ ...base, s: 630 }, lane, [], "ask", 2);
    expect(ev.kind).toBe("blocked");
    expect(ev.msg).toContain("would have 3 jobs at once");
  });
});

describe("frozen seed reproduces the pack's printed numbers", () => {
  // These are read off the pack's board-at-rest render (state 01). They are
  // the regression net: if travel, utilisation or the seed drifts, the board
  // stops matching the pack and one of these moves first.
  it("prints every lane's utilisation the way the pack prints it", () => {
    const jobs = seedJobs();
    const got: Record<string, number> = {};
    for (const t of TECHS) got[t.id] = utilisation(jobs.filter((j) => j.tech === t.id), false).pctv;
    expect(got).toEqual({ TL: 70, AM: 98, AO: 74, RN: 50, NP: 44, SK: 111, JT: 59, KB: 89, DM: 94, LZ: 55, PV: 31, BH: 50 });
  });

  it("prints SK's over-capacity figure as the pack does (111%, Over by 53m)", () => {
    const u = utilisation(seedJobs().filter((j) => j.tech === "SK"), false);
    expect(u.pctv).toBe(111);
    expect(u.min - CAP).toBe(53);
    expect(dur(u.min - CAP)).toBe("53m");
  });

  it("reads presence at the frozen 11:41 clock, with the absence not yet started", () => {
    const jobs = seedJobs();
    const abs = seedAbsences();
    expect(presenceOf(tech("TL"), jobs, abs).txt).toBe("Remote · #18432");
    // Jordan T.'s absence starts at 14:00, so at 11:41 they are still on site.
    expect(presenceOf(tech("JT"), jobs, abs).txt).toBe("On site · Lumen Architects");
  });
});

describe("frozen seed matches the pack", () => {
  it("carries the pack's twelve technicians, thirty-seven jobs, one absence and six queue tickets", () => {
    expect(TECHS).toHaveLength(12);
    expect(seedJobs()).toHaveLength(37);
    expect(seedAbsences()).toHaveLength(1);
    expect(seedQueue()).toHaveLength(6);
  });

  it("puts the pack's live block on Tumi L. at 10:30 and the flagged overlap on Alex M.", () => {
    const jobs = seedJobs();
    expect(jobs.find((j) => j.tk === 18432)).toMatchObject({ tech: "TL", s: 630, d: 120, mode: "remote" });
    expect(jobs.find((j) => j.tk === 18348)).toMatchObject({ tech: "AM", s: 615, d: 105, mode: "site" });
  });

  it("carries the queue's tightest SLA first so the plan reads by urgency", () => {
    const q = seedQueue();
    expect(q.find((x) => x.tk === 18441)?.sla).toBe(50);
    expect(q.filter((x) => x.sla === null).map((x) => x.tk)).toEqual([18435, 18430]);
  });
});
