// Shared instant WebSocket (IF5 live leg).
//
// Purpose: ONE socket for the instant layer with refcounted channel
// subscriptions, capped-backoff reconnect, automatic resubscription, and
// connection-status listeners (the offline queue replays on connect).
// Frames flow through the event bus; the store and read models never see
// the transport.
//
// Non-purpose: no cache mutation and no auth-refresh scheduling — the
// access token is read at connect time (same policy as use-notifications).

import { parseEventFrame, type InstantEvent } from "./events";
import type { EventBus } from "./event-bus";

export type SocketFactory = (url: string) => WireSocket;

export type ConnectionStatus = "connecting" | "open" | "closed";

export interface InstantSocketOptions {
  url: () => string | null;
  bus: EventBus;
  factory?: SocketFactory;
  maxBackoffMs?: number;
}

/** The minimal wire contract (structural: fakes satisfy it with casts). */
export interface WireSocket {
  send(data: string): void;
  close(): void;
  addEventListener(type: "open" | "close", fn: () => void): void;
  addEventListener(type: "message", fn: (ev: { data: unknown }) => void): void;
  removeEventListener(type: "open" | "close", fn: () => void): void;
  removeEventListener(type: "message", fn: (ev: { data: unknown }) => void): void;
}

export class InstantSocket {
  private readonly bus: EventBus;
  private readonly urlOf: () => string | null;
  private readonly factory: SocketFactory;
  private readonly maxBackoffMs: number;
  private readonly channels = new Map<string, number>();
  private readonly statusListeners = new Set<(status: ConnectionStatus) => void>();
  private wire: WireSocket | null = null;
  private attempt = 0;
  private stopped = false;
  private retry: ReturnType<typeof setTimeout> | null = null;
  private status: ConnectionStatus = "closed";

  constructor(options: InstantSocketOptions) {
    this.bus = options.bus;
    this.urlOf = options.url;
    this.maxBackoffMs = options.maxBackoffMs ?? 15_000;
    this.factory =
      options.factory ??
      ((url) => new WebSocket(url) as unknown as WireSocket);
  }

  onStatus(fn: (status: ConnectionStatus) => void): () => void {
    this.statusListeners.add(fn);
    fn(this.status);
    return () => {
      this.statusListeners.delete(fn);
    };
  }

  /** The current status without subscribing (banner initial render). */
  statusNow(): ConnectionStatus {
    return this.status;
  }

  /** Refcounted subscribe; the frame goes out once the wire is open. */
  subscribe(channel: string): () => void {
    const refs = (this.channels.get(channel) ?? 0) + 1;
    this.channels.set(channel, refs);
    this.ensureConnected();
    if (refs === 1 && this.status === "open") {
      this.wire?.send(JSON.stringify({ type: "subscribe", channel }));
    }
    return () => {
      const current = (this.channels.get(channel) ?? 1) - 1;
      if (current <= 0) this.channels.delete(channel);
      else this.channels.set(channel, current);
      if (current <= 0 && this.wire !== null && this.status === "open") {
        this.wire.send(JSON.stringify({ type: "unsubscribe", channel }));
      }
    };
  }

  /** Idempotently opens the socket when at least one channel is wanted. */
  ensureConnected(): void {
    if (this.stopped || this.wire !== null || this.channels.size === 0) return;
    const url = this.urlOf();
    if (url === null) return;
    this.setStatus("connecting");
    let socket: WireSocket;
    try {
      socket = this.factory(url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.wire = socket;
    const onOpen = () => {
      this.attempt = 0;
      this.setStatus("open");
      for (const channel of this.channels.keys()) {
        socket.send(JSON.stringify({ type: "subscribe", channel }));
      }
    };
    const onMessage = (event: { data: unknown }) => {
      const parsed = parseEventFrame(String(event.data));
      if (parsed !== null) this.bus.publish(parsed);
    };
    const onClose = () => {
      if (this.wire === socket) this.wire = null;
      this.setStatus("closed");
      this.scheduleReconnect();
    };
    socket.addEventListener("open", onOpen);
    socket.addEventListener("message", onMessage);
    socket.addEventListener("close", onClose);
  }

  /** Test seam: deliver a text frame as if the wire received it. */
  deliver(raw: string): void {
    const parsed = parseEventFrame(raw);
    if (parsed !== null) this.bus.publish(parsed);
  }

  stop(): void {
    this.stopped = true;
    if (this.retry !== null) clearTimeout(this.retry);
    this.wire?.close();
    this.wire = null;
    this.setStatus("closed");
  }

  private setStatus(status: ConnectionStatus): void {
    if (this.status === status) return;
    this.status = status;
    for (const fn of [...this.statusListeners]) fn(status);
  }

  private scheduleReconnect(): void {
    if (this.stopped || this.channels.size === 0 || this.retry !== null) return;
    this.attempt += 1;
    this.retry = setTimeout(() => {
      this.retry = null;
      this.ensureConnected();
    }, Math.min(1000 * 2 ** this.attempt, this.maxBackoffMs));
  }
}

/** Delivers an event straight onto the bus (test seam shared by specs). */
export function emitTestEvent(bus: EventBus, event: InstantEvent): void {
  bus.publish(event);
}
