// Registry-driven trigger picker (Wave 43 lane 18).
//
// Purpose: build the rule editor's "what starts this workflow" list from the
// node registry (`GET /api/v1/workflows/nodes?family=trigger`) instead of a
// hand-maintained constant, so the list cannot drift from the catalogue the
// engine actually runs and a new trigger node arrives on the surface without a
// code change. Each trigger node carries its own plain-language `label`, an
// `agent_description`, and the `eventType` values it accepts — the same seam the
// server's own `trigger_event_types()` reads.
//
// The label overlay below does NOT decide which events exist; the registry does.
// It only upgrades the wording of the ones an MSP reaches for most, the way the
// editor's sentence read-back prefers "a ticket's status changes" to the raw
// string. An event with no overlay is humanised, never hidden.
//
// Non-purpose: no fetching (the caller owns the request), no React.
import type { components } from "@marula-core/sdk";

export type NodeView = components["schemas"]["NodeView"];
export type NodeCatalogueView = components["schemas"]["NodeCatalogueView"];

export interface TriggerOption {
  value: string;
  label: string;
}

export interface TriggerGroup {
  /** The registry node's id, or `platform` for the group below. */
  id: string;
  /** The trigger node's own plain-language label. */
  label: string;
  /** The trigger node's own description, shown as the group's hint. */
  description: string;
  options: TriggerOption[];
}

/** Wording of record for the events an operator picks most. Wording only. */
const EVENT_LABEL: Readonly<Record<string, string>> = {
  "ticket.created": "Ticket created",
  "ticket.updated": "Ticket updated",
  "ticket.status_changed": "Ticket status changed",
  "ticket.assigned": "Ticket assigned",
  "ticket.closed": "Ticket closed",
  "ticket.reverted": "Ticket reopened",
  "ticket.board_moved": "Ticket moved to another board",
  "ticket.tags_changed": "Ticket tags changed",
  "ticket.comment_added": "Comment added",
  "ticket.field_set": "A ticket field is set",
  "timer.started": "Timer started",
  "timer.stopped": "Timer stopped",
  "time_entry.created": "Time entry created",
  "time_entry.approval_submitted": "Time submitted for approval",
  "time_entry.approval_approved": "Time entry approved",
  "time_entry.approval_rejected": "Time entry rejected",
  "client.created": "Client created",
  "client.updated": "Client updated",
  "site.created": "Site created",
  "contact.created": "Contact created",
  "quote.accepted": "Quote accepted",
  "quote.declined": "Quote declined",
  "invoice.created": "Invoice created",
  "invoice.payment_recorded": "Invoice paid",
  "invoice.voided": "Invoice voided",
  "credit_note.issued": "Credit note issued",
  "contract.created": "Contract created",
  "appointment.created": "Appointment created",
  "appointment.cancelled": "Appointment cancelled",
  "appointment_request.created": "Appointment requested",
  "appointment_request.approved": "Appointment request approved",
  "appointment_request.declined": "Appointment request declined",
  "dispatch.status_updated": "Dispatch status changed",
  "dispatch.queue_updated": "Dispatch queue changed",
  "on_call.updated": "On-call roster changed",
  "alert.ingested": "Alert ingested",
  "alert.acknowledged": "Alert acknowledged",
  "alert.resolved": "Alert resolved",
  "travel_draft.submitted": "Travel claimed",
  "csat.response_recorded": "CSAT response recorded",
  "sla.escalation_fired": "SLA escalation fires",
  "webhook.ingested": "Inbound webhook received",
  "notification.created": "Notification created",
};

/** How each event reads as a clause of the one-sentence read-back. Wording only. */
const EVENT_PHRASE: Readonly<Record<string, string>> = {
  "ticket.created": "a ticket is created",
  "ticket.updated": "a ticket is updated",
  "ticket.status_changed": "a ticket's status changes",
  "ticket.assigned": "a ticket is assigned",
  "ticket.closed": "a ticket is closed",
  "ticket.comment_added": "a comment is added",
  "csat.response_recorded": "a CSAT response arrives",
  "sla.escalation_fired": "an SLA escalation fires",
  "webhook.ingested": "an inbound webhook arrives",
  "alert.ingested": "an alert is ingested",
  "client.created": "a client is created",
  "client.updated": "a client is updated",
  "notification.created": "a notification is created",
};

const ACRONYM: Readonly<Record<string, string>> = {
  csat: "CSAT",
  sla: "SLA",
  rmm: "RMM",
  api: "API",
};

/**
 * The platform's own emitted types the node registry does not expose as a
 * trigger parameter: the generic signed receiver's `webhook.ingested` and the
 * notification fan-out's `notification.created` (both registered as `platform`
 * in the event-type registry, migration 0124). They are grouped and labelled
 * apart rather than folded into a trigger node, so the surface says where they
 * come from.
 */
const PLATFORM_EVENTS: string[] = ["webhook.ingested", "notification.created"];

const PLATFORM_GROUP: TriggerGroup = {
  id: "platform",
  label: "Platform events",
  description:
    "Events the platform itself emits — an inbound signed webhook, or a notification. Registered in the event-type registry rather than a trigger node's parameters.",
  options: PLATFORM_EVENTS.map((value) => ({ value, label: EVENT_LABEL[value] ?? humanizeEventType(value) })),
};

/** Plain-language label for an event type, curated first and humanised after. */
export function eventLabel(value: string): string {
  return EVENT_LABEL[value] ?? humanizeEventType(value);
}

/** The read-back clause for an event type, curated first and derived after. */
export function triggerPhrase(value: string): string {
  return EVENT_PHRASE[value] ?? `the ${humanizeEventType(value).toLowerCase()} event fires`;
}

/** "ticket.status_changed" -> "Ticket status changed". Never hides an unknown type. */
export function humanizeEventType(value: string): string {
  const words = value
    .split(/[._]+/)
    .filter((word) => word.length > 0)
    .map((word) => ACRONYM[word.toLowerCase()] ?? word);
  if (words.length === 0) return value;
  const sentence = words.join(" ");
  return sentence.charAt(0).toUpperCase() + sentence.slice(1);
}

/** The `eventType` enum a trigger node declares, or [] when it declares none. */
export function nodeEventTypes(node: NodeView): string[] {
  const schema = node.input_schema as
    | { properties?: Record<string, { enum?: unknown }> }
    | null
    | undefined;
  const raw = schema?.properties?.eventType?.enum;
  if (!Array.isArray(raw)) return [];
  return raw.filter((v): v is string => typeof v === "string");
}

/** One option group per trigger node that declares event types, registry order. */
export function registryTriggerGroups(
  catalogue: NodeCatalogueView | null | undefined,
): TriggerGroup[] {
  const groups: TriggerGroup[] = [];
  for (const node of catalogue?.nodes ?? []) {
    const values = nodeEventTypes(node);
    if (values.length === 0) continue;
    groups.push({
      id: node.id,
      label: node.label,
      description: node.agent_description,
      options: values.map((value) => ({ value, label: eventLabel(value) })),
    });
  }
  // The platform group rides beside the registry groups so a board-subject rule
  // keeps the platform's own envelopes available.
  groups.push(PLATFORM_GROUP);
  return groups;
}

/** Every value a picker built from these groups offers, in order. */
export function triggerValues(groups: TriggerGroup[]): string[] {
  return groups.flatMap((group) => group.options.map((option) => option.value));
}

/**
 * Does the registry still offer this value? Used to keep a saved rule's trigger
 * visible and honest when the catalogue no longer lists it, rather than silently
 * dropping the selection.
 */
export function hasTriggerValue(groups: TriggerGroup[], value: string): boolean {
  return triggerValues(groups).includes(value);
}
