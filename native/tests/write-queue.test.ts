// Offline write queue invariants — the replay-safety proof.
//
// Covers the binding lane requirements: per-entity FIFO ordering across
// relaunch, idempotency keys stable across replay, transient/offline ops
// staying durable and re-arming, last-write conflict resolution with an
// audited version adoption, typed rejection surfacing (never dropped), and
// idempotent-replay dedupe.

import { describe, expect, it } from "vitest";

import { WriteQueue, type FlushReport, type QueueOpInput, type TransportFailure } from "@/core/queue";
import { MemoryStore } from "@/core/storage";
import type { FailureKind } from "@/core/errors";

let counter = 0;
const uuid = () => `00000000-0000-4000-8000-${String(counter++).padStart(12, "0")}`;

function input(overrides: Partial<QueueOpInput> = {}): QueueOpInput {
  const idempotencyKey = overrides.idempotencyKey ?? `key-${counter++}`;
  return {
    kind: "ticket.add_note",
    entityKey: "ticket:t1",
    versionKey: "ticket:t1",
    label: "Note on ticket",
    method: "POST",
    path: "/api/v1/tickets/t1/comments",
    body: { body: "hello", idempotency_key: idempotencyKey },
    idempotencyKey,
    invalidate: ["ticket:t1:comments"],
    ...overrides,
  };
}

function failure(overrides: Partial<TransportFailure> = {}): TransportFailure {
  return {
    ok: false,
    kind: "http" as FailureKind["kind"],
    message: "Someone else updated this.",
    transient: false,
    conflict: false,
    ...overrides,
  };
}

describe("WriteQueue", () => {
  it("keeps per-entity FIFO order and lets other entities proceed", async () => {
    const store = new MemoryStore();
    const sent: string[] = [];
    const queue = new WriteQueue({
      store,
      uuid,
      send: async (op) => {
        sent.push(`${op.entityKey}:${String(op.body["seq"])}`);
        return { ok: true, data: {} };
      },
      resolveConflict: async () => ({ retry: false, detail: "no" }),
    });
    await queue.enqueue(input({ body: { seq: 1 }, entityKey: "ticket:a" }));
    await queue.enqueue(input({ body: { seq: 2 }, entityKey: "ticket:a" }));
    await queue.enqueue(input({ body: { seq: 3 }, entityKey: "ticket:b" }));
    const report = await queue.flush();
    expect(sent).toEqual(["ticket:a:1", "ticket:a:2", "ticket:b:3"]);
    expect(report.applied).toBe(3);
    expect(queue.pendingCount()).toBe(0);
  });

  it("blocks later ops for a failing entity but not other entities", async () => {
    const queue = new WriteQueue({
      store: new MemoryStore(),
      uuid,
      send: async (op) => {
        if (op.entityKey === "ticket:a") {
          return failure({ reason: "validation_failed", message: "Missing details." });
        }
        return { ok: true, data: {} };
      },
      resolveConflict: async () => ({ retry: false, detail: "no" }),
    });
    await queue.enqueue(input({ entityKey: "ticket:a", body: { seq: 1 } }));
    await queue.enqueue(input({ entityKey: "ticket:a", body: { seq: 2 } }));
    await queue.enqueue(input({ entityKey: "ticket:b", body: { seq: 3 } }));
    const report = await queue.flush();
    expect(report.rejected).toBe(1);
    expect(report.applied).toBe(1);
    expect(queue.rejected()).toHaveLength(1);
    expect(queue.pending()).toHaveLength(1);
    expect(queue.rejected()[0]?.failure?.reason).toBe("validation_failed");
  });

  it("survives relaunch: pending ops replay with the SAME idempotency key", async () => {
    const store = new MemoryStore();
    const key = "stable-key-1";
    const first = new WriteQueue({
      store,
      uuid,
      send: async () => failure({ kind: "offline", message: "offline", transient: true }),
      resolveConflict: async () => ({ retry: false, detail: "no" }),
    });
    await first.enqueue(input({ idempotencyKey: key, body: { body: "x", idempotency_key: key } }));
    await first.flush();
    expect(first.pendingCount()).toBe(1);

    const replayed: Record<string, unknown>[] = [];
    const second = new WriteQueue({
      store,
      uuid,
      send: async (op) => {
        replayed.push(op.body);
        return { ok: true, data: {} };
      },
      resolveConflict: async () => ({ retry: false, detail: "no" }),
    });
    await second.hydrate();
    expect(second.pendingCount()).toBe(1);
    await second.flush();
    expect(replayed).toHaveLength(1);
    expect(replayed[0]?.["idempotency_key"]).toBe(key);
    expect(second.pendingCount()).toBe(0);
  });

  it("resolves a version conflict by adopting the server's current version (last-write, audited)", async () => {
    const bodies: Record<string, unknown>[] = [];
    const queue = new WriteQueue({
      store: new MemoryStore(),
      uuid,
      send: async (op) => {
        bodies.push(op.body);
        if (op.body["version"] === 3) {
          return failure({
            reason: "version_conflict",
            message: "version mismatch",
            conflict: true,
            details: { expected: 7, provided: 3 },
          });
        }
        return { ok: true, data: { mutation: { version: 8 } } };
      },
      resolveConflict: async (_op, err) => ({
        retry: true,
        version: err.details?.["expected"] as number,
        detail: "adopted the server's current version",
      }),
    });
    const op = await queue.enqueue(input({ version: 3, body: { note: "x", version: 3 } }));
    const report = await queue.flush();
    expect(report.applied).toBe(1);
    expect(bodies[1]?.["version"]).toBe(7);
    expect(op.audit.map((entry) => entry.kind)).toContain("conflict_resolved");
    const resolved = queue.appliedThisSession().find((candidate) => candidate.id === op.id);
    expect(resolved?.audit.some((entry) => entry.detail?.includes("version 3 -> 7"))).toBe(true);
  });

  it("rejects after a second conflict instead of merging silently", async () => {
    const queue = new WriteQueue({
      store: new MemoryStore(),
      uuid,
      send: async () =>
        failure({
          reason: "version_conflict",
          message: "version mismatch",
          conflict: true,
          details: { expected: 9 },
        }),
      resolveConflict: async (_op, err) => ({
        retry: true,
        version: err.details?.["expected"] as number,
        detail: "adopted",
      }),
    });
    await queue.enqueue(input({ version: 1 }));
    const report = await queue.flush();
    expect(report.rejected).toBe(1);
    expect(queue.rejected()[0]?.failure?.reason).toBe("version_conflict");
  });

  it("treats idempotency_key_reused as an applied dedupe", async () => {
    const callbacks: string[] = [];
    const queue = new WriteQueue({
      store: new MemoryStore(),
      uuid,
      send: async () =>
        failure({ reason: "idempotency_key_reused", message: "already", transient: false }),
      resolveConflict: async () => ({ retry: false, detail: "no" }),
      onSettled: (op) => {
        callbacks.push(op.id);
      },
    });
    const op = await queue.enqueue(input());
    const report = await queue.flush();
    expect(report.applied).toBe(1);
    expect(report.rejected).toBe(0);
    expect(callbacks).toEqual([op.id]);
    expect(op.audit.some((entry) => entry.detail?.includes("idempotent replay"))).toBe(true);
  });

  it("exhausts its retry budget while offline, then re-arms on user retry", async () => {
    let online = false;
    const queue = new WriteQueue({
      store: new MemoryStore(),
      uuid,
      maxTransientAttempts: 2,
      send: async () =>
        online
          ? { ok: true, data: {} }
          : failure({ kind: "offline", message: "offline", transient: true }),
      resolveConflict: async () => ({ retry: false, detail: "no" }),
    });
    const op = await queue.enqueue(input());
    await queue.flush();
    expect(queue.pendingCount()).toBe(1);
    await queue.flush();
    expect(queue.rejected()).toHaveLength(1);
    online = true;
    await queue.retryRejected(op.id);
    const report = await queue.flush();
    expect(report.applied).toBe(1);
  });

  it("keeps rejected ops visible until the user drops them", async () => {
    const queue = new WriteQueue({
      store: new MemoryStore(),
      uuid,
      send: async () => failure({ reason: "forbidden", message: "no permission" }),
      resolveConflict: async () => ({ retry: false, detail: "no" }),
    });
    const op = await queue.enqueue(input());
    await queue.flush();
    expect(queue.rejected()).toHaveLength(1);
    await queue.drop(op.id);
    expect(queue.rejected()).toHaveLength(0);
    expect(queue.snapshot()).toHaveLength(0);
  });

  it("clear() drops queued work at session end", async () => {
    const queue = new WriteQueue({
      store: new MemoryStore(),
      uuid,
      send: async () => failure({ kind: "offline", message: "offline", transient: true }),
      resolveConflict: async () => ({ retry: false, detail: "no" }),
    });
    await queue.enqueue(input());
    await queue.enqueue(input({ entityKey: "ticket:b" }));
    expect(queue.pendingCount()).toBe(2);
    await queue.clear();
    expect(queue.pendingCount()).toBe(0);
  });

  it("single-flights concurrent flush calls", async () => {
    let sends = 0;
    const queue = new WriteQueue({
      store: new MemoryStore(),
      uuid,
      send: async () => {
        sends += 1;
        return { ok: true, data: {} };
      },
      resolveConflict: async () => ({ retry: false, detail: "no" }),
    });
    await queue.enqueue(input());
    const [a, b]: FlushReport[] = await Promise.all([queue.flush(), queue.flush()]);
    expect(sends).toBe(1);
    expect(b).toBe(a);
    expect(a?.applied).toBe(1);
  });
});
