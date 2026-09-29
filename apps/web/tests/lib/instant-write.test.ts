// executeInstantWrite — optimistic render → authoritative reconcile /
// typed rejection revert / durable offline handoff (IF1+IF5+IF6 client).
import { describe, expect, it, vi } from "vitest";
import { InstantStore } from "../../lib/instant/instant-store";
import { OfflineQueue } from "../../lib/instant/offline-queue";
import {
  executeInstantWrite,
  type InstantTransport,
  type InstantTransportResult,
} from "../../lib/instant/instant-write";
import type { ReadModelSpec } from "../../lib/instant/instant-store";

interface Model {
  items: { id: string; title: string; status: string }[];
}

const spec: ReadModelSpec<Model> = {
  hydrate: async () => ({ items: [{ id: "t1", title: "First", status: "open" }] }),
  relevant: () => false,
  revalidate: async (data) => data,
  applyEntity: (data, entity) => ({
    items: data.items.map((r) =>
      r.id === (entity as { id: string }).id
        ? { ...r, ...(entity as { id: string; title?: string; status?: string }) }
        : r,
    ),
  }),
  entityIds: (data) => data.items.map((r) => r.id),
};

const META = (seq: number, title: string) => ({
  after: { id: "t1", title, status: "in_progress" },
  before: { id: "t1", title, status: "open" },
  seq,
  updated_at: `2026-08-30T10:0${seq}:00Z`,
  version: seq,
  idempotency_key: "key-1",
});

async function hydratedStore(): Promise<InstantStore> {
  const store = new InstantStore();
  await store.ensure("k", spec);
  return store;
}

const NOOP_QUEUE = () =>
  new OfflineQueue({
    storage: {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {},
    },
    invoke: async () => ({ outcome: "accepted" as const }),
  });

describe("executeInstantWrite — accepted path", () => {
  it("renders optimistically before the wire and reconciles from the envelope", async () => {
    const store = await hydratedStore();
    const queue = NOOP_QUEUE();
    const optimistic = vi.fn(() => {
      store.patch<Model>("k", (m) => ({
        items: m.items.map((r) => (r.id === "t1" ? { ...r, status: "in_progress" } : r)),
      }));
    });
    const transport: InstantTransport = {
      request: vi.fn(async (): Promise<InstantTransportResult> => ({
        outcome: "accepted",
        mutation: META(4, "First"),
      })),
    };

    store.ledger.observe("t1", { seq: 3, updatedAt: "2026-08-30T10:03:00Z" });
    const result = await executeInstantWrite(store, queue, transport, {
      describe: "Set status In Progress on MAR-1",
      entityId: "t1",
      method: "POST",
      path: "/api/v1/tickets/t1/status",
      buildBody: (base) => ({ status_id: "s2", ...base }),
      optimistic,
    });

    expect(optimistic).toHaveBeenCalledTimes(1);
    expect(result.status).toBe("accepted");
    // The authoritative snapshot patched the collection through the ledger.
    expect(store.peek<Model>("k")?.items[0]?.status).toBe("in_progress");
    expect(store.ledger.version("t1")).toBe(4);
    // Pending write cleared: entity refetches flow again.
    expect(store.ledger.isPending("t1")).toBe(false);
    // Nothing durable queued.
    expect(queue.pendingCount).toBe(0);
  });

  it("attaches the idempotency key and the observed If-Match version", async () => {
    const store = await hydratedStore();
    store.ledger.observe("t1", { seq: 6 });
    let captured: unknown;
    const transport: InstantTransport = {
      request: async (_method, _path, body) => {
        captured = body;
        return { outcome: "accepted", mutation: META(7, "First") };
      },
    };
    await executeInstantWrite(store, NOOP_QUEUE(), transport, {
      describe: "x",
      entityId: "t1",
      method: "POST",
      path: "/api/v1/tickets/t1/status",
      buildBody: (base) => ({ status_id: "s2", ...base }),
    });
    expect(captured).toMatchObject({
      status_id: "s2",
      idempotency_key: expect.any(String),
      version: 6,
    });
  });
});

describe("executeInstantWrite — typed rejection", () => {
  it("reverts the optimistic patch and surfaces the IF3 reason", async () => {
    const store = await hydratedStore();
    const revert = vi.fn(() => {
      store.patch<Model>("k", (m) => ({
        items: m.items.map((r) => (r.id === "t1" ? { ...r, status: "open" } : r)),
      }));
    });
    const transport: InstantTransport = {
      request: async () => ({
        outcome: "rejected",
        reason: "version_conflict",
        message: "entity moved from version 3 to 9",
      }),
    };
    const result = await executeInstantWrite(store, NOOP_QUEUE(), transport, {
      describe: "x",
      entityId: "t1",
      method: "POST",
      path: "/api/v1/tickets/t1/status",
      buildBody: (base) => ({ status_id: "s2", ...base }),
      optimistic: () => {
        store.patch<Model>("k", (m) => ({
          items: m.items.map((r) => (r.id === "t1" ? { ...r, status: "in_progress" } : r)),
        }));
      },
      revert,
    });

    expect(result).toEqual({
      status: "rejected",
      reason: "version_conflict",
      message: "entity moved from version 3 to 9",
    });
    expect(revert).toHaveBeenCalledTimes(1);
    expect(store.peek<Model>("k")?.items[0]?.status).toBe("open");
    expect(store.ledger.isPending("t1")).toBe(false);
  });
});

describe("executeInstantWrite — offline handoff (IF6)", () => {
  it("keeps the optimistic render and enqueues the op durably", async () => {
    const store = await hydratedStore();
    const queue = new OfflineQueue({
      storage: {
        getItem: () => null,
        setItem: () => {},
        removeItem: () => {},
      },
      invoke: async () => ({ outcome: "accepted" as const }),
    });
    const transport: InstantTransport = {
      request: async () => ({ outcome: "network" }),
    };
    const result = await executeInstantWrite(store, queue, transport, {
      describe: "Set status In Progress on MAR-1",
      entityId: "t1",
      method: "POST",
      path: "/api/v1/tickets/t1/status",
      buildBody: (base) => ({ status_id: "s2", ...base }),
      optimistic: () => {
        store.patch<Model>("k", (m) => ({
          items: m.items.map((r) => (r.id === "t1" ? { ...r, status: "in_progress" } : r)),
        }));
      },
      captureBefore: () => ({ id: "t1", status: "open" }),
    });

    expect(result.status).toBe("queued");
    // Optimistic render SURVIVES while offline.
    expect(store.peek<Model>("k")?.items[0]?.status).toBe("in_progress");
    // Pending is retained: event refetches must not clobber the queued write.
    expect(store.ledger.isPending("t1")).toBe(true);
    // The queued op carries the same idempotency key + the pre-write snapshot.
    const [queued] = queue.list();
    expect(queued?.key).toEqual((result as { key: string }).key);
    expect(queued?.before).toEqual({ id: "t1", status: "open" });
    expect(queued?.entityId).toBe("t1");
  });
});
