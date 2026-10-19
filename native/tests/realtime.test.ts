// Realtime client invariants: SSE frame parsing, the org-scoped stream,
// presence exclusion, the hello signal, and reconnect backoff.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { EventEnvelope } from "@/core/realtime";
import { RealtimeClient } from "@/core/realtime";

import { FakeEventSourceFactory } from "./helpers";

const envelopeFrame = {
  type: "ticket.updated",
  org_id: "o1",
  entity: "ticket",
  entity_id: "t1",
  seq: 7,
  at: "2026-09-25T00:00:00Z",
};

let sourceUrl = "";

function client(factory: FakeEventSourceFactory, token: string | null = "tok") {
  return new RealtimeClient({
    url: () => "http://localhost:8080/api/events/stream",
    token: () => token,
    factory: (url) => {
      sourceUrl = url;
      return factory.create();
    },
    maxBackoffMs: 30_000,
  });
}

describe("RealtimeClient", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    sourceUrl = "";
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("opens the stream with the token in the query and reports open", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    const statuses: string[] = [];
    rt.onStatus((status) => statuses.push(status));
    rt.connect();
    expect(sourceUrl).toBe("http://localhost:8080/api/events/stream?token=tok");
    factory.latest().open();
    expect(rt.getStatus()).toBe("open");
    expect(statuses).toEqual(["connecting", "open"]);
  });

  it("leaves a live stream alone when connect runs again", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    rt.connect();
    rt.connect();
    factory.latest().open();
    rt.connect();
    expect(factory.sources).toHaveLength(1);
  });

  it("delivers contract events on the org channel and drops presence frames", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    const seen: string[] = [];
    rt.onEvent((envelope, channel) => seen.push(`${channel}:${envelope.type}`));
    rt.connect();
    factory.latest().open();
    factory.latest().receive(envelopeFrame, "e1");
    factory.latest().receive({ type: "presence", org_id: "o1", entity: "ticket", viewers: [] });
    expect(seen).toEqual(["org:o1:ticket.updated"]);
  });

  it("carries the frame identity and the SSE id onto the envelope", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    const received: EventEnvelope[] = [];
    rt.onEvent((envelope) => received.push(envelope));
    rt.connect();
    factory.latest().open();
    factory.latest().receive(envelopeFrame, "e1");
    expect(received[0]).toMatchObject({
      id: "e1",
      type: "ticket.updated",
      org_id: "o1",
      entity: "ticket",
      entity_id: "t1",
      occurred_at: "2026-09-25T00:00:00Z",
      seq: 7,
    });
  });

  it("announces the org once per connection through onHello", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    const hellos: string[] = [];
    rt.onHello((userId, orgId) => hellos.push(`${userId}|${orgId}`));
    rt.connect();
    factory.latest().open();
    factory.latest().receive(envelopeFrame, "e1");
    factory.latest().receive(envelopeFrame, "e2");
    expect(hellos).toEqual(["|o1"]);

    factory.latest().serverClose();
    vi.advanceTimersByTime(2000);
    factory.latest().open();
    factory.latest().receive(envelopeFrame, "e3");
    expect(hellos).toEqual(["|o1", "|o1"]);
  });

  it("ignores malformed frames", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    const seen: string[] = [];
    rt.onEvent((envelope) => seen.push(envelope.type));
    rt.connect();
    factory.latest().open();
    factory.latest().receiveRaw("not json");
    factory.latest().receiveRaw(JSON.stringify({ org_id: "o1" }));
    expect(seen).toEqual([]);
  });

  it("tracks subscribed channels locally without a wire frame", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    rt.subscribe("org:o1");
    rt.subscribe("board:b1");
    rt.subscribe("board:b1");
    expect(rt.subscribedChannels()).toEqual(["org:o1", "board:b1"]);
    rt.unsubscribe("board:b1");
    expect(rt.subscribedChannels()).toEqual(["org:o1"]);
  });

  it("does not reconnect after an intentional close", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    rt.connect();
    factory.latest().open();
    rt.close();
    vi.advanceTimersByTime(60_000);
    expect(factory.sources).toHaveLength(1);
    expect(factory.latest().closed).toBe(true);
    expect(rt.getStatus()).toBe("closed");
  });

  it("leaves a transient drop to the source's own retry", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    rt.connect();
    factory.latest().open();
    factory.latest().fail();
    expect(rt.getStatus()).toBe("connecting");
    vi.advanceTimersByTime(60_000);
    expect(factory.sources).toHaveLength(1);
  });

  it("reconnects after the server closes the stream", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory);
    rt.connect();
    factory.latest().open();
    expect(factory.sources).toHaveLength(1);

    factory.latest().serverClose();
    expect(rt.getStatus()).toBe("connecting");
    vi.advanceTimersByTime(1999);
    expect(factory.sources).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(factory.sources).toHaveLength(2);
  });

  it("backs off exponentially and caps at maxBackoffMs", () => {
    const factory = new FakeEventSourceFactory();
    const rt = new RealtimeClient({
      url: () => "http://localhost:8080/api/events/stream",
      token: () => "tok",
      factory: (url) => {
        sourceUrl = url;
        return factory.create();
      },
      maxBackoffMs: 3_000,
    });
    rt.connect();
    factory.latest().open();

    factory.latest().serverClose();
    vi.advanceTimersByTime(1999);
    expect(factory.sources).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(factory.sources).toHaveLength(2);

    // attempt 2 would be 4000ms; the ceiling caps it at 3000ms.
    factory.latest().serverClose();
    vi.advanceTimersByTime(2999);
    expect(factory.sources).toHaveLength(2);
    vi.advanceTimersByTime(1);
    expect(factory.sources).toHaveLength(3);
  });

  it("stays closed without a token", () => {
    const factory = new FakeEventSourceFactory();
    const rt = client(factory, null);
    rt.connect();
    expect(factory.sources).toHaveLength(0);
    expect(rt.getStatus()).toBe("closed");
  });
});
