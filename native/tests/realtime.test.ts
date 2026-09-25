// Realtime client invariants: protocol frames, resubscription after
// reconnect, event parsing, heartbeat-driven recovery, and backoff.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { EventEnvelope } from "@/core/realtime";
import { RealtimeClient } from "@/core/realtime";

import { FakeSocketFactory } from "./helpers";

const envelope: EventEnvelope = {
  id: "e1",
  type: "ticket.updated",
  org_id: "org-1",
  entity: "ticket",
  entity_id: "t1",
  occurred_at: "2026-09-25T00:00:00Z",
  payload_schema_ref: "ticket.updated/1",
  payload: { ticket_id: "t1" },
};

function client(factory: FakeSocketFactory, token: string | null = "tok") {
  return new RealtimeClient({
    url: () => "ws://localhost:8081/ws",
    token: () => token,
    socketFactory: (url) => {
      const socket = factory.create();
      socketUrl = url;
      return socket;
    },
    baseDelayMs: 100,
    maxDelayMs: 1000,
    heartbeatMs: 1000,
    silentForMs: 2000,
    random: () => 0,
  });
}

let socketUrl = "";

describe("RealtimeClient", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    socketUrl = "";
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("connects with the token in the query and reports open after hello", async () => {
    const factory = new FakeSocketFactory();
    const rt = client(factory);
    const statuses: string[] = [];
    rt.onStatus((status) => statuses.push(status));
    void rt.connect();
    expect(socketUrl).toBe("ws://localhost:8081/ws?token=tok");
    factory.latest().open();
    factory.latest().receive({ type: "hello", user_id: "u1", org_id: "o1" });
    expect(rt.getStatus()).toBe("open");
    expect(statuses).toEqual(["connecting", "open"]);
  });

  it("subscribes before open and replays subscriptions after reconnect", async () => {
    const factory = new FakeSocketFactory();
    const rt = client(factory);
    rt.subscribe("board:b1");
    void rt.connect();
    await vi.runOnlyPendingTimersAsync();
    factory.latest().open();
    expect(factory.latest().frames()).toEqual([
      { type: "subscribe", channel: "board:b1" },
    ]);
    factory.latest().receive({ type: "subscribed", channel: "board:b1" });

    factory.latest().serverClose();
    expect(rt.getStatus()).toBe("reconnecting");
    await vi.advanceTimersByTimeAsync(120);
    expect(factory.sockets).toHaveLength(2);
    factory.latest().open();
    expect(factory.latest().frames()[0]).toEqual({ type: "subscribe", channel: "board:b1" });
  });

  it("delivers typed event frames to listeners", async () => {
    const factory = new FakeSocketFactory();
    const rt = client(factory);
    const seen: string[] = [];
    rt.onEvent((event, channel) => seen.push(`${channel}:${event.type}`));
    void rt.connect();
    factory.latest().open();
    factory.latest().receive({ type: "event", channel: "user:u1", event: envelope });
    factory.latest().receive({ type: "event", channel: "board:b1", event: envelope });
    factory.latest().receive({ type: "pong" });
    expect(seen).toEqual(["user:u1:ticket.updated", "board:b1:ticket.updated"]);
  });

  it("does not reconnect after an intentional close", async () => {
    const factory = new FakeSocketFactory();
    const rt = client(factory);
    void rt.connect();
    factory.latest().open();
    rt.close();
    await vi.advanceTimersByTimeAsync(500);
    expect(factory.sockets).toHaveLength(1);
    expect(rt.getStatus()).toBe("closed");
  });

  it("forces a reconnect when the link goes silent", async () => {
    const factory = new FakeSocketFactory();
    const rt = client(factory);
    void rt.connect();
    factory.latest().open();
    await vi.advanceTimersByTimeAsync(1100);
    expect(factory.latest().frames()).toContainEqual({ type: "ping" });
    await vi.advanceTimersByTimeAsync(2100);
    expect(factory.sockets.length).toBeGreaterThan(1);
  });

  it("stays idle without a token", async () => {
    const factory = new FakeSocketFactory();
    const rt = client(factory, null);
    await rt.connect();
    expect(factory.sockets).toHaveLength(0);
    expect(rt.getStatus()).toBe("idle");
  });
});
