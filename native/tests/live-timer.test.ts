// Live timer snapshot — the one mapping every glanceable surface renders.
//
// These tests pin the fold: running/paused, banked seconds, segment anchor,
// and the wire payload both native bridges parse.

import { describe, expect, it } from "vitest";

import type { TimerSession } from "@/core/models";
import {
  liveTimerPayload,
  liveTimerSnapshot,
  sameLiveTimerSnapshot,
} from "@/live/timer-snapshot";

function session(segments: { start: number; end?: number }[], startedAt: number): TimerSession {
  return {
    id: "11111111-1111-1111-1111-111111111111",
    org_id: "22222222-2222-2222-2222-222222222222",
    ticket_id: "33333333-3333-3333-3333-333333333333",
    user_id: "44444444-4444-4444-4444-444444444444",
    started_at: new Date(startedAt).toISOString(),
    stopped_at: null,
    segments: segments.map((segment) => ({
      started_at: new Date(segment.start).toISOString(),
      ...(segment.end !== undefined ? { ended_at: new Date(segment.end).toISOString() } : {}),
    })),
  } as TimerSession;
}

describe("liveTimerSnapshot", () => {
  it("folds an open segment into running with a ticking anchor", () => {
    const now = 10_000_000;
    const snapshot = liveTimerSnapshot(
      session([{ start: now - 90_000 }], now - 90_000),
      { ticketNumber: "DEMO-1001", title: "Comms room" },
      now,
    );
    expect(snapshot.state).toBe("running");
    expect(snapshot.segmentStartedAtIso).toBe(new Date(now - 90_000).toISOString());
    expect(snapshot.accumulatedSeconds).toBe(0);
    expect(snapshot.totalSeconds).toBeCloseTo(90, 0);
    expect(snapshot.ticketNumber).toBe("DEMO-1001");
  });

  it("banks earlier segments and anchors on the current one", () => {
    const now = 20_000_000;
    const snapshot = liveTimerSnapshot(
      session(
        [
          { start: now - 600_000, end: now - 300_000 },
          { start: now - 60_000 },
        ],
        now - 600_000,
      ),
      {},
      now,
    );
    expect(snapshot.state).toBe("running");
    expect(snapshot.accumulatedSeconds).toBe(300);
    expect(snapshot.totalSeconds).toBeCloseTo(360, 0);
    expect(snapshot.segmentStartedAtIso).toBe(new Date(now - 60_000).toISOString());
  });

  it("reports paused when the last segment closed, total frozen", () => {
    const now = 30_000_000;
    const snapshot = liveTimerSnapshot(
      session(
        [
          { start: now - 600_000, end: now - 420_000 },
          { start: now - 300_000, end: now - 60_000 },
        ],
        now - 600_000,
      ),
      { ticketNumber: "DEMO-1002" },
      now,
    );
    expect(snapshot.state).toBe("paused");
    expect(snapshot.accumulatedSeconds).toBe(180 + 240);
    expect(snapshot.totalSeconds).toBe(180 + 240);
  });

  it("carries the ISO + epoch pair both native bridges parse", () => {
    const now = 40_000_000;
    const snapshot = liveTimerSnapshot(session([{ start: now - 5_000 }], now - 5_000), {}, now);
    const payload = liveTimerPayload(snapshot);
    expect(payload.segmentStartedAt).toBe(snapshot.segmentStartedAtIso);
    expect(payload.segmentStartedAtMillis).toBe(Date.parse(snapshot.segmentStartedAtIso));
    expect(payload.updatedAt).toBe(snapshot.updatedAtIso);
    expect(payload.sessionId).toBe(snapshot.sessionId);
  });

  it("treats sub-second drift as the same snapshot but a pause as a change", () => {
    const now = 50_000_000;
    const a = liveTimerSnapshot(session([{ start: now - 10_000 }], now - 10_000), {}, now);
    const b = liveTimerSnapshot(session([{ start: now - 10_000 }], now - 10_000), {}, now + 200);
    const paused = liveTimerSnapshot(
      session([{ start: now - 10_000, end: now - 1_000 }], now - 10_000),
      {},
      now,
    );
    expect(sameLiveTimerSnapshot(a, b)).toBe(true);
    expect(sameLiveTimerSnapshot(a, paused)).toBe(false);
    expect(sameLiveTimerSnapshot(null, null)).toBe(true);
    expect(sameLiveTimerSnapshot(a, null)).toBe(false);
  });
});
