// Unit tests: /lib/admin/webhook-events — event multiselect reducer over
// the curated catalog + custom types, wildcard-empty payload semantics,
// registry origin labels, the ungranted-types refusal parser, and
// delivery-log classification tones.
import { describe, expect, it } from "vitest";
import {
  ALL_CATALOG_TYPES,
  EVENT_CATALOG,
  classifyDelivery,
  eventSelectionReducer,
  eventTypeOrigin,
  eventTypeOriginLabel,
  selectionToPayload,
  ungrantedTypesFromError,
} from "../../lib/admin/webhook-events";
import type { components } from "@marula-core/sdk";

type DeliveryAttemptView = components["schemas"]["DeliveryAttemptView"];

const delivery = (patch: Partial<DeliveryAttemptView>): DeliveryAttemptView => ({
  id: "d1",
  event_id: "e1",
  event_type: "ticket.created",
  attempt: 1,
  response_status: 200,
  next_retry_at: null,
  delivered_at: new Date().toISOString(),
  last_error: null,
  created_at: new Date().toISOString(),
  ...patch,
});

describe("event-type multiselect", () => {
  it("catalog is non-empty and dedup-free", () => {
    const all = ALL_CATALOG_TYPES;
    expect(all.length).toBeGreaterThan(10);
    expect(new Set(all).size).toBe(all.length);
    for (const group of EVENT_CATALOG) expect(group.types.length).toBeGreaterThan(0);
  });

  it("toggle adds then removes a type", () => {
    let state = { selected: new Set<string>() };
    state = eventSelectionReducer(state, { type: "toggle", eventType: "ticket.created" });
    expect(state.selected.has("ticket.created")).toBe(true);
    state = eventSelectionReducer(state, { type: "toggle", eventType: "ticket.created" });
    expect(state.selected.size).toBe(0);
  });

  it("add-custom trims, rejects blanks and catalog duplicates silently", () => {
    let state = { selected: new Set<string>() };
    state = eventSelectionReducer(state, { type: "add-custom", raw: "  custom.thing  " });
    expect(state.selected.has("custom.thing")).toBe(true);
    state = eventSelectionReducer(state, { type: "add-custom", raw: "" });
    state = eventSelectionReducer(state, { type: "add-custom", raw: "custom.thing" });
    expect(state.selected.size).toBe(1);
  });

  it("select-all covers exactly the catalog; clear empties", () => {
    let state = { selected: new Set<string>() };
    state = eventSelectionReducer(state, { type: "select-all" });
    expect(state.selected.size).toBe(ALL_CATALOG_TYPES.length);
    state = eventSelectionReducer(state, { type: "clear" });
    expect(state.selected.size).toBe(0);
  });

  it("empty selection serializes to [] which means wildcard on the wire", () => {
    expect(selectionToPayload(new Set())).toEqual([]);
    expect(selectionToPayload(new Set(["b.type", "a.type"]))).toEqual(["a.type", "b.type"]);
  });
});

describe("delivery log classification", () => {
  it("delivered attempts read success with their HTTP status", () => {
    const verdict = classifyDelivery(delivery({ response_status: 200 }));
    expect(verdict.tone).toBe("success");
    expect(verdict.label).toContain("200");
  });

  it("pending retries surface warning with attempt numbers", () => {
    const verdict = classifyDelivery(
      delivery({
        delivered_at: null,
        response_status: null,
        next_retry_at: new Date().toISOString(),
        last_error: "connection refused",
        attempt: 3,
      }),
    );
    expect(verdict.tone).toBe("warning");
    expect(verdict.label).toContain("#3");
  });

  it("transport failure with no retry slot reads danger via its error", () => {
    const verdict = classifyDelivery(
      delivery({ delivered_at: null, response_status: null, next_retry_at: null, last_error: "timeout" }),
    );
    expect(verdict.tone).toBe("danger");
    expect(verdict.label).toBe("timeout");
  });

  it("brand-new attempts without anything yet stay neutral 'pending'", () => {
    const verdict = classifyDelivery(
      delivery({ delivered_at: null, response_status: null, next_retry_at: null, last_error: null }),
    );
    expect(verdict.tone).toBe("neutral");
    expect(verdict.label).toBe("pending");
  });
});

describe("event-type registry origin (Wave 43 lane 5)", () => {
  it("catalog types are the platform seed and read as platform", () => {
    for (const t of ALL_CATALOG_TYPES) {
      expect(eventTypeOrigin(t)).toBe("platform");
      expect(eventTypeOriginLabel(t)).toBe("platform");
    }
  });

  it("anything outside the catalog is grant-admitted and says so", () => {
    expect(eventTypeOrigin("rmm.device_offline")).toBe("grant");
    expect(eventTypeOriginLabel("rmm.device_offline")).toBe("needs grant");
  });

  it("the catalog carries the inbound webhook types lane 1 emits", () => {
    for (const t of ["webhook.ingested", "webhook.received", "alert.ingested", "webhook.test"]) {
      expect(ALL_CATALOG_TYPES).toContain(t);
    }
  });
});

describe("ungrantedTypesFromError — the registry refusal parser", () => {
  const refusal =
    'unknown or ungranted event types ["rmm.device_offline", "acme.crm.contact_created"]; either subscribe only to platform types, or obtain a grant for custom types';

  it("pulls every refused type out of the wire message", () => {
    expect(ungrantedTypesFromError(refusal)).toEqual([
      "rmm.device_offline",
      "acme.crm.contact_created",
    ]);
  });

  it("reads the message off a thrown Error or an envelope object", () => {
    expect(ungrantedTypesFromError(new Error(refusal))).toEqual([
      "rmm.device_offline",
      "acme.crm.contact_created",
    ]);
    expect(ungrantedTypesFromError({ message: refusal })).toEqual([
      "rmm.device_offline",
      "acme.crm.contact_created",
    ]);
  });

  it("yields [] on anything unparseable so callers fall back to generic copy", () => {
    expect(ungrantedTypesFromError("rate limited")).toEqual([]);
    expect(ungrantedTypesFromError(null)).toEqual([]);
    expect(ungrantedTypesFromError(undefined)).toEqual([]);
    expect(ungrantedTypesFromError({})).toEqual([]);
  });
});
