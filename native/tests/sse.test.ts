// The React Native SSE transport: framing, dispatch, the reconnect that
// distinguishes a transient drop from a refusal, and the web passthrough.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createSseSource } from "@/core/sse";

/** One XMLHttpRequest double. The source drives it through readyState the way
 * React Native's implementation does: headers at 2, body chunks at 3, then 4. */
class FakeXhr {
  static instances: FakeXhr[] = [];
  readyState = 0;
  status = 0;
  responseText = "";
  headers: Record<string, string> = {};
  aborted = false;
  onreadystatechange: (() => void) | null = null;
  onerror: (() => void) | null = null;
  ontimeout: (() => void) | null = null;

  constructor() {
    FakeXhr.instances.push(this);
  }

  open(): void {}
  setRequestHeader(name: string, value: string): void {
    this.headers[name] = value;
  }
  send(): void {}
  abort(): void {
    this.aborted = true;
  }

  receiveHeaders(status: number): void {
    this.status = status;
    this.readyState = 2;
    this.onreadystatechange?.();
  }
  chunk(text: string): void {
    this.responseText += text;
    this.readyState = 3;
    this.onreadystatechange?.();
  }
  finish(): void {
    this.readyState = 4;
    this.onreadystatechange?.();
  }
  networkError(): void {
    this.onerror?.();
  }
}

function latestXhr(): FakeXhr {
  const xhr = FakeXhr.instances[FakeXhr.instances.length - 1];
  if (xhr === undefined) throw new Error("no request opened yet");
  return xhr;
}

/** A recorder for the three event types the client listens for. */
function recorder() {
  const events: string[] = [];
  const frames: Array<{ data: string; lastEventId: string }> = [];
  return {
    events,
    frames,
    onOpen: () => events.push("open"),
    onError: () => events.push("error"),
    onMessage: (event?: unknown) => frames.push(event as { data: string; lastEventId: string }),
  };
}

const realEventSource = (globalThis as { EventSource?: unknown }).EventSource;

beforeEach(() => {
  FakeXhr.instances = [];
  (globalThis as { XMLHttpRequest?: unknown }).XMLHttpRequest = FakeXhr;
  delete (globalThis as { EventSource?: unknown }).EventSource;
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  if (realEventSource === undefined) {
    delete (globalThis as { EventSource?: unknown }).EventSource;
  } else {
    (globalThis as { EventSource?: unknown }).EventSource = realEventSource;
  }
});

describe("createSseSource over XMLHttpRequest", () => {
  it("opens on a 2xx header and reports the readyState the client reads", () => {
    const rec = recorder();
    const source = createSseSource("http://host/api/events/stream?token=t");
    source.addEventListener("open", rec.onOpen);
    source.addEventListener("message", rec.onMessage);
    source.addEventListener("error", rec.onError);

    expect(source.readyState).toBe(0);
    expect(latestXhr().headers.Accept).toBe("text/event-stream");

    latestXhr().receiveHeaders(200);

    expect(source.readyState).toBe(1);
    expect(rec.events).toEqual(["open"]);
  });

  it("dispatches one frame per blank line, joining multi-line data and skipping comments", () => {
    const rec = recorder();
    const source = createSseSource("u");
    source.addEventListener("message", rec.onMessage);
    latestXhr().receiveHeaders(200);

    latestXhr().chunk(": keep-alive comment\n\n");
    latestXhr().chunk('id: 7\ndata: {"type":"ticket.updated",\n');
    latestXhr().chunk('data: "org_id":"o1"}\n\n');

    expect(rec.frames).toEqual([
      { data: '{"type":"ticket.updated",\n"org_id":"o1"}', lastEventId: "7" },
    ]);
  });

  it("holds a partial frame until its terminating blank line arrives", () => {
    const rec = recorder();
    const source = createSseSource("u");
    source.addEventListener("message", rec.onMessage);
    latestXhr().receiveHeaders(200);

    latestXhr().chunk("data: {\"type\":\"a\"}");
    expect(rec.frames).toEqual([]);

    latestXhr().chunk("\n\n");
    expect(rec.frames).toEqual([{ data: '{"type":"a"}', lastEventId: "" }]);
  });

  it("reopens a stream that ended, resending Last-Event-ID, and stays out of CLOSED", () => {
    const rec = recorder();
    const source = createSseSource("u");
    source.addEventListener("open", rec.onOpen);
    source.addEventListener("message", rec.onMessage);
    source.addEventListener("error", rec.onError);
    latestXhr().receiveHeaders(200);
    latestXhr().chunk('id: 42\ndata: {"type":"a"}\n\n');

    latestXhr().finish();

    // A transient end is not a refusal: the source retries itself, so the
    // client (which only reconnects on readyState 2) leaves it alone.
    expect(rec.events).toEqual(["open", "error"]);
    expect(source.readyState).toBe(0);

    vi.advanceTimersByTime(1_000);

    expect(FakeXhr.instances).toHaveLength(2);
    expect(latestXhr().headers["Last-Event-ID"]).toBe("42");

    latestXhr().receiveHeaders(200);
    expect(source.readyState).toBe(1);
    expect(rec.events).toEqual(["open", "error", "open"]);
  });

  it("closes with readyState 2 on a refused response so the client reconnects", () => {
    const rec = recorder();
    const source = createSseSource("u");
    source.addEventListener("error", rec.onError);
    latestXhr().receiveHeaders(401);

    expect(source.readyState).toBe(2);
    expect(rec.events).toEqual(["error"]);

    // A refusal does not retry here — the client owns that path.
    vi.advanceTimersByTime(5_000);
    expect(FakeXhr.instances).toHaveLength(1);
  });

  it("stops retrying and aborts the request once closed", () => {
    const source = createSseSource("u");
    latestXhr().receiveHeaders(200);
    const xhr = latestXhr();

    source.close();

    expect(source.readyState).toBe(2);
    expect(xhr.aborted).toBe(true);

    vi.advanceTimersByTime(5_000);
    expect(FakeXhr.instances).toHaveLength(1);
  });

  it("prefers a global EventSource when the platform ships one", () => {
    class GlobalSource {
      readyState = 0;
      close(): void {}
      addEventListener(): void {}
      removeEventListener(): void {}
    }
    (globalThis as { EventSource?: unknown }).EventSource = GlobalSource;

    const source = createSseSource("u");

    expect(source).toBeInstanceOf(GlobalSource);
    expect(FakeXhr.instances).toHaveLength(0);
  });
});
