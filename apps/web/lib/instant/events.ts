// Instant event types — the client-side view of the core-events envelope
// delivered over /ws (crates/realtime protocol: ServerMessage::Event).
//
// Purpose: parse socket frames into typed events and give read models one
// predicate surface (`relevant`) for routing. Non-purpose: no transport
// (socket.ts owns the connection) and no cache mutation (store owns that).

/** The core-events Envelope (crates/core-events), as delivered over WS. */
export interface InstantEvent {
  id: string;
  type: string;
  entity: string;
  entity_id: string | null;
  occurred_at: string;
  payload: unknown;
  /** The channel that matched (board:<id> | user:<id> | org:<id>). */
  channel: string;
  /**
   * Per-entity monotonic ledger seq — present on SSE stream frames
   * (crates/core-api routes/events.rs), absent on legacy WS events.
   * Identical frames from overlapping scopes/replays dedupe by this
   * identity in the store.
   */
  seq?: number | null;
}

export interface WsFrame {
  type: string;
  channel?: string;
  event?: {
    id?: string;
    type?: string;
    entity?: string;
    entity_id?: string | null;
    occurred_at?: string;
    payload?: unknown;
  };
}

/** Parses one socket text frame; null for anything that is not an event. */
export function parseEventFrame(raw: string): InstantEvent | null {
  let frame: WsFrame;
  try {
    frame = JSON.parse(raw) as WsFrame;
  } catch {
    return null;
  }
  if (
    frame.type !== "event" ||
    frame.event === undefined ||
    frame.event.type === undefined ||
    frame.channel === undefined
  ) {
    return null;
  }
  return {
    id: frame.event.id ?? "",
    type: frame.event.type,
    entity: frame.event.entity ?? frame.event.type.split(".")[0] ?? "",
    entity_id: frame.event.entity_id ?? null,
    occurred_at: frame.event.occurred_at ?? new Date().toISOString(),
    payload: frame.event.payload,
    channel: frame.channel,
  };
}

// --- SSE stream frames (GET /api/events/stream, crates/core-api) -------------

/** The `data:` JSON of one stream frame — routes/events.rs wire contract:
 * contract events carry exactly `{type, org_id, entity, entity_id, seq, at}`;
 * presence frames add a `viewers` roster and are EPHEMERAL (never trail,
 * never contract events). */
export interface StreamFrameJson {
  type: string;
  org_id: string;
  entity: string;
  entity_id?: string | null;
  seq?: number | null;
  at: string;
  viewers?: PresenceViewerJson[];
}

/** One presence viewer (realtime.rs PresenceViewer). */
export interface PresenceViewerJson {
  user_id: string;
  name: string;
  at: string;
  editing: boolean;
}

/** A parsed stream frame: either a contract event for the reconcile path
 * or an ephemeral presence roster update — never both. */
export type ParsedStreamFrame =
  | { kind: "event"; event: InstantEvent }
  | {
      kind: "presence";
      entity: string;
      entity_id: string | null;
      viewers: PresenceViewerJson[];
      at: string;
    };

/** Maps one SSE stream frame onto the parsed union. `channel` labels the
 * connection scope that delivered it (org-wide: `org:<org_id>`). Presence
 * frames keep their own kind so they can be kept OUT of the reconcile
 * path and the event trail. */
export function parseStreamFrame(
  raw: string,
  channel: string,
): ParsedStreamFrame | null {
  let frame: StreamFrameJson;
  try {
    frame = JSON.parse(raw) as StreamFrameJson;
  } catch {
    return null;
  }
  if (typeof frame?.type !== "string" || typeof frame?.entity !== "string") {
    return null;
  }
  if (frame.type === "presence") {
    return {
      kind: "presence",
      entity: frame.entity,
      entity_id: frame.entity_id ?? null,
      viewers: Array.isArray(frame.viewers) ? frame.viewers : [],
      at: typeof frame.at === "string" ? frame.at : new Date().toISOString(),
    };
  }
  return {
    kind: "event",
    event: {
      id: "",
      type: frame.type,
      entity: frame.entity,
      entity_id: frame.entity_id ?? null,
      occurred_at: typeof frame.at === "string" ? frame.at : new Date().toISOString(),
      payload: null,
      channel,
      seq: frame.seq ?? null,
    },
  };
}
