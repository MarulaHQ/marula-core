// Instant event types — the client-side view of the core-events envelope
// delivered over the SSE stream (crates/core-api routes/events.rs).
//
// Purpose: parse stream frames into typed events and give read models one
// predicate surface (`relevant`) for routing. Non-purpose: no transport
// (stream.ts owns the connection) and no cache mutation (store owns that).

/** The core-events Envelope (crates/core-events), as delivered over the stream. */
export interface InstantEvent {
  id: string;
  type: string;
  entity: string;
  entity_id: string | null;
  occurred_at: string;
  payload: unknown;
  /** The stream scope that delivered the frame (org:<id> | ticket:<id>). */
  channel: string;
  /**
   * Per-entity monotonic ledger seq — carried on SSE stream frames
   * (crates/core-api routes/events.rs). Identical frames from overlapping
   * scopes/replays dedupe by this identity in the store.
   */
  seq?: number | null;
  /**
   * The one user a per-user frame (`notification.*`, `notification_prefs.*`,
   * `timer.*`) was
   * addressed to. The server already delivered it to that user only; a
   * subscriber comparing it to its own id is asserting the contract held,
   * not doing the routing. Absent on org-scoped frames.
   */
  recipient_user_id?: string | null;
}

// --- SSE stream frames (GET /api/events/stream, crates/core-api) -------------

/** The `data:` JSON of one stream frame — routes/events.rs wire contract:
 * contract events carry exactly `{type, org_id, entity, entity_id, seq, at}`,
 * plus `recipient_user_id` when the event belongs to one user; presence
 * frames add a `viewers` roster and are EPHEMERAL (never trail, never
 * contract events). */
export interface StreamFrameJson {
  type: string;
  org_id: string;
  entity: string;
  entity_id?: string | null;
  seq?: number | null;
  at: string;
  recipient_user_id?: string | null;
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
      recipient_user_id:
        typeof frame.recipient_user_id === "string" ? frame.recipient_user_id : null,
    },
  };
}
