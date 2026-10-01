/**
 * The timer-bar rule: the bar follows the ticket being worked.
 * Lane 15 (`p41-ui-audit`) runs this as the check that the bar does not become
 * permanent chrome or vanish on a paused timer.
 */
import { describe, expect, it } from "vitest";
import { selectWorkSession } from "./work-session";

type S = { id: string; stopped_at: string | null; segments?: Array<{ started_at?: string | null }> | null };

const running = (id: string): S => ({ id, stopped_at: null, segments: [{ started_at: "2026-10-01T10:00:00Z" }] });
const paused = (id: string, startedAt: string): S => ({ id, stopped_at: null, segments: [{ started_at: startedAt }] });
const stopped = (id: string): S => ({ id, stopped_at: "2026-10-01T09:00:00Z", segments: [{ started_at: "2026-10-01T08:00:00Z" }] });

describe("selectWorkSession", () => {
  it("hides the bar when no ticket is being worked", () => {
    expect(selectWorkSession(null, [])).toBeNull();
    expect(selectWorkSession(null, null)).toBeNull();
  });

  it("never follows a stopped session", () => {
    expect(selectWorkSession(null, [stopped("t1")])).toBeNull();
  });

  it("shows the running session, and it wins over any paused one", () => {
    const r = running("run");
    expect(selectWorkSession(r, [paused("p", "2026-10-01T11:00:00Z"), r])?.id).toBe("run");
  });

  it("keeps the bar on a paused timer so it is never stranded", () => {
    expect(selectWorkSession(null, [paused("p", "2026-10-01T10:00:00Z")])?.id).toBe("p");
  });

  it("picks the most recently started open session when several are paused", () => {
    const s = selectWorkSession(null, [
      paused("old", "2026-10-01T08:00:00Z"),
      paused("new", "2026-10-01T12:00:00Z"),
      stopped("done"),
    ]);
    expect(s?.id).toBe("new");
  });
});
