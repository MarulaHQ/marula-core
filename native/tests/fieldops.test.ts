// Field-ops write shapes + scan vocabulary (Wave 33 lane 2).
//
// The movement ops must build exactly the public contract bodies, and the
// mobile field surfaces must accept/reject the same kinds the web table
// does — a scan fills a form, it never records.

import { describe, expect, it } from "vitest";

import type { MeridianClient } from "@meridian/sdk";

import type { ScanResolution } from "@/core/models";
import { accepts, pickCandidate, scanRoute } from "@/core/scan";
import { checkinTools, consumeStock, receiveStock, transferStock } from "@/core/writes";

const base = { idempotencyKey: "idem-1" };

function fakeClient(captured: { path: string; opts: unknown }[]): MeridianClient {
  return {
    api: {
      POST: (path: string, opts: unknown) => {
        captured.push({ path, opts });
        return Promise.resolve({
          data: { mutation: { version: 3 } },
          response: new Response(null, { status: 200 }),
        });
      },
    },
  } as unknown as MeridianClient;
}

function resolution(overrides: Partial<ScanResolution>): ScanResolution {
  return {
    input: "x",
    kind: "unknown",
    entity_id: null,
    stock_item_id: null,
    serial_id: null,
    location_id: null,
    device_asset_id: null,
    display_name: "",
    secondary: null,
    deep_link: null,
    is_tool: false,
    item_kind: null,
    serial_status: null,
    candidates: [],
    ...overrides,
  } as ScanResolution;
}

describe("movement write bodies", () => {
  it("builds a van transfer with both locations and the serial", () => {
    const body = transferStock.build(
      {
        itemId: "item-1",
        fromLocationId: "van",
        toLocationId: "shelf",
        qtyMilli: 1000,
        serialId: "serial-1",
        note: "restock",
      },
      base,
    );
    expect(body).toMatchObject({
      item_id: "item-1",
      from_location_id: "van",
      to_location_id: "shelf",
      qty_milli: 1000,
      serial_id: "serial-1",
      note: "restock",
      idempotency_key: "idem-1",
    });
    expect(transferStock.method).toBe("POST");
    expect(transferStock.path(undefined as never)).toBe("/api/v1/stock/transfer");
  });

  it("builds a receive with serial numbers and the ticket anchor", () => {
    const body = receiveStock.build(
      {
        itemId: "item-1",
        locationId: "van",
        qtyMilli: 1000,
        serialNumbers: ["SN-1"],
        ticketId: "ticket-1",
      },
      base,
    );
    expect(body).toMatchObject({
      item_id: "item-1",
      location_id: "van",
      qty_milli: 1000,
      serial_numbers: ["SN-1"],
      ticket_id: "ticket-1",
      idempotency_key: "idem-1",
    });
  });

  it("builds a consume with the serial and ticket", () => {
    const body = consumeStock.build(
      { itemId: "item-1", locationId: "van", qtyMilli: 1000, serialId: "s1", ticketId: "t1" },
      base,
    );
    expect(body).toMatchObject({
      item_id: "item-1",
      location_id: "van",
      qty_milli: 1000,
      serial_id: "s1",
      ticket_id: "t1",
    });
  });

  it("returns a single line and maps it to the checkin contract", async () => {
    const built = checkinTools.build(
      {
        checkoutId: "checkout-1",
        lines: [{ lineId: "line-1", condition: "good" }],
        notes: "back in the crib",
      },
      base,
    );
    expect(built.lines).toEqual([{ lineId: "line-1", condition: "good" }]);
    const captured: { path: string; opts: unknown }[] = [];
    await checkinTools.send(fakeClient(captured), built);
    expect(captured[0]?.path).toBe("/api/v1/tool-checkouts/{id}/checkin");
    const opts = captured[0]?.opts as {
      params: { path: { id: string } };
      body: { lines: unknown[] };
    };
    expect(opts.params.path.id).toBe("checkout-1");
    expect(opts.body.lines).toEqual([
      { line_id: "line-1", return_condition: "good", return_notes: null },
    ]);
  });

  it("returns the whole checkout when no lines are named", () => {
    const built = checkinTools.build({ checkoutId: "checkout-2" }, base);
    expect(built.lines).toBeUndefined();
  });
});

describe("mobile field surface vocabulary", () => {
  it("routes a toolbox scan to the kit screen", () => {
    expect(
      scanRoute(resolution({ kind: "tool_kit", entity_id: "kit-1" })),
    ).toBe("/tool-kit/kit-1");
  });

  it("accepts kits on checkout/return and rejects them on movements", () => {
    const kit = resolution({ kind: "tool_kit", entity_id: "kit-1" });
    expect(accepts("field-checkout", kit)).toBe(true);
    expect(accepts("field-return", kit)).toBe(true);
    expect(accepts("stock-move", kit)).toBe(false);
    expect(accepts("field-checkout", resolution({ kind: "stock_location", location_id: "l" }))).toBe(
      false,
    );
  });

  it("fills the per-kind id when the user picks an ambiguous candidate", () => {
    const ambiguous = resolution({
      kind: "stock_item",
      input: "PN-1",
      display_name: "",
      candidates: [
        { kind: "stock_item", entity_id: "a", display_name: "Cable 1 m", secondary: "C6-1M" },
        { kind: "stock_item", entity_id: "b", display_name: "Cable 2 m", secondary: "C6-2M" },
      ],
    });
    const picked = pickCandidate(ambiguous, "b");
    expect(picked.stock_item_id).toBe("b");
    expect(picked.display_name).toBe("Cable 2 m");
    expect(picked.candidates).toEqual([]);
  });
});
