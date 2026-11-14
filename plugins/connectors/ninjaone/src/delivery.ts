//! The forward path: one signed Marula delivery becomes one NinjaOne note.
//!
//! Purpose: parse the envelope, refuse a delivery that cannot be built
//! honestly, and push. A refusal is typed and names the field to blame, so the
//! receiver can answer 4xx and the sender learns what was missing. A vendor
//! failure is deliberately NOT caught here, so the receiver can answer 5xx and
//! the sender can retry.

import type { Config } from "./config.js";
import type { TicketReference } from "./ninjaone.js";
import { buildActivity, pushActivity } from "./ninjaone.js";

/**
 * A delivery that cannot be turned into a vendor call. `field` is the payload
 * path the sender has to fix; the receiver puts it in the 4xx body.
 */
export class DeliveryRefusal extends Error {
  /** The payload path the sender has to fix. */
  readonly field: string;

  constructor(field: string, message: string) {
    super(message);
    this.name = "DeliveryRefusal";
    this.field = field;
  }
}

interface Envelope {
  type: string;
  payload?: Record<string, unknown>;
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}

function parseEnvelope(rawBody: Buffer): Envelope {
  let value: unknown;
  try {
    value = JSON.parse(rawBody.toString("utf8"));
  } catch {
    throw new DeliveryRefusal("body", "body is not JSON");
  }
  const envelope = asRecord(value);
  if (!envelope) throw new DeliveryRefusal("body", "body is not a JSON object");
  const type = envelope["type"];
  if (typeof type !== "string" || type.length === 0) {
    throw new DeliveryRefusal("type", "event type is missing");
  }
  return { type, payload: asRecord(envelope["payload"]) };
}

function requiredString(
  payload: Record<string, unknown>,
  field: string,
): string {
  const value = payload[field];
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new DeliveryRefusal(`payload.${field}`, `payload.${field} is required`);
  }
  return value;
}

function optionalString(
  payload: Record<string, unknown>,
  field: string,
): string | undefined {
  const value = payload[field];
  if (typeof value !== "string" || value.trim().length === 0) return undefined;
  return value;
}

/**
 * Maps one verified delivery onto a NinjaOne activity push.
 *
 * `payload.device_id` is required, and it is the NinjaOne device id the note is
 * posted against. Marula's ticket webhook carries no ticket-to-device link
 * today, so a delivery without it is refused by name rather than posted to a
 * guessed device.
 *
 * Returns `ignored` for an event this connector does not act on, and
 * `none-mode` when NinjaOne is unconfigured (the push is logged, not dialed).
 */
export async function forwardTicketEvent(
  config: Config,
  rawBody: Buffer,
): Promise<"delivered" | "ignored" | "none-mode"> {
  const envelope = parseEnvelope(rawBody);
  if (!envelope.type.startsWith("ticket.")) return "ignored";
  if (!envelope.payload) {
    throw new DeliveryRefusal("payload", "payload is missing");
  }
  const deviceId = requiredString(envelope.payload, "device_id");
  const ticket: TicketReference = {
    id: requiredString(envelope.payload, "ticket_id"),
    number: optionalString(envelope.payload, "ticket_number"),
    title: optionalString(envelope.payload, "ticket_title"),
  };
  const pushed = await pushActivity(
    config,
    deviceId,
    buildActivity({ type: envelope.type, ticket }),
  );
  return pushed ? "delivered" : "none-mode";
}
