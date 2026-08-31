// EntityLedger — IF5 seq reconciliation ordering.
import { describe, expect, it } from "vitest";
import { EntityLedger } from "../../lib/instant/entity-ledger";

describe("EntityLedger", () => {
  it("applies newer seqs and rejects out-of-order older ones", () => {
    const ledger = new EntityLedger();
    expect(ledger.observe("t1", { seq: 5, updatedAt: "2026-08-30T10:05:00Z" })).toBe("applied");
    expect(ledger.observe("t1", { seq: 3, updatedAt: "2026-08-30T10:03:00Z" })).toBe("stale");
    expect(ledger.observe("t1", { seq: 7 })).toBe("applied");
    expect(ledger.version("t1")).toBe(7);
  });

  it("treats same-seq observations as duplicates (idempotent replays)", () => {
    const ledger = new EntityLedger();
    ledger.observe("t1", { seq: 5, updatedAt: "2026-08-30T10:05:00Z" });
    expect(ledger.observe("t1", { seq: 5, updatedAt: "2026-08-30T10:05:00Z" })).toBe("duplicate");
    // The replayed write must never hand a stale version back for If-Match.
    expect(ledger.version("t1")).toBe(5);
  });

  it("keeps entities independent", () => {
    const ledger = new EntityLedger();
    ledger.observe("a", { seq: 10 });
    expect(ledger.observe("b", { seq: 2 })).toBe("applied");
    expect(ledger.version("a")).toBe(10);
    expect(ledger.version("b")).toBe(2);
  });

  it("falls back to updated_at ordering for seq-less reads", () => {
    const ledger = new EntityLedger();
    expect(ledger.observe("t1", { updatedAt: "2026-08-30T10:00:00Z" })).toBe("applied");
    // An older plain read cannot regress the entity.
    expect(ledger.observe("t1", { updatedAt: "2026-08-30T09:59:00Z" })).toBe("duplicate");
    expect(ledger.observe("t1", { updatedAt: "2026-08-30T10:01:00Z" })).toBe("applied");
  });

  it("accepts a newer seq whose timestamp lags the cursor", () => {
    const ledger = new EntityLedger();
    ledger.observe("t1", { seq: 5, updatedAt: "2026-08-30T10:05:00Z" });
    // seq is the authoritative order; a lagging stamp does not gate it.
    expect(ledger.observe("t1", { seq: 6, updatedAt: "2026-08-30T10:04:00Z" })).toBe("applied");
    expect(ledger.version("t1")).toBe(6);
  });

  it("returns undefined versions for never-observed entities", () => {
    const ledger = new EntityLedger();
    expect(ledger.version("ghost")).toBeUndefined();
  });

  it("tracks optimistic-write pending state", () => {
    const ledger = new EntityLedger();
    expect(ledger.isPending("t1")).toBe(false);
    ledger.beginPending("t1");
    expect(ledger.isPending("t1")).toBe(true);
    ledger.endPending("t1");
    expect(ledger.isPending("t1")).toBe(false);
    // Empty ids are never tracked (safety for comment-temp-style ids).
    ledger.beginPending("");
    expect(ledger.isPending("")).toBe(false);
  });
});
