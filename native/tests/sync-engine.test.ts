// Sync engine invariants: optimistic mutation → durable apply, offline
// queueing with reconnect flush, event-driven invalidation, and the
// version cursor from mutation envelopes.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MODELS } from "@/core/models";
import { MemoryStore } from "@/core/storage";
import { SyncEngine, invalidationFor } from "@/core/sync";
import type { EventEnvelope } from "@/core/realtime";
import type { MeridianClient } from "@marula-core/sdk";
import { startTimer } from "@/core/writes";

import { FakeApi, FakeSocketFactory } from "./helpers";

let counter = 0;
const uuid = () => `00000000-0000-4000-8000-${String(counter++).padStart(12, "0")}`;

function makeEngine(
  options: { online?: boolean; api?: FakeApi; factory?: FakeSocketFactory } = {},
) {
  const api = options.api ?? new FakeApi();
  const factory = options.factory ?? new FakeSocketFactory();
  const state = { online: options.online ?? true };
  const engine = new SyncEngine({
    client: api.asClient<MeridianClient>(),
    stores: {
      cache: new MemoryStore(),
      queue: new MemoryStore(),
      versions: new MemoryStore(),
    },
    realtime: {
      url: () => "ws://localhost:8081/ws",
      token: () => "tok",
      socketFactory: () => factory.create(),
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
      client: new FakeApi().asClient<MeridianClient>(),
      stores,
      realtime: {
        url: () => "ws://localhost:8081/ws",
        token: () => "tok",
        socketFactory: () => new FakeSocketFactory().create(),
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
      client: api.asClient<MeridianClient>(),
      stores,
      realtime: {
        url: () => "ws://localhost:8081/ws",
        token: () => "tok",
        socketFactory: () => new FakeSocketFactory().create(),
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
    factory.latest().receive({ type: "hello", user_id: "u1", org_id: "o1" });

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

    const envelope: EventEnvelope = {
      id: "e1",
      type: "ticket.updated",
      org_id: "o1",
      entity: "ticket",
      entity_id: "t1",
      occurred_at: "2026-09-25T00:00:00Z",
      payload_schema_ref: "ticket.updated/1",
      payload: { ticket_id: "t1" },
    };
    factory.latest().receive({ type: "event", channel: "board:b1", event: envelope });
    await vi.waitFor(() => {
      expect(engine.getModelState(MODELS.ticket("t1")).data).toEqual({
        id: "t1",
        title: "After the event",
      });
    });
    expect(engine.getModelState(MODELS.ticket("t1")).stale).toBe(false);
    detach();
  });

  it("subscribes to org and board channels for live routing", async () => {
    const { engine, api, factory } = makeEngine();
    api.route("GET", "/api/v1/tickets/my-work", {
      data: [{ id: "t1", board_id: "b1", number: "MER-1", title: "x" }],
    });
    await engine.start();
    factory.latest().open();
    factory.latest().receive({ type: "hello", user_id: "u1", org_id: "o1" });
    await engine.refresh(MODELS.myWork, async (client) => {
      const result = await client.api.GET("/api/v1/tickets/my-work");
      return result.data as never;
    });
    const frames = factory.latest().frames();
    expect(frames).toContainEqual({ type: "subscribe", channel: "org:o1" });
    expect(frames).toContainEqual({ type: "subscribe", channel: "board:b1" });
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
      payload_schema_ref: "x/1",
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
