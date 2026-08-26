// Unit tests: /lib/admin/webhook-events — event multiselect reducer over
// the curated catalog + custom types, wildcard-empty payload semantics,
// and delivery-log classification tones.
import { describe, expect, it } from "vitest";
import {
  ALL_CATALOG_TYPES,
  EVENT_CATALOG,
  classifyDelivery,
  eventSelectionReducer,
  selectionToPayload,
} from "../../lib/admin/webhook-events";
import type { components } from "@meridian/sdk";

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
