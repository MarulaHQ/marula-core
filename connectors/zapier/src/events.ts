//! The Meridian webhook event catalog, mirrored from the public surface
//! (Settings → Integrations → Webhooks exposes the same set). The trigger
//! side of the Zapier app offers each event as a RESTHook trigger with a
//! typed filter, and the delivery envelope is parsed here — the exact JSON
//! core-events ships: { id, type, org_id, entity, entity_id, occurred_at,
//! payload_schema_ref, payload }.

import type { MeridianClient } from "@meridian/sdk";

/** Every published event type (core-api webhook_admin::KNOWN_EVENT_TYPES). */
export const MERIDIAN_EVENT_TYPES = [
  "ticket.created",
  "ticket.updated",
  "ticket.status_changed",
  "ticket.assigned",
  "ticket.comment_added",
  "ticket.closed",
  "client.created",
  "client.updated",
  "timer.started",
  "timer.stopped",
  "time_entry.created",
  "notification.created",
  "sla.escalation_fired",
  "webhook.test",
] as const;

export type MeridianEventType = (typeof MERIDIAN_EVENT_TYPES)[number];

export function isKnownEventType(value: string): value is MeridianEventType {
  return (MERIDIAN_EVENT_TYPES as readonly string[]).includes(value);
}

/** Delivery envelope shape (core-events::Envelope — the signed body). */
export interface MeridianEnvelope {
  id: string;
  type: string;
  org_id: string;
  entity: string;
  entity_id?: string;
  occurred_at: string;
  payload_schema_ref: string;
  payload: Record<string, unknown>;
}

/** Parse one webhook body into the typed envelope (unknown types kept). */
export function parseEnvelope(raw: unknown): MeridianEnvelope | null {
  if (raw === null || typeof raw !== "object") return null;
  const record = raw as Record<string, unknown>;
  if (typeof record.id !== "string" || typeof record.type !== "string") {
    return null;
  }
  return {
    id: record.id,
    type: record.type,
    org_id: typeof record.org_id === "string" ? record.org_id : "",
    entity: typeof record.entity === "string" ? record.entity : "",
    entity_id: typeof record.entity_id === "string" ? record.entity_id : undefined,
    occurred_at: typeof record.occurred_at === "string" ? record.occurred_at : "",
    payload_schema_ref:
      typeof record.payload_schema_ref === "string" ? record.payload_schema_ref : "",
    payload:
      record.payload !== null && typeof record.payload === "object"
        ? (record.payload as Record<string, unknown>)
        : {},
  };
}

/**
 * The trigger side of the Zapier app: one RESTHook trigger per interesting
 * event family. Meridian's webhook subscription API takes an exact event
 * type list, so each trigger subscribes with exactly its own filter
 * (never "everything").
 */
export interface TriggerDefinition {
  /** Zapier trigger key (e.g. `new_ticket`). */
  key: string;
  noun: string;
  /** Exact event types subscribed for this trigger. */
  eventTypes: readonly MeridianEventType[];
  /** Extract Zapier's output record from the envelope payload. */
  pick: (envelope: MeridianEnvelope) => Record<string, unknown>;
}

/** The envelope's payload IS the record; type + id ride along. */
function withEnvelopeMeta(envelope: MeridianEnvelope): Record<string, unknown> {
  return {
    ...envelope.payload,
    _event_id: envelope.id,
    _event_type: envelope.type,
    _entity_id: envelope.entity_id,
    _occurred_at: envelope.occurred_at,
  };
}

/** The shipped Zapier trigger set (RESTHook over Meridian webhooks). */
export const TRIGGERS: readonly TriggerDefinition[] = [
  {
    key: "ticket_created",
    noun: "Ticket",
    eventTypes: ["ticket.created"],
    pick: withEnvelopeMeta,
  },
  {
    key: "ticket_status_changed",
    noun: "Ticket Status",
    eventTypes: ["ticket.status_changed", "ticket.closed"],
    pick: withEnvelopeMeta,
  },
  {
    key: "ticket_comment",
    noun: "Ticket Comment",
    eventTypes: ["ticket.comment_added"],
    pick: withEnvelopeMeta,
  },
  {
    key: "client_created",
    noun: "Client",
    eventTypes: ["client.created"],
    pick: withEnvelopeMeta,
  },
  {
    key: "sla_escalation",
    noun: "SLA Escalation",
    eventTypes: ["sla.escalation_fired"],
    pick: withEnvelopeMeta,
  },
];

/** Prove the event catalog matches the live deployment in one typed call:
 * the created test webhook must accept every known type. */
export async function verifyEventCatalog(
  client: MeridianClient,
): Promise<{ ok: boolean; rejected: string[] }> {
  const probe = await client.api.POST("/api/v1/webhooks", {
    body: { url: "https://zapier.example/verify-probe", event_types: [...MERIDIAN_EVENT_TYPES] },
  });
  if (probe.error) {
    return { ok: false, rejected: [...MERIDIAN_EVENT_TYPES] };
  }
  const id = probe.data?.webhook.id;
  if (id === undefined) return { ok: false, rejected: [...MERIDIAN_EVENT_TYPES] };
  await client.api.DELETE(`/api/v1/webhooks/{id}`, { params: { path: { id } } });
  return { ok: true, rejected: [] };
}
