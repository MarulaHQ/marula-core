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
