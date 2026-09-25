// Live-sync client for the realtime WebSocket hub (crates/realtime).
//
// Purpose: hold one authed socket to `/ws`, keep a resubscribable channel
// set, parse the server's typed frames (hello / subscribed / pong /
// presence / event / error), detect a dead link with a heartbeat, and
// reconnect with capped exponential backoff. The wire protocol is mirrored
// from crates/realtime/src/protocol.rs; the event envelope mirrors
// core_events::Envelope.
//
// Non-purpose: no cache or UI decisions — consumers subscribe to events and
// decide what to invalidate (see sync.ts).

export interface EventEnvelope {
  id: string;
  type: string;
  org_id: string;
  entity: string;
  entity_id: string | null;
  occurred_at: string;
  payload_schema_ref: string;
  payload: Record<string, unknown>;
}

export interface SocketLike {
  readonly readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: ((event?: unknown) => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: ((event?: unknown) => void) | null;
  onerror: ((event?: unknown) => void) | null;
}

export type SocketFactory = (url: string) => SocketLike;

export type RealtimeStatus =
  | "idle"
  | "connecting"
  | "open"
  | "reconnecting"
  | "closed";

interface ServerFrame {
  type: string;
  channel?: string;
  event?: EventEnvelope;
  users?: string[];
  message?: string;
  user_id?: string;
  org_id?: string;
}

export interface RealtimeDeps {
  /** Full ws URL without the token, e.g. ws://host:8081/ws */
  url: () => string;
  token: () => string | null;
  socketFactory: SocketFactory;
  /** Base reconnect delay; doubles up to maxDelayMs. */
  baseDelayMs?: number;
  maxDelayMs?: number;
  heartbeatMs?: number;
  /** No server frame within this window forces a reconnect. */
  silentForMs?: number;
  random?: () => number;
}

export class RealtimeClient {
  private readonly deps: Required<RealtimeDeps>;
  private socket: SocketLike | null = null;
  private readonly channels = new Set<string>();
  private readonly eventListeners = new Set<(envelope: EventEnvelope, channel: string) => void>();
  private readonly statusListeners = new Set<(status: RealtimeStatus) => void>();
  private readonly helloListeners = new Set<(userId: string, orgId: string) => void>();
  private reconnectAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private lastFrameAt = 0;
  private closedByUser = false;
  private connectPromise: Promise<void> | null = null;
  private status: RealtimeStatus = "idle";

  constructor(deps: RealtimeDeps) {
    this.deps = {
      baseDelayMs: 1_000,
      maxDelayMs: 30_000,
      heartbeatMs: 25_000,
      silentForMs: 60_000,
      random: Math.random,
      ...deps,
    };
  }

  getStatus(): RealtimeStatus {
    return this.status;
  }

  /** Opens the socket. Resolves once connected (or fails open — callers
   * treat a rejected connect as "offline", never as fatal). */
  connect(): Promise<void> {
    if (this.socket !== null && this.socket.readyState <= 1) return Promise.resolve();
    if (this.connectPromise !== null) return this.connectPromise;
    this.closedByUser = false;
    this.connectPromise = new Promise<void>((resolve) => {
      const token = this.deps.token();
      if (token === null) {
        this.setStatus("idle");
        resolve();
        return;
      }
      const separator = this.deps.url().includes("?") ? "&" : "?";
      const socketUrl = `${this.deps.url()}${separator}token=${encodeURIComponent(token)}`;
      this.setStatus("connecting");
      let settled = false;
      let socket: SocketLike;
      try {
        socket = this.deps.socketFactory(socketUrl);
      } catch {
        this.setStatus("reconnecting");
        this.scheduleReconnect();
        resolve();
        return;
      }
      this.socket = socket;
      socket.onopen = () => {
        this.reconnectAttempt = 0;
        this.lastFrameAt = Date.now();
        this.setStatus("open");
        for (const channel of this.channels) this.sendFrame({ type: "subscribe", channel });
        this.startHeartbeat();
        if (!settled) {
          settled = true;
          resolve();
        }
      };
      socket.onmessage = (event) => {
        this.lastFrameAt = Date.now();
        const frame = parseFrame(event.data);
        if (frame === null) return;
        if (frame.type === "event" && frame.event !== undefined) {
          for (const listener of this.eventListeners) {
            listener(frame.event, frame.channel ?? "");
          }
          return;
        }
        if (frame.type === "hello" && frame.user_id !== undefined && frame.org_id !== undefined) {
          for (const listener of this.helloListeners) listener(frame.user_id, frame.org_id);
          return;
        }
        if (frame.type === "error") {
          // Frames carry no request ids; a rejected subscribe is recoverable
          // and does not close the link.
          return;
        }
      };
      socket.onerror = () => {
        // onclose follows on every real implementation; reconnect there.
        if (!settled) {
          settled = true;
          resolve();
        }
      };
      socket.onclose = () => {
        this.stopHeartbeat();
        this.socket = null;
        if (this.closedByUser) {
          this.setStatus("closed");
          return;
        }
        this.setStatus("reconnecting");
        this.scheduleReconnect();
        if (!settled) {
          settled = true;
          resolve();
        }
      };
    }).finally(() => {
      this.connectPromise = null;
    });
    return this.connectPromise;
  }

  /** Intentional close — no reconnect. */
  close(): void {
    this.closedByUser = true;
    this.cancelReconnect();
    this.stopHeartbeat();
    const socket = this.socket;
    this.socket = null;
    socket?.close();
    this.setStatus("closed");
  }

  subscribe(channel: string): void {
    if (this.channels.has(channel)) return;
    this.channels.add(channel);
    if (this.socket !== null && this.socket.readyState === 1) {
      this.sendFrame({ type: "subscribe", channel });
    }
  }

  unsubscribe(channel: string): void {
    if (!this.channels.delete(channel)) return;
    if (this.socket !== null && this.socket.readyState === 1) {
      this.sendFrame({ type: "unsubscribe", channel });
    }
  }

  subscribedChannels(): string[] {
    return [...this.channels];
  }

  onEvent(listener: (envelope: EventEnvelope, channel: string) => void): () => void {
    this.eventListeners.add(listener);
    return () => {
      this.eventListeners.delete(listener);
    };
  }

  onStatus(listener: (status: RealtimeStatus) => void): () => void {
    this.statusListeners.add(listener);
    return () => {
      this.statusListeners.delete(listener);
    };
  }

  onHello(listener: (userId: string, orgId: string) => void): () => void {
    this.helloListeners.add(listener);
    return () => {
      this.helloListeners.delete(listener);
    };
  }

  private sendFrame(frame: Record<string, unknown>): void {
    if (this.socket === null || this.socket.readyState !== 1) return;
    this.socket.send(JSON.stringify(frame));
  }

  private startHeartbeat(): void {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      if (Date.now() - this.lastFrameAt > this.deps.silentForMs) {
        this.socket?.close();
        return;
      }
      this.sendFrame({ type: "ping" });
    }, this.deps.heartbeatMs);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer !== null) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  private scheduleReconnect(): void {
    this.cancelReconnect();
    const exponent = Math.min(this.reconnectAttempt, 5);
    const base = Math.min(this.deps.baseDelayMs * 2 ** exponent, this.deps.maxDelayMs);
    const jitter = base * 0.2 * this.deps.random();
    this.reconnectAttempt += 1;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      void this.connect();
    }, base + jitter);
  }

  private cancelReconnect(): void {
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  private setStatus(status: RealtimeStatus): void {
    if (this.status === status) return;
    this.status = status;
    for (const listener of this.statusListeners) listener(status);
  }
}

function parseFrame(data: unknown): ServerFrame | null {
  const text = typeof data === "string" ? data : null;
  if (text === null) return null;
  try {
    const parsed = JSON.parse(text) as ServerFrame;
    return typeof parsed.type === "string" ? parsed : null;
  } catch {
    return null;
  }
}
