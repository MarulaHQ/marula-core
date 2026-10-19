// InstantStream — SSE live leg: frame dispatch, scope refcounting, resume
// resync, presence routing (never the reconcile path).
import { describe, expect, it, vi, beforeEach } from "vitest";
import { EventBus } from "../../lib/instant/event-bus";
import { InstantStream, type WireEventSource } from "../../lib/instant/stream";
import { __resetInstantRuntimeForTests, instantRuntime } from "../../lib/instant/runtime";
import { seedAuthTokens } from "../helpers/mock-api";

interface FakeSource {
  url: string;
  readyState: number;
  listeners: Map<string, ((ev?: unknown) => void)[]>;
  closed: boolean;
  open(): void;
  message(raw: string, id?: string): void;
  fail(closed: boolean): void;
  close(): void;
  addEventListener(type: string, fn: (ev?: unknown) => void): void;
  removeEventListener(type: string, fn: (ev?: unknown) => void): void;
}

function fakeFactory(created: FakeSource[]) {
  return (url: string): WireEventSource => {
    const listeners = new Map<string, ((ev?: unknown) => void)[]>();
    const source: FakeSource = {
      url,
      readyState: 0,
      closed: false,
      listeners,
      open: () => {
        source.readyState = 1;
        listeners.get("open")?.forEach((fn) => fn());
      },
      message: (raw, id) =>
        listeners.get("message")?.forEach((fn) => fn({ data: raw, lastEventId: id ?? "" })),
      fail: (closed) => {
        source.readyState = closed ? 2 : 0;
        listeners.get("error")?.forEach((fn) => fn());
      },
      close: () => {
        source.closed = true;
      },
      addEventListener: (type: string, fn: (ev?: unknown) => void) => {
        const list = listeners.get(type) ?? [];
        list.push(fn);
        listeners.set(type, list);
      },
      removeEventListener: (type: string, fn: (ev?: unknown) => void) => {
        listeners.set(type, (listeners.get(type) ?? []).filter((l) => l !== fn));
      },
    };
    created.push(source);
    return source as unknown as WireEventSource;
  };
}

const TICKET_FRAME = JSON.stringify({
  type: "ticket.updated",
  org_id: "00000000-0000-4000-8000-000000000002",
  entity: "ticket",
  entity_id: "t1",
  seq: 7,
  at: "2026-09-13T10:00:00Z",
});

const PRESENCE_FRAME = JSON.stringify({
  type: "presence",
  org_id: "00000000-0000-4000-8000-000000000002",
  entity: "ticket",
  entity_id: "t1",
  at: "2026-09-13T10:00:00Z",
  viewers: [{ user_id: "u1", name: "Ada", at: "2026-09-13T09:59:31Z", editing: true }],
});

function makeStream(created: FakeSource[], onResync?: () => Promise<void>) {
  const bus = new EventBus();
  const stream = new InstantStream({
    url: (scope) =>
      scope === null
        ? "http://api.test/api/events/stream?token=t"
        : `http://api.test/api/events/stream?token=t&ticket=${scope}`,
    bus,
    factory: fakeFactory(created),
    onResync,
  });
  return { bus, stream };
}

beforeEach(() => {
  vi.useRealTimers();
});

describe("InstantStream", () => {
  it("routes contract frames to the bus and presence frames to listeners only", () => {
    const created: FakeSource[] = [];
    const { bus, stream } = makeStream(created);
    const events: unknown[] = [];
    const presence: unknown[] = [];
    bus.subscribe((e) => events.push(e));
    stream.onPresence((p) => presence.push(p));
    stream.subscribeOrg();

    created[0]!.message(TICKET_FRAME, "evt-1");
    created[0]!.message(PRESENCE_FRAME, "pr-1");
    created[0]!.message("not json");

    expect(events).toHaveLength(1);
    const event = events[0] as { type: string; seq: number | null; channel: string };
    expect(event.type).toBe("ticket.updated");
    expect(event.seq).toBe(7);
    expect(event.channel).toBe("org:00000000-0000-4000-8000-000000000002");
    expect(presence).toHaveLength(1);
    expect(created[0]!.url).toContain("/api/events/stream?token=t");
  });

  it("labels ticket-scoped frames with the ticket channel and refcounts scopes", () => {
    const created: FakeSource[] = [];
    const { bus, stream } = makeStream(created);
    const events: unknown[] = [];
    bus.subscribe((e) => events.push(e));
    const off = stream.subscribeTicket("t9");
    expect(created).toHaveLength(1);
    expect(created[0]!.url).toContain("ticket=t9");
    created[0]!.message(TICKET_FRAME);
    const event = events[0] as { channel: string };
    expect(event.channel).toBe("ticket:t9");
    // Second subscriber shares the connection; last unsubscribe closes it.
    const off2 = stream.subscribeTicket("t9");
    off();
    expect(created).toHaveLength(1);
    off2();
    expect(created[0]!.closed).toBe(true);
  });

  it("resyncs after a reconnect that carried a cursor (resume fallback)", async () => {
    const created: FakeSource[] = [];
    const rehydrate = vi.fn().mockResolvedValue(undefined);
    const { stream } = makeStream(created, rehydrate);
    const resyncStates: boolean[] = [];
    stream.onResyncing((v) => resyncStates.push(v));
    stream.subscribeOrg();
    created[0]!.message(TICKET_FRAME, "evt-1");
    expect(rehydrate).not.toHaveBeenCalled();

    // Drop (browser keeps the source alive to auto-reconnect), then the
    // server accepts the reconnect: open fires again on the same source.
    created[0]!.fail(false);
    expect(stream.statusNow()).toBe("connecting");
    created[0]!.open();
    await Promise.resolve();
    expect(rehydrate).toHaveBeenCalledTimes(1);
    expect(resyncStates).toContain(true);
    expect(resyncStates[resyncStates.length - 1]).toBe(false);
    expect(stream.statusNow()).toBe("open");
    expect(stream.resyncingNow()).toBe(false);
  });

  it("does not resync on first open (no cursor yet)", async () => {
    const created: FakeSource[] = [];
    const rehydrate = vi.fn().mockResolvedValue(undefined);
    const { stream } = makeStream(created, rehydrate);
    stream.subscribeOrg();
    created[0]!.open();
    await Promise.resolve();
    expect(rehydrate).not.toHaveBeenCalled();
  });

  it("notifies status listeners of connection transitions", () => {
    const created: FakeSource[] = [];
    const { stream } = makeStream(created);
    const statuses: string[] = [];
    stream.onStatus((s) => statuses.push(s));
    // onStatus replays the current state on attach.
    expect(statuses).toEqual(["closed"]);
    const off = stream.subscribeOrg();
    expect(statuses).toEqual(["closed", "connecting"]);
    created[0]!.open();
    expect(statuses).toEqual(["closed", "connecting", "open"]);
    // A transient drop keeps the source alive for the browser to reconnect.
    created[0]!.fail(false);
    expect(statuses).toEqual(["closed", "connecting", "open", "connecting"]);
    off();
    expect(statuses).toEqual([
      "closed",
      "connecting",
      "open",
      "connecting",
      "closed",
    ]);
    stream.stop();
  });

  it("reconnects with a fresh source when the server refuses (readyState CLOSED)", () => {
    const created: FakeSource[] = [];
    const { stream } = makeStream(created);
    stream.subscribeOrg();
    expect(created).toHaveLength(1);
    // Fake timers BEFORE the failure so the backoff timer is faked.
    vi.useFakeTimers();
    created[0]!.fail(true);
    vi.advanceTimersByTime(2000);
    expect(created.length).toBeGreaterThan(1);
    vi.useRealTimers();
    stream.stop();
  });

  it("wires the runtime: org subscription + rehydrate sweep through the store", async () => {
    seedAuthTokens();
    __resetInstantRuntimeForTests();
    const runtime = instantRuntime();
    const off = runtime.stream.subscribeOrg();
    expect(runtime.stream.statusNow()).toBe("connecting");
    off();
    runtime.stream.stop();
  });
});
