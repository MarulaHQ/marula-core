// InstantStream — the SSE live leg (IF5 push, Wave 13 lane 9).
//
// Purpose: native EventSource connections to GET /api/events/stream
// (crates/core-api routes/events.rs) feeding the SAME reconcile path as
// every other producer: contract-event frames land on the event bus and
// reach the store's scoped revalidations; `type: "presence"` frames are
// EPHEMERAL — they go to presence listeners (rosters) and NEVER to the
// reconcile path, the trail, or any contract surface. One org-wide
// connection serves the whole app; the ticket detail adds a per-ticket
// `?ticket=`-scoped connection for its presence roster and scoped event
// delivery. Overlapping delivery (org + ticket scope carrying the same
// frame) is safe: identical frames dedupe by their (type, entity, seq,
// at) identity in the store.
//
// Resume: every SSE event carries its uuid as the `id:` field, so the
// browser resends `Last-Event-ID` automatically on reconnect and the
// server replays the bounded outbox gap before live frames resume. The
// past-the-replay-buffer cutoff surfaces to the client only as a comment
// (invisible to EventSource), so a reconnect WITH a cursor also runs one
// idempotent snapshot rehydration sweep — surfaced honestly as a brief
// "resyncing" state, never an error. Within the buffer the replay frames
// arrive first and reconcile through the seq ledger; the sweep is the
// documented fallback (identical to the pre-resume behavior).
//
// Non-purpose: no cache mutation (store owns that), no React bindings
// (use-instant.ts), no presence rendering (use-presence.ts).

import { parseStreamFrame } from "./events";
import type { EventBus } from "./event-bus";

/** The minimal EventSource wire contract (structural: fakes satisfy it). */
export interface WireEventSource {
  readonly readyState: number;
  close(): void;
  addEventListener(
    type: "open" | "error" | "message",
    fn: (ev?: unknown) => void,
  ): void;
  removeEventListener(
    type: "open" | "error" | "message",
    fn: (ev?: unknown) => void,
  ): void;
}

export type EventSourceFactory = (url: string) => WireEventSource;

export type StreamStatus = "connecting" | "open" | "closed";

/** Ephemeral presence roster update from one `type: "presence"` frame. */
export interface PresenceUpdate {
  entity: string;
  entity_id: string | null;
  viewers: import("./events").PresenceViewerJson[];
  at: string;
}

export interface InstantStreamOptions {
  /** Full SSE URL for one scope; null when unauthenticated/disabled. */
  url: (scope: string | null) => string | null;
  bus: EventBus;
  factory?: EventSourceFactory;
  maxBackoffMs?: number;
  /**
   * The reconnect fallback sweep (full snapshot rehydration past the
   * replay buffer). Runs once per reconnect that carried a cursor;
   * resolves when the caches are fresh again.
   */
  onResync?: () => Promise<void>;
}

interface Connection {
  source: WireEventSource;
  scope: string | null;
  sawFrame: boolean;
  open: boolean;
}

export class InstantStream {
  private readonly bus: EventBus;
  private readonly urlOf: (scope: string | null) => string | null;
  private readonly factory: EventSourceFactory;
  private readonly maxBackoffMs: number;
  private readonly onResync: (() => Promise<void>) | null;

  private readonly connections = new Map<string, Connection>();
  private readonly ticketRefs = new Map<string, number>();
  private readonly statusListeners = new Set<(status: StreamStatus) => void>();
  private readonly presenceListeners = new Set<(update: PresenceUpdate) => void>();
  private readonly resyncListeners = new Set<(resyncing: boolean) => void>();

  private stopped = false;
  private retries = new Map<string, ReturnType<typeof setTimeout>>();
  private attempt = 0;
  private resyncing = false;
  private status: StreamStatus = "closed";

  constructor(options: InstantStreamOptions) {
    this.bus = options.bus;
    this.urlOf = options.url;
    this.maxBackoffMs = options.maxBackoffMs ?? 15_000;
    this.factory =
      options.factory ??
      ((url) => new EventSource(url) as unknown as WireEventSource);
    this.onResync = options.onResync ?? null;
  }

  onStatus(fn: (status: StreamStatus) => void): () => void {
    this.statusListeners.add(fn);
    fn(this.status);
    return () => {
      this.statusListeners.delete(fn);
    };
  }

  statusNow(): StreamStatus {
    return this.status;
  }

  /** True while a reconnect is rehydrating caches past the replay buffer. */
  resyncingNow(): boolean {
    return this.resyncing;
  }

  onResyncing(fn: (resyncing: boolean) => void): () => void {
    this.resyncListeners.add(fn);
    fn(this.resyncing);
    return () => {
      this.resyncListeners.delete(fn);
    };
  }

  /** Ephemeral presence frames (rosters) — never contract events. */
  onPresence(fn: (update: PresenceUpdate) => void): () => void {
    this.presenceListeners.add(fn);
    return () => {
      this.presenceListeners.delete(fn);
    };
  }

  /** Refcounted org-wide subscription (every frame in the org). */
  subscribeOrg(): () => void {
    return this.retain(null);
  }

  /** Refcounted per-ticket scope: the ticket's events AND its presence. */
  subscribeTicket(ticketId: string): () => void {
    if (ticketId === "") return () => {};
    return this.retain(ticketId);
  }

  private retain(scope: string | null): () => void {
    const key = scope ?? "";
    if (scope === null) {
      this.orgRefCount += 1;
    } else {
      this.ticketRefs.set(scope, (this.ticketRefs.get(scope) ?? 0) + 1);
    }
    this.ensureConnected();
    return () => {
      if (scope === null) {
        this.orgRefCount = Math.max(0, this.orgRefCount - 1);
      } else {
        const refs = (this.ticketRefs.get(scope) ?? 1) - 1;
        if (refs <= 0) this.ticketRefs.delete(scope);
        else this.ticketRefs.set(scope, refs);
      }
      const conn = this.connections.get(key);
      if (conn !== undefined && !this.wanted(key)) {
        this.connections.delete(key);
        conn.source.close();
      }
      if (this.wantedCount() === 0) this.setStatus("closed");
    };
  }

  private orgRefCount = 0;

  private wanted(key: string): boolean {    return key === "" ? this.orgRefCount > 0 : (this.ticketRefs.get(key) ?? 0) > 0;
  }

  private wantedCount(): number {
    return this.orgRefCount + this.ticketRefs.size;
  }

  /** Idempotently opens connections for every wanted scope. */
  ensureConnected(): void {
    if (this.stopped) return;
    for (const key of ["", ...this.ticketRefs.keys()]) {
      if (this.wanted(key) && !this.connections.has(key)) this.open(key);
    }
  }

  private open(key: string): void {
    const scope = key === "" ? null : key;
    const url = this.urlOf(scope);
    if (url === null) {
      this.setStatus("connecting");
      return;
    }
    this.setStatus("connecting");
    let source: WireEventSource;
    try {
      source = this.factory(url);
    } catch {
      this.scheduleReconnect(key);
      return;
    }
    const conn: Connection = { source, scope, sawFrame: false, open: false };
    this.connections.set(key, conn);

    const onOpen = () => {
      this.attempt = 0;
      conn.open = true;
      this.setStatus("open");
      // Resume with a cursor: the server replays the bounded gap; past
      // the buffer it stays silent, so sweep once — the seq ledger makes
      // replayed frames and the fresh snapshot mutually idempotent.
      if (conn.sawFrame) void this.resync();
    };
    const onMessage = (ev?: unknown) => {
      conn.sawFrame = true;
      const data = (ev as { data?: unknown } | null)?.data;
      if (typeof data !== "string") return;
      this.deliver(data, scope);
    };
    const onError = () => {
      conn.open = false;
      this.setStatus("connecting");
      // readyState CLOSED means the server refused (401 expired token,
      // 429 cap): EventSource gives up, so reconnect ourselves with a
      // fresh URL/token after backoff. Otherwise the browser reconnects
      // natively and resends Last-Event-ID (resume consumed).
      if (source.readyState === 2) {
        this.connections.delete(key);
        source.close();
        this.scheduleReconnect(key);
      }
    };
    source.addEventListener("open", onOpen);
    source.addEventListener("message", onMessage);
    source.addEventListener("error", onError);
  }

  /** Test seam: deliver one `data:` payload as if the wire received it. */
  deliver(raw: string, scope: string | null): void {
    const channel = this.channelFor(raw, scope);
    if (channel === null) return;
    const parsed = parseStreamFrame(raw, channel);
    if (parsed === null) return;
    if (parsed.kind === "presence") {
      const update: PresenceUpdate = {
        entity: parsed.entity,
        entity_id: parsed.entity_id,
        viewers: parsed.viewers,
        at: parsed.at,
      };
      for (const fn of [...this.presenceListeners]) fn(update);
      return;
    }
    this.bus.publish(parsed.event);
  }

  private channelFor(raw: string, scope: string | null): string | null {
    try {
      const frame = JSON.parse(raw) as { org_id?: string };
      if (typeof frame?.org_id !== "string") return null;
      if (scope !== null) return `ticket:${scope}`;
      return `org:${frame.org_id}`;
    } catch {
      return null;
    }
  }

  private async resync(): Promise<void> {
    if (this.resyncing) return;
    this.resyncing = true;
    for (const fn of [...this.resyncListeners]) fn(true);
    try {
      await (this.onResync?.() ?? Promise.resolve());
    } finally {
      this.resyncing = false;
      for (const fn of [...this.resyncListeners]) fn(false);
    }
  }

  stop(): void {
    this.stopped = true;
    for (const retry of this.retries.values()) clearTimeout(retry);
    this.retries.clear();
    for (const conn of this.connections.values()) conn.source.close();
    this.connections.clear();
    this.orgRefCount = 0;
    this.ticketRefs.clear();
    this.setStatus("closed");
  }

  private setStatus(status: StreamStatus): void {
    if (this.status === status) return;
    this.status = status;
    for (const fn of [...this.statusListeners]) fn(status);
  }

  private scheduleReconnect(key: string): void {
    if (this.stopped || this.retries.has(key) || !this.wanted(key)) return;
    this.attempt += 1;
    const retry = setTimeout(() => {
      this.retries.delete(key);
      if (this.wanted(key) && !this.connections.has(key)) this.open(key);
    }, Math.min(1000 * 2 ** this.attempt, this.maxBackoffMs));
    this.retries.set(key, retry);
  }
}
