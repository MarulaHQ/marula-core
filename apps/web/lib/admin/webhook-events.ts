// Pure state logic for webhook event-type selection and secrets UX.
//
// Purpose: the event-catalog metadata, a multiselect reducer over that
// catalog plus free-text custom types, and delivery-log row classification
// used by /settings/integrations. Unit-testable pure functions only.
//
// Wave 43 lane 5: the catalog below is the PLATFORM SEED of the event-type
// registry (migration `0124_event_type_registry.sql`, lane 2) — the types
// Marula itself emits, always subscribable without a grant. The registry
// itself is validation-only server-side (`core_db::event_types::check_event_types`
// gates create/update; there is no read endpoint yet), so a
// non-platform type the org holds a grant for is selected through the
// custom-type field and confirmed or refused by the server. The 422
// refusal names every ungranted type — `ungrantedTypesFromError` parses it
// so the surface can say exactly what was refused and why.
//
// Non-purpose: no network I/O; retry/backoff policy stays worker-side.
import type { components } from "@marula-core/sdk";

export type DeliveryAttemptView = components["schemas"]["DeliveryAttemptView"];

/** Catalog groups surfaced by the multiselect — the registry's platform seed. */
export const EVENT_CATALOG: { group: string; types: string[] }[] = [
  {
    group: "Tickets",
    types: [
      "ticket.created",
      "ticket.updated",
      "ticket.status_changed",
      "ticket.assigned",
      "ticket.comment_added",
      "ticket.closed",
    ],
  },
  {
    group: "Timers & time",
    types: ["timer.started", "timer.stopped", "time_entry.created"],
  },
  {
    group: "Clients",
    types: ["client.created", "client.updated"],
  },
  {
    group: "Notifications & SLA",
    types: ["notification.created", "sla.escalation_fired"],
  },
  {
    group: "Inbound webhooks",
    types: ["alert.ingested", "webhook.ingested", "webhook.received"],
  },
  {
    group: "Webhooks",
    types: ["webhook.test"],
  },
];

/**
 * Registry source of a type as the client can know it: the platform seed
 * (migration 0124) needs no grant; everything else — plugin-contributed,
 * external, or unknown — is admitted only by an org grant, which lives
 * server-side until the registry gains a read endpoint.
 */
export type EventTypeOrigin = "platform" | "grant";

export function eventTypeOrigin(eventType: string): EventTypeOrigin {
  return ALL_CATALOG_TYPES.includes(eventType) ? "platform" : "grant";
}

/** Human label for the origin badge beside a selected type. */
export function eventTypeOriginLabel(eventType: string): string {
  return eventTypeOrigin(eventType) === "platform" ? "platform" : "needs grant";
}

/**
 * Extracts the refused event types from the registry validation failure
 * (`webhook_admin::validate_event_types`): "unknown or ungranted event
 * types [\"a.type\", \"b.type\"]; either subscribe only to platform types,
 * or obtain a grant for custom types". Unparseable errors yield [] so
 * callers fall back to the generic rejection copy.
 */
export function ungrantedTypesFromError(err: unknown): string[] {
  const message =
    typeof err === "string"
      ? err
      : (err as { message?: string } | null | undefined)?.message ?? "";
  const match = /unknown or ungranted event types \[([^\]]*)\]/.exec(message);
  if (match === null) return [];
  return (match[1] ?? "")
    .split(",")
    .map((part) => part.trim().replace(/^"|"$/g, ""))
    .filter((part) => part.length > 0);
}

export const ALL_CATALOG_TYPES: string[] = EVENT_CATALOG.flatMap((g) => g.types);

export interface EventSelectionState {
  /** Explicitly selected known + custom types. */
  selected: Set<string>;
}

export type EventSelectionAction =
  | { type: "toggle"; eventType: string }
  | { type: "add-custom"; raw: string }
  | { type: "clear" }
  | { type: "select-all" };

export function eventSelectionReducer(
  state: EventSelectionState,
  action: EventSelectionAction,
): EventSelectionState {
  switch (action.type) {
    case "toggle": {
      const selected = new Set(state.selected);
      if (selected.has(action.eventType)) selected.delete(action.eventType);
      else selected.add(action.eventType);
      return { selected };
    }
    case "add-custom": {
      const value = action.raw.trim();
      if (value.length === 0) return state;
      const selected = new Set(state.selected);
      selected.add(value);
      return { selected };
    }
    case "clear":
      return { selected: new Set() };
    case "select-all":
      return { selected: new Set(ALL_CATALOG_TYPES) };
  }
}

/** Empty selection means wildcard; mirrors CreateWebhookRequest semantics. */
export function selectionToPayload(selected: Set<string>): string[] {
  return [...selected].sort();
}

// --- delivery log ------------------------------------------------------------

export type DeliveryStatusTone = "success" | "warning" | "danger" | "neutral";

/**
 * Classification shared by rows and filters:
 * success — delivered with a 2xx; pending — awaiting first success or retry;
 * failed — terminal failure for this attempt awaiting backoff/retry.
 */
export function classifyDelivery(d: DeliveryAttemptView): {
  tone: DeliveryStatusTone;
  label: string;
} {
  if (d.delivered_at !== null && d.delivered_at !== undefined) {
    return { tone: "success", label: `delivered · ${d.response_status ?? "?"}` };
  }
  if (d.next_retry_at !== null && d.next_retry_at !== undefined) {
    return { tone: "warning", label: `retry ${d.attempt > 1 ? `#${d.attempt}` : "queued"}` };
  }
  return { tone: d.last_error ? "danger" : "neutral", label: d.last_error ?? "pending" };
}
