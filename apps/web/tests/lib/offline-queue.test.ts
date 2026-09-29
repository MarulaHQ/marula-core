// OfflineQueue — IF6: durable FIFO, strict in-order replay, per-op results.
import { describe, expect, it, vi } from "vitest";
import {
  OfflineQueue,
  type InvokeFn,
  type InvokeResult,
  type QueuedWrite,
} from "../../lib/instant/offline-queue";

function memoryStorage(): Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  const map = new Map<string, string>();
  return {
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => void map.set(k, v),
    removeItem: (k) => void map.delete(k),
  };
}

function op(key: string, describe = `op ${key}`): Omit<QueuedWrite, "queuedAt"> {
  return {
    key,
    method: "POST",
    path: "/api/v1/tickets/t1/status",
    body: { idempotency_key: key, status_id: "s2" },
    describe,
    entityId: "t1",
  };
}

function queueWith(
  invoke: InvokeFn,
  storage = memoryStorage(),
): OfflineQueue {
  return new OfflineQueue({ storage, invoke, now: () => "2026-08-30T10:00:00Z" });
}

describe("OfflineQueue — durability", () => {
  it("persists enqueued ops to storage", () => {
    const storage = memoryStorage();
    const q = queueWith(async () => ({ outcome: "accepted" as const }), storage);
    q.enqueue(op("k1"));
    const raw = storage.getItem("marula-instant-outbox");
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw ?? "[]") as QueuedWrite[];
    expect(parsed).toHaveLength(1);
    expect(parsed[0]?.key).toBe("k1");
    expect(parsed[0]?.body).toEqual({ idempotency_key: "k1", status_id: "s2" });
  });

  it("survives a reload: a fresh queue over the same storage resumes", async () => {
    const storage = memoryStorage();
    const first = queueWith(async () => ({ outcome: "network" as const }), storage);
    first.enqueue(op("k1"));
    first.enqueue(op("k2", "second"));

    // "Reload": new instance, same storage; replay drains in order.
    const seen: string[] = [];
    const second = queueWith(
      async (o) => {
        seen.push(o.key);
        return { outcome: "accepted" as const };
      },
      storage,
    );
    expect(second.pendingCount).toBe(2);
    await second.replay();
    expect(seen).toEqual(["k1", "k2"]);
    expect(second.pendingCount).toBe(0);
    expect(storage.getItem("marula-instant-outbox")).toBeNull();
  });
});

describe("OfflineQueue — strict in-order replay", () => {
  it("replays FIFO and records per-op outcomes", async () => {
    const order: string[] = [];
    let n = 0;
    const q = queueWith(async (o) => {
      order.push(o.key);
      n += 1;
      return n === 1
        ? { outcome: "accepted" as const }
        : { outcome: "rejected" as const, reason: "version_conflict" };
    });
    q.enqueue(op("k1"));
    q.enqueue(op("k2"));
    const outcomes: string[] = [];
    q.onOutcome((o) => outcomes.push(`${o.key}:${o.status}`));

    await q.replay();
    expect(order).toEqual(["k1", "k2"]);
    expect(outcomes).toEqual(["k1:accepted", "k2:rejected"]);
    expect(q.pendingCount).toBe(0);
    expect(q.recentOutcomes()[1]?.reason).toBe("version_conflict");
  });

  it("stops at the first op that still cannot reach the network", async () => {
    const seen: string[] = [];
    const q = queueWith(async (o) => {
      seen.push(o.key);
      return o.key === "k1" ? { outcome: "network" as const } : { outcome: "accepted" as const };
    });
    q.enqueue(op("k1"));
    q.enqueue(op("k2"));
    await q.replay();
    // k1 stays at the head; k2 was never attempted.
    expect(seen).toEqual(["k1"]);
    expect(q.list().map((o) => o.key)).toEqual(["k1", "k2"]);
  });

  it("treats a throwing invoker as a network failure", async () => {
    const q = queueWith(async () => {
      throw new Error("fetch failed");
    });
    q.enqueue(op("k1"));
    await q.replay();
    expect(q.pendingCount).toBe(1);
  });

  it("is reentrancy-safe: overlapping triggers drain once", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const calls: string[] = [];
    const q = queueWith(async (o) => {
      calls.push(o.key);
      await gate;
      return { outcome: "accepted" as const };
    });
    q.enqueue(op("k1"));
    const a = q.replay();
    const b = q.replay();
    release();
    await Promise.all([a, b]);
    expect(calls).toEqual(["k1"]);
    expect(q.pendingCount).toBe(0);
  });
});

describe("OfflineQueue — surfacing", () => {
  it("caps the retained outcome history", async () => {
    const q = new OfflineQueue({
      storage: memoryStorage(),
      invoke: async () => ({ outcome: "accepted" as const }),
      outcomeHistory: 3,
    });
    for (const key of ["k1", "k2", "k3", "k4", "k5"]) q.enqueue(op(key));
    await q.replay();
    expect(q.recentOutcomes().map((o) => o.key)).toEqual(["k3", "k4", "k5"]);
  });

  it("notifies state listeners on enqueue and drain", async () => {
    const q = queueWith(async () => ({ outcome: "accepted" as const }));
    const updates: number[] = [];
    const off = q.onState(() => updates.push(q.pendingCount));
    q.enqueue(op("k1"));
    await q.replay();
    off();
    expect(updates).toEqual([1, 0]);
  });

  it("carries the describe label and typed reason on outcomes", async () => {
    const q = queueWith(async () => ({
      outcome: "rejected" as const,
      reason: "forbidden",
      message: "board is read-only",
    }));
    const seen: { describe: string; reason?: string }[] = [];
    q.onOutcome((o) => seen.push({ describe: o.describe, reason: o.reason }));
    q.enqueue(op("k1", "Set status Waiting on MAR-12"));
    await q.replay();
    expect(seen[0]?.describe).toBe("Set status Waiting on MAR-12");
    expect(seen[0]?.reason).toBe("forbidden");
  });
});
