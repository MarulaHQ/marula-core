//! The forward path: one signed Marula delivery becomes one Hudu push.
//!
//! Purpose: parse the envelope, refuse a delivery that cannot be built
//! honestly, and push. A refusal is typed and names the field to blame, so the
//! receiver can answer 4xx and the sender learns what was missing. A vendor
//! failure is deliberately NOT caught here, so the receiver can answer 5xx and
//! the sender can retry.

import type { Config } from "./config.js";
import { buildCompany, pushCompany } from "./hudu.js";

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

/**
 * Maps one verified delivery onto a Hudu company push.
 *
 * Returns `ignored` for an event this connector does not act on, and
 * `none-mode` when Hudu is unconfigured (the push is logged, not dialed). Both
 * are a 200: there is nothing for the sender to retry.
 */
export async function forwardClientEvent(
  config: Config,
  rawBody: Buffer,
): Promise<"delivered" | "ignored" | "none-mode"> {
  const envelope = parseEnvelope(rawBody);
  if (!envelope.type.startsWith("client.")) return "ignored";
  if (!envelope.payload) {
    throw new DeliveryRefusal("payload", "payload is missing");
  }
  const id = requiredString(envelope.payload, "id");
  const name = requiredString(envelope.payload, "name");
  const pushed = await pushCompany(config, buildCompany({ id, name }));
  return pushed ? "delivered" : "none-mode";
}
