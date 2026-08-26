// Pure state logic for webhook event-type selection and secrets UX.
//
// Purpose: the event-catalog metadata (curated from core-domain's event
// constants), a multiselect reducer over that catalog plus free-text
// custom types, and delivery-log row classification used by
// /settings/integrations. Unit-testable pure functions only.
//
// Non-purpose: no network I/O; retry/backoff policy stays worker-side.
import type { components } from "@meridian/sdk";

export type DeliveryAttemptView = components["schemas"]["DeliveryAttemptView"];

/** Catalog groups surfaced by the multiselect (subset of EventType consts). */
export const EVENT_CATALOG: { group: string; types: string[] }[] = [
  {
    group: "Tickets",
    types: [
      "ticket.created",
      "ticket.updated",
      "ticket.status_changed",
      "ticket.assigned",
      "ticket.comment_added",
      "ticket.checklist_updated",
      "ticket.closed",
    ],
  },
  {
    group: "Timers & time",
    types: [
      "timer.started",
      "timer.paused",
      "timer.resumed",
      "timer.stopped",
      "time_entry.created",
      "time_entry.updated",
    ],
  },
  {
    group: "Boards & org",
    types: ["board.created", "board.transitions_updated", "user.created", "client.created", "org.updated"],
  },
  {
    group: "SLA & automation",
    types: ["sla.escalation_fired", "automation.triggered"],
  },
];

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
