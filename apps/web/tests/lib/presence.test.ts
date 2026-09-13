// Presence roster logic — TTL aging, honest idle states, editing window.
import { describe, expect, it } from "vitest";
import { isEditing, PRESENCE_TTL_MS, rosterView } from "../../lib/instant/presence";
import { parseStreamFrame } from "../../lib/instant/events";

const NOW = Date.parse("2026-09-13T10:00:00Z");

describe("rosterView", () => {
  it("classifies editing, viewing, and TTL-past idle viewers", () => {
    const viewers = rosterView(
      [
        { user_id: "u1", name: "Ada", at: "2026-09-13T09:59:45Z", editing: true },
        { user_id: "u2", name: "Grace", at: "2026-09-13T09:59:50Z", editing: false },
        { user_id: "u3", name: "Linus", at: "2026-09-13T09:59:10Z", editing: false },
        { user_id: "u4", name: "Gone", at: new Date(NOW - PRESENCE_TTL_MS).toISOString(), editing: true },
      ],
      NOW,
    );
    expect(viewers.map((v) => v.state)).toEqual(["editing", "viewing", "viewing", "idle"]);
    // A past-TTL viewer reads as idle even if its editing flag was set —
    // honest aging, never a fake active state.
    expect(viewers[3]!.editing).toBe(true);
  });
});

describe("isEditing", () => {
  it("expires the editing window", () => {
    expect(isEditing(NOW + 1000, NOW)).toBe(true);
    expect(isEditing(NOW - 1, NOW)).toBe(false);
  });
});

describe("presence frames parse as their own kind", () => {
  it("keeps presence out of the contract-event shape", () => {
    const raw = JSON.stringify({
      type: "presence",
      org_id: "o",
      entity: "ticket",
      entity_id: "t1",
      at: "2026-09-13T10:00:00Z",
      viewers: [{ user_id: "u1", name: "Ada", at: "2026-09-13T09:59:31Z", editing: false }],
    });
    const parsed = parseStreamFrame(raw, "ticket:t1");
    expect(parsed?.kind).toBe("presence");
    if (parsed?.kind === "presence") {
      expect(parsed.viewers[0]!.name).toBe("Ada");
      expect(parsed.entity_id).toBe("t1");
    }
    const event = parseStreamFrame(
      JSON.stringify({ type: "ticket.updated", org_id: "o", entity: "ticket", entity_id: "t1", seq: 1, at: "2026-09-13T10:00:00Z" }),
      "ticket:t1",
    );
    expect(event?.kind).toBe("event");
  });
});
