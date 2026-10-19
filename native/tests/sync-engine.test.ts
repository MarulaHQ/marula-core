// Sync engine invariants: optimistic mutation → durable apply, offline
// queueing with reconnect flush, event-driven invalidation, and the
// version cursor from mutation envelopes.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MODELS } from "@/core/models";
import { MemoryStore } from "@/core/storage";
import { SyncEngine, invalidationFor } from "@/core/sync";
import type { EventEnvelope } from "@/core/realtime";
import type { MarulaClient } from "@marula-core/sdk";
import { startTimer } from "@/core/writes";

import { FakeApi, FakeEventSourceFactory } from "./helpers";

let counter = 0;
const uuid = () => `00000000-0000-4000-8000-${String(counter++).padStart(12, "0")}`;

function makeEngine(
  options: { online?: boolean; api?: FakeApi; factory?: FakeEventSourceFactory } = {},
) {
  const api = options.api ?? new FakeApi();
  const factory = options.factory ?? new FakeEventSourceFactory();
  const state = { online: options.online ?? true };
  const engine = new SyncEngine({
    client: api.asClient<MarulaClient>(),
    stores: {
      cache: new MemoryStore(),
      queue: new MemoryStore(),
      versions: new MemoryStore(),
    },
    realtime: {
      url: () => "http://localhost:8080/api/events/stream",
      token: () => "tok",
      factory: () => factory.create(),
    },
    isOnline: () => state.online,
    uuid,
  });
  return { engine, api, factory, state };
}

describe("SyncEngine", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("applies an online mutation through the queue and marks its caches stale", async () => {
    const { engine, api } = makeEngine();
    api.route("POST", "/api/v1/timers/start", {
      data: { id: "s1", mutation: { version: 4 } },
    });
    await engine.start();
    await engine.cache.set(MODELS.activeTimer, null);
    const result = await engine.mutate(startTimer, { ticketId: "t1" });
    expect(result.ok).toBe(true);
    expect(result.queued).toBe(false);
    expect(api.calls).toHaveLength(1);
    expect(api.calls[0]?.body).toMatchObject({ ticket_id: "t1" });
    expect(String((api.calls[0]?.body as Record<string, unknown>)["idempotency_key"])).toMatch(
      /^rn-/,
    );
    expect(engine.queue.pendingCount()).toBe(0);
    expect(engine.cache.get(MODELS.activeTimer)?.stale).toBe(true);
    expect(engine.knownVersion("timer")).toBe(4);
  });

  it("queues an offline mutation, then flushes it when connectivity returns", async () => {
    const { engine, api, state } = makeEngine({ online: false });
    api.route("POST", "/api/v1/timers/start", { data: { id: "s1", mutation: { version: 1 } } });
    await engine.start();
    const result = await engine.mutate(startTimer, { ticketId: "t1" });
    expect(result.ok).toBe(true);
    expect(result.queued).toBe(true);
    expect(engine.queue.pendingCount()).toBe(1);
    expect(api.calls).toHaveLength(0);

    state.online = true;
    await engine.handleConnectivityChange(true);
    expect(engine.queue.pendingCount()).toBe(0);
    expect(api.calls).toHaveLength(1);
  });

  it("replays the same idempotency key after a relaunch", async () => {
    const stores = {
      cache: new MemoryStore(),
      queue: new MemoryStore(),
      versions: new MemoryStore(),
    };
    const first = new SyncEngine({
      client: new FakeApi().asClient<MarulaClient>(),
      stores,
      realtime: {
        url: () => "http://localhost:8080/api/events/stream",
        token: () => "tok",
        factory: () => new FakeEventSourceFactory().create(),
      },
      isOnline: () => false,
      uuid,
    });
    await first.start();
    await first.mutate(startTimer, { ticketId: "t1" });
    expect(first.queue.pendingCount()).toBe(1);

    const api = new FakeApi();
    api.route("POST", "/api/v1/timers/start", { data: { id: "s1", mutation: { version: 1 } } });
    const second = new SyncEngine({
      client: api.asClient<MarulaClient>(),
      stores,
      realtime: {
        url: () => "http://localhost:8080/api/events/stream",
        token: () => "tok",
        factory: () => new FakeEventSourceFactory().create(),
      },
      isOnline: () => true,
      uuid,
    });
    await second.start();
    await second.catchUp();
    expect(second.queue.pendingCount()).toBe(0);
    expect(api.calls).toHaveLength(1);
  });

  it("invalidates and refetches a registered model on a live event", async () => {
    const { engine, api, factory } = makeEngine();
    api.route("GET", "/api/v1/tickets/{id}", {
      data: { id: "t1", title: "After the event" },
    });
    await engine.start();
    factory.latest().open();

    await engine.cache.set(MODELS.ticket("t1"), { id: "t1", title: "Before" });
    const detach = engine.registerModel(
      MODELS.ticket("t1"),
      async (client) => {
        const result = await client.api.GET("/api/v1/tickets/{id}", {
          params: { path: { id: "t1" } },
        });
        return result.data;
      },
    );

    factory.latest().receive(
      {
        type: "ticket.updated",
        org_id: "o1",
        entity: "ticket",
        entity_id: "t1",
        seq: 1,
        at: "2026-09-25T00:00:00Z",
      },
      "e1",
    );
    await vi.waitFor(() => {
      expect(engine.getModelState(MODELS.ticket("t1")).data).toEqual({
        id: "t1",
        title: "After the event",
      });
    });
    expect(engine.getModelState(MODELS.ticket("t1")).stale).toBe(false);
    detach();
  });

  it("tracks the org and board channels for live routing", async () => {
    const { engine, api, factory } = makeEngine();
    api.route("GET", "/api/v1/tickets/my-work", {
      data: [{ id: "t1", board_id: "b1", number: "MAR-1", title: "x" }],
    });
    await engine.start();
    factory.latest().open();
    // The org scope is learned from the stream's first frame, which is the
    // SSE stand-in for a connection hello.
    factory.latest().receive({ type: "presence", org_id: "o1", entity: "ticket", viewers: [] });
    await engine.refresh(MODELS.myWork, async (client) => {
      const result = await client.api.GET("/api/v1/tickets/my-work");
      return result.data as never;
    });
    expect(engine.realtime.subscribedChannels()).toEqual(["org:o1", "board:b1"]);
  });
});

describe("invalidationFor", () => {
  function envelope(entity: string, payload: Record<string, unknown>): EventEnvelope {
    return {
      id: "e",
      type: `${entity}.changed`,
      org_id: "o",
      entity,
      entity_id: null,
      occurred_at: "2026-09-25T00:00:00Z",
      payload,
    };
  }

  it("maps ticket comments onto detail, comments, and my-work", () => {
    const { keys } = invalidationFor(envelope("ticket_comment", { ticket_id: "t9" }));
    expect(keys).toContain(MODELS.ticketComments("t9"));
    expect(keys).toContain(MODELS.ticket("t9"));
    expect(keys).toContain(MODELS.myWork);
  });

  it("maps notifications onto the inbox and badge", () => {
    const { keys } = invalidationFor(envelope("notification", { user_id: "u1" }));
    expect(keys).toEqual([MODELS.notifications, MODELS.unreadCount]);
  });

  it("maps appointments onto dispatch-day prefixes", () => {
    const { prefixes } = invalidationFor(envelope("appointment", {}));
    expect(prefixes).toContain("dispatch.day:");
  });

  it("marks nothing for an unknown entity", () => {
    const { keys, prefixes } = invalidationFor(envelope("mystery", {}));
    expect(keys).toEqual([]);
    expect(prefixes).toEqual([]);
  });
});
