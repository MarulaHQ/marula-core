// Live-sync client for the core-api SSE event stream.
//
// Purpose: hold one authed `EventSource` to `/api/events/stream?token=<jwt>`,
// parse the server's frames (contract events plus ephemeral presence), and
// reconnect with capped exponential backoff. The wire contract is the `data:`
// JSON of one `StreamFrame` (`crates/core-api/src/realtime.rs`, served by
// `routes/events.rs`): `{type, org_id, entity, entity_id, seq, at}` with the
// event uuid on the SSE `id:` field. This leg mirrors the web client,
// `apps/web/lib/instant/stream.ts` — same status vocabulary, same backoff,
// same event/presence split.
//
// SSE is one-directional: there is no client→server subscribe frame. The
// stream is org-scoped by construction (every frame the org commits is
// delivered on the one connection), so `subscribe`/`unsubscribe` are kept for
// the sync engine's channel bookkeeping and tracked locally — see `subscribe`.
//
// Non-purpose: no cache or UI decisions — consumers subscribe to events and
// decide what to invalidate (see sync.ts).

/** The core-events identity of one contract-event frame. The stream carries
 * no `payload` (routes/events.rs ships the identity only) and puts the event
 * uuid on the SSE `id:` field, so nested entity ids resolve from `entity_id`. */
export interface EventEnvelope {
  id: string;
  type: string;
  org_id: string;
  entity: string;
  entity_id: string | null;
  occurred_at: string;
  /** Per-entity ledger seq; absent for events emitted outside the instant
   * path, which clients treat as always-apply. */
  seq?: number | null;
  /** Reserved: the stream carries no payload today. A frame type that adds
   * one lands here with no signature change. */
  payload?: Record<string, unknown>;
}

/** The minimal EventSource wire contract (structural: fakes satisfy it). */
export interface WireEventSource {
  readonly readyState: number;
  close(): void;
  addEventListener(type: "open" | "error" | "message", fn: (event?: unknown) => void): void;
  removeEventListener(type: "open" | "error" | "message", fn: (event?: unknown) => void): void;
}

export type EventSourceFactory = (url: string) => WireEventSource;

/** Mirrors the web stream's vocabulary: `connecting` covers the first attempt
 * and every retry (the source owns native retries), `closed` is both the
 * initial state and the intentional stop. */
export type RealtimeStatus = "connecting" | "open" | "closed";

export interface RealtimeDeps {
  /** Full SSE URL without the token, e.g. http://host:8080/api/events/stream */
  url: () => string;
  token: () => string | null;
  factory: EventSourceFactory;
  /** Ceiling for the client's own reconnects, in ms; doubles from 1s. */
  maxBackoffMs?: number;
}

interface StreamFrameJson {
  type?: string;
  org_id?: string;
  entity?: string;
  entity_id?: string | null;
  seq?: number | null;
  at?: string;
}

type ParsedFrame =
  | { kind: "event"; envelope: EventEnvelope; channel: string; orgId: string }
  | { kind: "presence"; orgId: string };

export class RealtimeClient {
  private readonly deps: Required<RealtimeDeps>;
  private source: WireEventSource | null = null;
  private readonly channels = new Set<string>();
  private readonly eventListeners = new Set<(envelope: EventEnvelope, channel: string) => void>();
  private readonly statusListeners = new Set<(status: RealtimeStatus) => void>();
  private readonly helloListeners = new Set<(userId: string, orgId: string) => void>();
  private reconnectAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private helloSent = false;
  private closedByUser = false;
  private status: RealtimeStatus = "closed";

  constructor(deps: RealtimeDeps) {
    this.deps = { maxBackoffMs: 15_000, ...deps };
  }

  getStatus(): RealtimeStatus {
    return this.status;
  }

  /** Opens the stream. Idempotent: a live or in-flight source is left alone,
   * and without a token the client stays closed and opens nothing. */
  connect(): void {
    if (this.source !== null) return;
    this.closedByUser = false;
    const token = this.deps.token();
    if (token === null) {
      this.setStatus("closed");
      return;
    }
    const separator = this.deps.url().includes("?") ? "&" : "?";
    const streamUrl = `${this.deps.url()}${separator}token=${encodeURIComponent(token)}`;
    this.setStatus("connecting");
    let source: WireEventSource;
    try {
      source = this.deps.factory(streamUrl);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.source = source;
    source.addEventListener("open", () => this.handleOpen(source));
    source.addEventListener("message", (event) => this.handleMessage(source, event));
    source.addEventListener("error", () => this.handleError(source));
  }

  /** Intentional close — no reconnect. */
  close(): void {
    this.closedByUser = true;
    this.cancelReconnect();
    const source = this.source;
    this.source = null;
    source?.close();
    this.setStatus("closed");
  }

  /** The stream is org-scoped, so every channel is already covered by the one
   * connection. The set is kept for API compatibility and for callers that
   * diff it; nothing is sent on the wire. A future per-ticket scope would map
   * a `ticket:<id>` channel onto `?ticket=<id>`, but no caller subscribes one
   * today, so no second connection is opened. */
  subscribe(channel: string): void {
    this.channels.add(channel);
  }

  unsubscribe(channel: string): void {
    this.channels.delete(channel);
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

  /** SSE has no hello frame. The closest signal is the stream's own ready
   * announcement: the first frame of each connection is scoped to the org the
   * token belongs to, so it fires once per open with that `org_id`. The user
   * id is not on the wire and is reported as an empty string. */
  onHello(listener: (userId: string, orgId: string) => void): () => void {
    this.helloListeners.add(listener);
    return () => {
      this.helloListeners.delete(listener);
    };
  }

  private handleOpen(source: WireEventSource): void {
    // Ignore a late `open` from a source that was already replaced or closed.
    if (this.source !== source) return;
    this.reconnectAttempt = 0;
    this.helloSent = false;
    this.setStatus("open");
  }

  private handleMessage(source: WireEventSource, event: unknown): void {
    // A frame queued on a source that has since been replaced or closed is
    // not part of the live stream.
    if (this.source !== source) return;
    const raw = (event as { data?: unknown } | null | undefined)?.data;
    if (typeof raw !== "string") return;
    const lastEventId = (event as { lastEventId?: unknown } | null | undefined)?.lastEventId;
    const frame = parseFrame(raw, typeof lastEventId === "string" ? lastEventId : "");
    if (frame === null) return;
    if (!this.helloSent && frame.orgId !== "") {
      this.helloSent = true;
      for (const listener of this.helloListeners) listener("", frame.orgId);
    }
    // Presence frames are EPHEMERAL — they never reach the event path. Native
    // has no roster surface yet, so they are parsed and dropped.
    if (frame.kind === "presence") return;
    for (const listener of this.eventListeners) listener(frame.envelope, frame.channel);
  }

  private handleError(source: WireEventSource): void {
    // A stale source's error must not touch the live connection's status.
    if (this.source !== source) return;
    this.setStatus("connecting");
    // readyState CLOSED (2) means the server refused and the source gave up —
    // a 401 from an expired token, or the per-user 429 cap. Reconnect
    // ourselves with a fresh URL and token after backoff. Any other state is
    // a transient drop the source retries natively (resending Last-Event-ID),
    // so there is nothing to do here.
    if (source.readyState === 2) {
      this.source = null;
      source.close();
      this.scheduleReconnect();
    }
  }

  private scheduleReconnect(): void {
    if (this.closedByUser || this.reconnectTimer !== null) return;
    this.reconnectAttempt += 1;
    const delay = Math.min(1_000 * 2 ** this.reconnectAttempt, this.deps.maxBackoffMs);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (!this.closedByUser) this.connect();
    }, delay);
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

function parseFrame(raw: string, lastEventId: string): ParsedFrame | null {
  let frame: StreamFrameJson;
  try {
    frame = JSON.parse(raw) as StreamFrameJson;
  } catch {
    return null;
  }
  if (typeof frame?.type !== "string") return null;
  const orgId = typeof frame.org_id === "string" ? frame.org_id : "";
  if (frame.type === "presence") return { kind: "presence", orgId };
  if (typeof frame.entity !== "string") return null;
  const envelope: EventEnvelope = {
    id: lastEventId,
    type: frame.type,
    org_id: orgId,
    entity: frame.entity,
    entity_id: typeof frame.entity_id === "string" ? frame.entity_id : null,
    occurred_at: typeof frame.at === "string" ? frame.at : new Date().toISOString(),
    seq: typeof frame.seq === "number" ? frame.seq : null,
  };
  return {
    kind: "event",
    envelope,
    channel: orgId !== "" ? `org:${orgId}` : "",
    orgId,
  };
}
