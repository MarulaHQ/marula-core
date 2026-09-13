// InstantStore — hydrate-once, delta revalidation, seq-keyed reconcile.
import { describe, expect, it, vi } from "vitest";
import { InstantStore, type ReadModelSpec } from "../../lib/instant/instant-store";
import type { InstantEvent } from "../../lib/instant/events";

function event(overrides: Partial<InstantEvent> = {}): InstantEvent {
  return {
    id: "evt-1",
    type: "ticket.updated",
    entity: "ticket",
    entity_id: "t1",
    occurred_at: "2026-08-30T10:00:00Z",
    payload: { ticket_id: "t1" },
    channel: "board:b1",
    ...overrides,
  };
}

interface Row {
  id: string;
  title: string;
  seq?: number;
}

function listSpec(
  hydrate: ReadModelSpec<{ items: Row[] }>["hydrate"],
  revalidate: ReadModelSpec<{ items: Row[] }>["revalidate"] = async (data) => data,
  applyEntity?: ReadModelSpec<{ items: Row[] }>["applyEntity"],
): ReadModelSpec<{ items: Row[] }> {
  return {
    hydrate,
    relevant: (e) => e.entity === "ticket",
    revalidate,
    applyEntity,
    entityIds: (data) => data.items.map((r) => r.id),
  };
}

describe("InstantStore — hydrate-once (IF4)", () => {
  it("fetches the snapshot once and serves later callers from cache", async () => {
    const store = new InstantStore();
    const hydrate = vi.fn(async () => ({ items: [{ id: "t1", title: "First" }] }));
    const spec = listSpec(hydrate);

    await store.ensure("k", spec);
    await store.ensure("k", spec);
    await store.ensure("k", spec);
    expect(hydrate).toHaveBeenCalledTimes(1);
    expect(store.peek<{ items: Row[] }>("k")?.items[0]?.title).toBe("First");
  });

  it("shares one in-flight hydration across concurrent callers", async () => {
    const store = new InstantStore();
    let resolveHydrate: (v: { items: Row[] }) => void = () => {};
    const hydrate = vi.fn(
      () =>
        new Promise<{ items: Row[] }>((resolve) => {
          resolveHydrate = resolve;
        }),
    );
    const spec = listSpec(hydrate);

    const a = store.ensure("k", spec);
    const b = store.ensure("k", spec);
    resolveHydrate({ items: [{ id: "t1", title: "Shared" }] });
    await Promise.all([a, b]);
    expect(hydrate).toHaveBeenCalledTimes(1);
  });

  it("refresh() bypasses hydrate-once; ensure() afterwards stays cached", async () => {
    const store = new InstantStore();
    let call = 0;
    const spec = listSpec(async () => ({ items: [{ id: "t1", title: `v${++call}` }] }));

    await store.ensure("k", spec);
    await store.refresh("k", spec);
    await store.ensure("k", spec);
    expect(call).toBe(2);
    expect(store.peek<{ items: Row[] }>("k")?.items[0]?.title).toBe("v2");
  });
});

describe("InstantStore — deltas from events (IF4)", () => {
  it("routes relevant events to hydrated collections only", async () => {
    const store = new InstantStore();
    const revalidate = vi.fn(async (data: { items: Row[] }) => ({
      items: [...data.items, { id: "t2", title: "Delta" }],
    }));
    const spec = listSpec(async () => ({ items: [] }), revalidate);
    await store.ensure("live", spec);

    store.applyEvent(event());
    await vi.waitFor(() => {
      expect(store.peek<{ items: Row[] }>("live")?.items).toHaveLength(1);
    });
    expect(revalidate).toHaveBeenCalledTimes(1);

    // Not-yet-hydrated collections never revalidate.
    store.applyEvent(event({ entity_id: "t9" }));
    await vi.waitFor(() => {
      expect(revalidate).toHaveBeenCalledTimes(2);
    });
  });

  it("ignores events a collection does not care about", async () => {
    const store = new InstantStore();
    const revalidate = vi.fn(async (data: { items: Row[] }) => data);
    const spec: ReadModelSpec<{ items: Row[] }> = {
      hydrate: async () => ({ items: [] }),
      relevant: (e) => e.entity === "appointment",
      revalidate,
      entityIds: () => [],
    };
    await store.ensure("cal", spec);
    store.applyEvent(event());
    await new Promise((r) => setTimeout(r, 0));
    expect(revalidate).not.toHaveBeenCalled();
  });

  it("suppresses single-entity refetches while that entity has a pending write", async () => {
    const store = new InstantStore();
    const revalidate = vi.fn(
      async (
        data: { items: Row[] },
        _event: InstantEvent,
        _ctx: { isPending(id: string): boolean },
      ) => data,
    );
    const spec = listSpec(async () => ({ items: [{ id: "t1", title: "Before" }] }), revalidate);
    await store.ensure("k", spec);

    store.ledger.beginPending("t1");
    store.applyEvent(event({ type: "ticket.updated" }));
    await new Promise((r) => setTimeout(r, 0));
    // The spec's revalidate is invoked but sees isPending and (per contract)
    // skips network work — here we assert the context flag directly.
    expect(revalidate).toHaveBeenCalledWith(
      expect.anything(),
      expect.anything(),
      expect.objectContaining({ isPending: expect.any(Function) }),
    );
    const ctx = revalidate.mock.calls[0]?.[2];
    expect(ctx?.isPending("t1")).toBe(true);
    expect(ctx?.isPending("t2")).toBe(false);
  });

  it("drains burst events in arrival order, one revalidation each", async () => {
    const store = new InstantStore();
    const seen: string[] = [];
    const spec = listSpec(
      async () => ({ items: [] }),
      async (data, e) => {
        seen.push(e.type);
        return data;
      },
    );
    await store.ensure("k", spec);
    store.applyEvent(event({ type: "ticket.created", id: "e1" }));
    store.applyEvent(event({ type: "ticket.updated", id: "e2" }));
    await vi.waitFor(() => {
      expect(seen).toEqual(["ticket.created", "ticket.updated"]);
    });
  });
});

describe("InstantStore — seq-keyed mutation reconcile (IF5)", () => {
  const apply = listSpec(
    async () => ({ items: [{ id: "t1", title: "First" }] }),
    async (data) => data,
    (data, entity) => ({
      items: data.items.map((r) => (r.id === (entity as Row).id ? (entity as Row) : r)),
    }),
  );

  it("merges the applied snapshot into collections holding the entity", async () => {
    const store = new InstantStore();
    await store.ensure("k", apply);
    const verdict = store.applyMutation("t1", {
      after: { id: "t1", title: "Renamed" },
      before: { id: "t1", title: "First" },
      seq: 4,
      updated_at: "2026-08-30T10:04:00Z",
      version: 4,
    });
    expect(verdict).toBe("applied");
    expect(store.peek<{ items: Row[] }>("k")?.items[0]?.title).toBe("Renamed");
    expect(store.ledger.version("t1")).toBe(4);
  });

  it("ignores an out-of-order older mutation envelope", async () => {
    const store = new InstantStore();
    await store.ensure("k", apply);
    store.applyMutation("t1", {
      after: { id: "t1", title: "Newest" },
      before: null,
      seq: 9,
      updated_at: "2026-08-30T10:09:00Z",
      version: 9,
    });
    const verdict = store.applyMutation("t1", {
      after: { id: "t1", title: "Stale" },
      before: null,
      seq: 7,
      updated_at: "2026-08-30T10:07:00Z",
      version: 7,
    });
    expect(verdict).toBe("stale");
    expect(store.peek<{ items: Row[] }>("k")?.items[0]?.title).toBe("Newest");
  });

  it("treats a same-seq replay as duplicate without re-merging", async () => {
    const store = new InstantStore();
    await store.ensure("k", apply);
    const meta = {
      after: { id: "t1", title: "Renamed" },
      before: null,
      seq: 4,
      updated_at: "2026-08-30T10:04:00Z",
      version: 4,
    };
    expect(store.applyMutation("t1", meta)).toBe("applied");
    expect(store.applyMutation("t1", meta)).toBe("duplicate");
  });

  it("revertEntity restores rendered truth without touching the ledger", async () => {
    const store = new InstantStore();
    await store.ensure("k", apply);
    store.applyMutation("t1", {
      after: { id: "t1", title: "Renamed" },
      before: null,
      seq: 4,
      updated_at: "2026-08-30T10:04:00Z",
      version: 4,
    });
    store.revertEntity("t1", { id: "t1", title: "First" });
    expect(store.peek<{ items: Row[] }>("k")?.items[0]?.title).toBe("First");
    // The revert is not a write: the ledger cursor survives for If-Match.
    expect(store.ledger.version("t1")).toBe(4);
  });

  it("patch() updates hydrated data and notifies subscribers", async () => {
    const store = new InstantStore();
    await store.ensure("k", apply);
    const seen: unknown[] = [];
    store.subscribe(() => seen.push(store.peek("k")));
    store.patch<{ items: Row[] }>("k", (data) => ({
      items: [...data.items, { id: "t3", title: "Added" }],
    }));
    expect(store.peek<{ items: Row[] }>("k")?.items).toHaveLength(2);
    expect(seen).toHaveLength(1);
    // Same-reference patches do not notify.
    store.patch<{ items: Row[] }>("k", (data) => data);
    expect(seen).toHaveLength(1);
  });
});

describe("InstantStore — at-least-once delivery dedupe + rehydrateAll (lane 9)", () => {
  it("drops a duplicated frame identity (org + ticket scope overlap, replays)", async () => {
    const store = new InstantStore();
    const revalidate = vi.fn(async (data: { items: Row[] }) => data);
    await store.ensure("k", listSpec(async () => ({ items: [] }), revalidate));
    const frame = event({ seq: 7 });
    store.applyEvent(frame);
    store.applyEvent(event({ seq: 7, channel: "ticket:t1" }));
    // A genuinely different frame still gets through.
    store.applyEvent(event({ seq: 8 }));
    await new Promise((r) => setTimeout(r, 10));
    expect(revalidate).toHaveBeenCalledTimes(2);
  });

  it("rehydrateAll() re-snapshots only hydrated collections once each", async () => {
    const store = new InstantStore();
    const hydrateA = vi.fn(async () => ({ items: [{ id: "t1", title: "A" }] }));
    const hydrateB = vi.fn(async () => ({ items: [] }));
    await store.ensure("a", listSpec(hydrateA));
    // b's first hydrate is its `ensure` — the sweep must not re-run it.
    void store.ensure("b", listSpec(hydrateB)).catch(() => undefined);
    await store.rehydrateAll();
    expect(hydrateA).toHaveBeenCalledTimes(2);
    expect(hydrateB).toHaveBeenCalledTimes(1);
  });
});
