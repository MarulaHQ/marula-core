// Test doubles for the SSE live leg and the SDK transport.

import type { WireEventSource } from "@/core/realtime";

/** One EventSource double. `open`/`receive`/`serverClose` drive the client's
 * listeners; `close` records an intentional stop. */
export class FakeEventSource implements WireEventSource {
  readyState = 0;
  closed = false;
  private readonly listeners = new Map<string, Set<(event?: unknown) => void>>();

  close(): void {
    this.closed = true;
    this.readyState = 2;
  }

  addEventListener(type: "open" | "error" | "message", fn: (event?: unknown) => void): void {
    const set = this.listeners.get(type) ?? new Set();
    set.add(fn);
    this.listeners.set(type, set);
  }

  removeEventListener(type: "open" | "error" | "message", fn: (event?: unknown) => void): void {
    this.listeners.get(type)?.delete(fn);
  }

  open(): void {
    this.readyState = 1;
    this.emit("open");
  }

  /** Deliver one frame as a message event, with the frame's SSE `id:` field. */
  receive(frame: unknown, lastEventId = ""): void {
    this.receiveRaw(JSON.stringify(frame), lastEventId);
  }

  /** Deliver a raw `data:` payload (for malformed-frame cases). */
  receiveRaw(data: string, lastEventId = ""): void {
    this.emit("message", { data, lastEventId });
  }

  /** A refused stream: readyState CLOSED, then an error (the client reconnects). */
  serverClose(): void {
    this.readyState = 2;
    this.emit("error");
  }

  /** A transient drop the source retries itself (state stays CONNECTING/OPEN). */
  fail(): void {
    this.emit("error");
  }

  private emit(type: string, event?: unknown): void {
    for (const fn of [...(this.listeners.get(type) ?? [])]) fn(event);
  }
}

export class FakeEventSourceFactory {
  sources: FakeEventSource[] = [];

  create(): FakeEventSource {
    const source = new FakeEventSource();
    this.sources.push(source);
    return source;
  }

  latest(): FakeEventSource {
    const source = this.sources[this.sources.length - 1];
    if (source === undefined) throw new Error("no source created yet");
    return source;
  }
}

export interface FakeApiCall {
  method: string;
  path: string;
  body?: unknown;
}

export interface FakeApiRoute {
  data?: unknown;
  error?: unknown;
  status?: number;
}

/** Minimal SDK-client double: routes by "METHOD path". */
export class FakeApi {
  calls: FakeApiCall[] = [];
  private routes = new Map<string, FakeApiRoute | ((call: FakeApiCall) => FakeApiRoute)>();

  route(method: string, path: string, result: FakeApiRoute | ((call: FakeApiCall) => FakeApiRoute)): void {
    this.routes.set(`${method} ${path}`, result);
  }

  asClient<T>(): T {
    return {
      api: {
        GET: (path: string, init?: { params?: unknown }) => this.invoke("GET", path, init),
        POST: (path: string, init?: { body?: unknown }) => this.invoke("POST", path, init),
        PATCH: (path: string, init?: { body?: unknown }) => this.invoke("PATCH", path, init),
        PUT: (path: string, init?: { body?: unknown }) => this.invoke("PUT", path, init),
        DELETE: (path: string, init?: { body?: unknown }) => this.invoke("DELETE", path, init),
      },
    } as unknown as T;
  }

  private invoke(
    method: string,
    path: string,
    init?: { body?: unknown; params?: unknown },
  ): Promise<unknown> {
    const call: FakeApiCall = { method, path, ...(init?.body !== undefined ? { body: init.body } : {}) };
    this.calls.push(call);
    const route = this.routes.get(`${method} ${path}`);
    const result =
      typeof route === "function"
        ? route(call)
        : route ?? { error: { error: { reason: "not_found", message: "no route" } }, status: 404 };
    const status = result.status ?? (result.error !== undefined ? 409 : 200);
    return Promise.resolve({
      data: result.data,
      error: result.error,
      response: { status, ok: status >= 200 && status < 300, statusText: "" },
    });
  }
}
