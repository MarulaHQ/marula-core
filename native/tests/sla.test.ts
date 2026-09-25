// The shared SLA pill rule: risk only, never a neutral chip on every row.

import { describe, expect, it } from "vitest";

import { DUE_SOON_WINDOW_MS, slaState } from "@/ui/sla";

const NOW = Date.parse("2026-09-25T12:00:00Z");

function iso(offsetMs: number): string {
  return new Date(NOW + offsetMs).toISOString();
}

describe("slaState", () => {
  it("marks a past due date overdue", () => {
    expect(slaState(iso(-60_000), NOW)).toEqual({ severity: "overdue", label: "OVERDUE" });
  });

  it("marks the inside of the due-soon window", () => {
    expect(slaState(iso(DUE_SOON_WINDOW_MS - 60_000), NOW)).toEqual({
      severity: "due_soon",
      label: "DUE SOON",
    });
    expect(slaState(iso(30 * 60_000), NOW)?.severity).toBe("due_soon");
  });

  it("shows nothing at exactly the window edge or beyond it", () => {
    expect(slaState(iso(DUE_SOON_WINDOW_MS), NOW)).toBeUndefined();
    expect(slaState(iso(3 * 24 * 60 * 60 * 1000), NOW)).toBeUndefined();
  });

  it("shows nothing for missing or unparseable values", () => {
    expect(slaState(null, NOW)).toBeUndefined();
    expect(slaState(undefined, NOW)).toBeUndefined();
    expect(slaState("not-a-date", NOW)).toBeUndefined();
  });
});
