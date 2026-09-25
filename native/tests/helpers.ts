// Test doubles for the realtime hub and the SDK transport.

import type { SocketLike } from "@/core/realtime";

export class FakeSocket implements SocketLike {
  readyState = 0;
  sent: string[] = [];
  onopen: ((event?: unknown) => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: ((event?: unknown) => void) | null = null;
  onerror: ((event?: unknown) => void) | null = null;

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }

  receive(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }

  frames(): Record<string, unknown>[] {
    return this.sent.map((raw) => JSON.parse(raw) as Record<string, unknown>);
  }

  serverClose(): void {
    this.readyState = 3;
    this.onclose?.();
  }
}

export class FakeSocketFactory {
  sockets: FakeSocket[] = [];

  create(): FakeSocket {
    const socket = new FakeSocket();
    this.sockets.push(socket);
    return socket;
  }

  latest(): FakeSocket {
    const socket = this.sockets[this.sockets.length - 1];
    if (socket === undefined) throw new Error("no socket created yet");
    return socket;
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
