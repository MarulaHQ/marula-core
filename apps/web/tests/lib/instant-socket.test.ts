// InstantSocket — refcounted subscriptions, reconnect, frame dispatch.
import { describe, expect, it, vi } from "vitest";
import { EventBus } from "../../lib/instant/event-bus";
import { InstantSocket, type SocketFactory, type WireSocket } from "../../lib/instant/socket";

interface FakeWire {
  url: string;
  sent: string[];
  closed: boolean;
  open(): void;
  message(raw: string): void;
  close(): void;
  send(data: string): void;
  addEventListener(type: "open" | "message" | "close", fn: (ev?: unknown) => void): void;
  removeEventListener(type: "open" | "message" | "close", fn: (ev?: unknown) => void): void;
  listeners: Map<string, ((ev?: unknown) => void)[]>;
}

function fakeFactory(created: FakeWire[]): SocketFactory {
  return (url: string) => {
    const listeners = new Map<string, ((ev?: unknown) => void)[]>();
    const wire: FakeWire = {
      url,
      sent: [],
      closed: false,
      listeners,
      open: () => listeners.get("open")?.forEach((fn) => fn()),
      message: (raw: string) =>
        listeners.get("message")?.forEach((fn) => fn({ data: raw })),
      close: () => {
        wire.closed = true;
        listeners.get("close")?.forEach((fn) => fn());
      },
      send(data: string) {
        wire.sent.push(data);
      },
      addEventListener(type: "open" | "message" | "close", fn: () => void) {
        const list = listeners.get(type) ?? [];
        list.push(fn);
        listeners.set(type, list);
      },
      removeEventListener(type: "open" | "message" | "close", fn: () => void) {
        listeners.set(
          type,
          (listeners.get(type) ?? []).filter((l) => l !== fn),
        );
      },
    };
    created.push(wire);
    return wire as unknown as WireSocket;
  };
}

const TICKET_EVENT = JSON.stringify({
  type: "event",
  channel: "board:b1",
  event: {
    id: "e1",
    type: "ticket.created",
    entity: "ticket",
    entity_id: "t1",
    occurred_at: "2026-08-30T10:00:00Z",
    payload: { ticket_id: "t1", board_id: "b1" },
  },
});

describe("InstantSocket", () => {
  it("connects lazily on first subscribe and sends the frame after open", () => {
    const wires: FakeWire[] = [];
    const bus = new EventBus();
    const socket = new InstantSocket({
      url: () => "ws://test/ws?token=x",
      bus,
      factory: fakeFactory(wires),
    });
    const off = socket.subscribe("board:b1");
    expect(wires).toHaveLength(1);
    // Frame waits for the socket's open event.
    expect(wires[0]?.sent).toEqual([]);
    wires[0]?.open();
    expect(wires[0]?.sent).toEqual([
      JSON.stringify({ type: "subscribe", channel: "board:b1" }),
    ]);
    off();
    wires[0]?.close();
    socket.stop();
  });

  it("refcounts channels: two subscribers, one frame; unsubscribe on last release", () => {
    const wires: FakeWire[] = [];
    const bus = new EventBus();
    const socket = new InstantSocket({
      url: () => "ws://test/ws",
      bus,
      factory: fakeFactory(wires),
    });
    const offA = socket.subscribe("board:b1");
    const offB = socket.subscribe("board:b1");
    wires[0]?.open();
    expect(wires[0]?.sent).toHaveLength(1);
    offA();
    expect(wires[0]?.sent).toHaveLength(1);
    offB();
    expect(wires[0]?.sent).toHaveLength(2);
    expect(JSON.parse(wires[0]?.sent[1] ?? "{}")).toEqual({
      type: "unsubscribe",
      channel: "board:b1",
    });
    wires[0]?.close();
    socket.stop();
  });

  it("dispatches parsed event frames onto the bus", () => {
    const wires: FakeWire[] = [];
    const bus = new EventBus();
    const seen: string[] = [];
    bus.subscribe((e) => seen.push(`${e.entity}:${e.entity_id}:${e.channel}`));
    const socket = new InstantSocket({
      url: () => "ws://test/ws",
      bus,
      factory: fakeFactory(wires),
    });
    socket.subscribe("board:b1");
    wires[0]?.message(TICKET_EVENT);
    expect(seen).toEqual(["ticket:t1:board:b1"]);
    wires[0]?.close();
    socket.stop();
  });

  it("reconnects with resubscription when the wire closes", () => {
    vi.useFakeTimers();
    try {
      const wires: FakeWire[] = [];
      const bus = new EventBus();
      const socket = new InstantSocket({
        url: () => "ws://test/ws",
        bus,
        factory: fakeFactory(wires),
        maxBackoffMs: 1000,
      });
      socket.subscribe("board:b1");
      wires[0]?.open();
      wires[0]?.close();
      // First backoff: 2 ** 1 seconds capped at maxBackoffMs.
      vi.advanceTimersByTime(1000);
      expect(wires).toHaveLength(2);
      wires[1]?.open();
      expect(JSON.parse(wires[1]?.sent[0] ?? "{}")).toEqual({
        type: "subscribe",
        channel: "board:b1",
      });
      wires[1]?.close();
      socket.stop();
    } finally {
      vi.useRealTimers();
    }
  });

  it("notifies status listeners of connection transitions", () => {
    const wires: FakeWire[] = [];
    const bus = new EventBus();
    const socket = new InstantSocket({
      url: () => "ws://test/ws",
      bus,
      factory: fakeFactory(wires),
    });
    const statuses: string[] = [];
    socket.onStatus((s) => statuses.push(s));
    // onStatus replays the current state on attach.
    expect(statuses).toEqual(["closed"]);
    const off = socket.subscribe("board:b1");
    wires[0]?.open();
    wires[0]?.close();
    expect(statuses).toEqual(["closed", "connecting", "open", "closed"]);
    off();
    wires[0]?.close();
    socket.stop();
  });
});
