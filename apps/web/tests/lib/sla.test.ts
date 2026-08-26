// Unit tests: /lib/admin/sla — offset→kind mapping, ladder validation and
// payload shaping (typed against ReplaceLadderRequest), policy target
// summaries, calendar draft reducer + validation.
import { describe, expect, it } from "vitest";
import type { components } from "@meridian/sdk";

type EscalationStep = components["schemas"]["EscalationStep"];
import {
  calendarDraftFromBusinessCalendar,
  calendarReducer,
  emptyCalendarDraft,
  isLadderKind,
  kindExplanation,
  ladderPayload,
  ladderReducer,
  policyTargetSummary,
  suggestKindForOffset,
  validateCalendar,
  validateLadder,
} from "../../lib/admin/sla";

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;

describe("offset → notification-kind mapping", () => {
  it("maps early offsets to risk, mid to escalate, ≥100% to breach", () => {
    expect(suggestKindForOffset(25)).toBe("risk");
    expect(suggestKindForOffset(49)).toBe("risk");
    expect(suggestKindForOffset(50)).toBe("escalate");
    expect(suggestKindForOffset(90)).toBe("escalate");
    expect(suggestKindForOffset(100)).toBe("breach");
    expect(suggestKindForOffset(500)).toBe("breach");
  });

  it("keeps a user-chosen kind through the reducer while sync applies otherwise", () => {
    let state = ladderReducer({ rows: [] }, { type: "add-row", defaultAtPct: 50 });
    const key = state.rows[0]?.key ?? "";
    // User explicitly downgrades an escalate row to risk — must survive.
    state = ladderReducer(state, { type: "update", key, patch: { kind: "risk" } });
    state = ladderReducer(state, { type: "update", key, patch: { atPct: 99 } });
    expect(state.rows[0]?.kind).toBe("risk");
    // A fresh pristine row added at another offset follows its suggestion…
    state = ladderReducer(state, { type: "add-row", defaultAtPct: 150 });
    expect(state.rows[1]?.kind).toBe("breach");
    // …while the user-chosen kind stays sticky even past 100%.
    state = ladderReducer(state, { type: "update", key, patch: { atPct: 100 } });
    expect(state.rows[0]?.kind).toBe("risk");
  });

  it("every kind explains itself", () => {
    for (const kind of ["risk", "escalate", "breach"] as const) {
      expect(kindExplanation(kind).length).toBeGreaterThan(10);
    }
  });
});

describe("ladder draft + payload", () => {
  const serverSteps = (): EscalationStep[] => [
    {
      id: uuid(1), org_id: "o", policy_id: "p", step_index: 0, at_pct: 75,
      trigger_kind: "escalate", enabled: true, notify_user_ids: [uuid(9)],
    },
    {
      id: uuid(2), org_id: "o", policy_id: "p", step_index: 1, at_pct: 100,
      trigger_kind: "breach", enabled: false, notify_user_ids: [],
    },
  ];

  it("init sorts by step_index, keeps enabled flags and unknown kinds surface as invalid", () => {
    const swapped = [...serverSteps()].reverse();
    let state = ladderReducer({ rows: [] }, { type: "init", steps: swapped });
    expect(state.rows.map((r) => r.atPct)).toEqual([75, 100]);
    expect(state.rows[1]?.enabled).toBe(false);

    const first = serverSteps()[0];
    if (first === undefined) throw new Error("fixture missing");
    const weird: EscalationStep[] = serverSteps().map((s, i) =>
      i === 0 ? { ...first, trigger_kind: "mystery" } : s,
    );
    state = ladderReducer({ rows: [] }, { type: "init", steps: weird });
    expect(isLadderKind(state.rows[0]?.kind ?? "")).toBe(false);
    expect(validateLadder(state).ok).toBe(false);
  });

  it("payload assigns contiguous step_index in row order; typed as PUT body", () => {
    let state = ladderReducer({ rows: [] }, { type: "init", steps: serverSteps() });
    state = ladderReducer(state, { type: "add-row", defaultAtPct: 150 });
    const body: components["schemas"]["ReplaceLadderRequest"] = ladderPayload(state);
    expect(body.steps.map((s) => s.step_index)).toEqual([0, 1, 2]);
    expect(body.steps[2]).toMatchObject({ at_pct: 150, trigger_kind: "breach", enabled: true });
  });

  it("validation enforces integer percents within 1..=500 and known kinds", () => {
    const seedZero = serverSteps()[0];
    if (seedZero === undefined) throw new Error("fixture missing");
    let state = ladderReducer({ rows: [] }, { type: "init", steps: [{ ...seedZero, at_pct: 0 }] });
    let verdict = validateLadder(state);
    expect(verdict.ok).toBe(false);
    expect(Object.values(verdict.errorsByIndex)[0]).toMatch(/between 1 and 500/);

    state = ladderReducer({ rows: [] }, { type: "init", steps: [{ ...seedZero, at_pct: 501 }] });
    verdict = validateLadder(state);
    expect(verdict.ok).toBe(false);

    state = ladderReducer(
      { rows: [] },
      { type: "init", steps: [{ ...seedZero, at_pct: 80, trigger_kind: "risk" }] },
    );
    expect(validateLadder(state).ok).toBe(true);
  });
});

describe("policy presentation", () => {
  it("target summary joins priority budgets or admits emptiness", () => {
    expect(
      policyTargetSummary({
        id: uuid(1), name: "Gold", is_default: true, uses_embedded_calendar: true,
        calendar_id: null, targets: { p1: 60, p4: 1440 },
      }),
    ).toBe("P1 60m · P4 1440m");
    expect(policyTargetSummary({
      id: uuid(2), name: "Empty", is_default: false, uses_embedded_calendar: false,
      calendar_id: uuid(3), targets: {},
    })).toBe("No targets set");
  });
});

describe("calendar draft editor", () => {
  it("toggle-day opens Mon with 09–17 default then closes again", () => {
    let draft = emptyCalendarDraft();
    draft = calendarReducer(draft, { type: "toggle-day", index: 0 });
    expect(draft.windows[0]).toEqual(["09:00", "17:00"]);
    draft = calendarReducer(draft, { type: "toggle-day", index: 0 });
    expect(draft.windows[0]).toBeNull();
  });

  it("set-window edits one half of an open day only", () => {
    let draft = emptyCalendarDraft();
    draft = calendarReducer(draft, { type: "toggle-day", index: 4 });
    draft = calendarReducer(draft, { type: "set-window", index: 4, half: 1, value: "18:30" });
    expect(draft.windows[4]).toEqual(["09:00", "18:30"]);
    // Editing half of a closed day is a no-op.
    draft = calendarReducer(draft, { type: "set-window", index: 5, half: 0, value: "08:00" });
    expect(draft.windows[5]).toBeNull();
  });

  it("holidays stay sorted and deduped", () => {
    let draft = emptyCalendarDraft();
    draft = calendarReducer(draft, { type: "add-holiday", date: "2026-12-25" });
    draft = calendarReducer(draft, { type: "add-holiday", date: "2026-01-01" });
    draft = calendarReducer(draft, { type: "add-holiday", date: "2026-01-01" });
    expect(draft.holidays).toEqual(["2026-01-01", "2026-12-25"]);
    draft = calendarReducer(draft, { type: "remove-holiday", date: "2026-01-01" });
    expect(draft.holidays).toEqual(["2026-12-25"]);
  });

  it("validation rejects blank names, malformed windows, and inverted ranges", () => {
    const base = calendarDraftFromBusinessCalendar("Corp", {
      timezone: "UTC",
      windows: [["08:00", "17:00"], null, null, null, null, null, null],
      holidays: [],
    });
    expect(Object.keys(validateCalendar(base))).toHaveLength(0);

    let draft = calendarReducer(base, { type: "set-name", value: "" });
    expect(validateCalendar(draft)["name"]).toBeDefined();

    const inverted = calendarReducer(base, { type: "set-window", index: 0, half: 1, value: "07:00" });
    expect(validateCalendar(inverted)["window-0"]).toMatch(/after open/);

    // Partial HH:MM typed by hand fails instead of silently saving.
    const partial = calendarReducer(base, { type: "set-window", index: 0, half: 0, value: "8:5" });
    expect(validateCalendar(partial)["window-0"]).toMatch(/HH:MM/);
  });
});
