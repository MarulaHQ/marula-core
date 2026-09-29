// Typed scan routing + print-target helpers (Wave 32 lane 5,
// `p32-labels-scanning`).
import { describe, expect, it } from "vitest";
import type { components } from "@marula-core/sdk";
import {
  accepts,
  rejectionCopy,
  scanSummary,
  SURFACE_ACCEPTS,
  ticketPartDraft,
} from "../../lib/scan/routing";
import {
  labelPrintHtml,
  parsePrinterAddress,
  svgDataUri,
} from "../../lib/labels/print";

type ScanResolutionView = components["schemas"]["ScanResolutionView"];

function resolution(overrides: Partial<ScanResolutionView>): ScanResolutionView {
  return {
    input: "C6-1M",
    kind: "stock_item",
    entity_id: "00000000-0000-4000-8000-000000000010",
    stock_item_id: "00000000-0000-4000-8000-000000000010",
    serial_id: null,
    location_id: null,
    device_asset_id: null,
    display_name: "Cat6 patch cable (1 m)",
    secondary: "C6-1M",
    deep_link: "marula://stock-item/00000000-0000-4000-8000-000000000010",
    is_tool: false,
    item_kind: "consumable",
    serial_status: null,
    candidates: [],
    ...overrides,
  } as ScanResolutionView;
}

describe("surface routing contract", () => {
  it("declares the accepted kinds per surface (the shared contract)", () => {
    expect(SURFACE_ACCEPTS["ticket-parts"]).toEqual(["stock_item", "serialized_unit"]);
    expect(SURFACE_ACCEPTS.tools).toEqual(["serialized_unit"]);
    expect(SURFACE_ACCEPTS["stock-locations"]).toEqual(["stock_item", "stock_location"]);
    expect(SURFACE_ACCEPTS.dispatch).toEqual(["stock_item", "serialized_unit"]);
    expect(SURFACE_ACCEPTS.receiving).toEqual([
      "stock_item",
      "serialized_unit",
      "stock_location",
    ]);
  });

  it("accepts a stock item on ticket parts and rejects a location", () => {
    expect(accepts("ticket-parts", resolution({}))).toBe(true);
    expect(accepts("ticket-parts", resolution({ kind: "stock_location" }))).toBe(false);
    expect(accepts("tools", resolution({}))).toBe(false);
    expect(accepts("tools", resolution({ kind: "serialized_unit", is_tool: true }))).toBe(true);
  });

  it("explains rejections as sentences, including ambiguity", () => {
    const location = resolution({ kind: "stock_location" });
    expect(rejectionCopy("ticket-parts", location)).toContain("does not apply here");
    const unknown = resolution({
      kind: "unknown",
      input: "NOPE",
      display_name: "",
      secondary: null,
      deep_link: null,
    });
    expect(rejectionCopy("ticket-parts", unknown)).toContain("NOPE");
    const ambiguous = resolution({
      candidates: [
        { kind: "stock_item", entity_id: "a", display_name: "A", secondary: null },
        { kind: "stock_item", entity_id: "b", display_name: "B", secondary: null },
      ],
    });
    expect(rejectionCopy("ticket-parts", ambiguous)).toContain("2 records match");
  });

  it("summarises a resolution for the scan chip", () => {
    expect(scanSummary(resolution({}))).toBe("Stock item · Cat6 patch cable (1 m) · C6-1M");
  });
});

describe("ticketPartDraft", () => {
  it("fills name, quantity 1 and the item cost when readable", () => {
    const draft = ticketPartDraft(resolution({}), {
      name: "Cat6 patch cable (1 m)",
      currency: "USD",
      last_unit_cost_minor: 2450,
    });
    expect(draft.description).toBe("Cat6 patch cable (1 m)");
    expect(draft.quantity).toBe(1);
    expect(draft.unitCost).toBe("24.50");
    expect(draft.currency).toBe("USD");
  });

  it("carries the serial into the description for a serialized unit", () => {
    const draft = ticketPartDraft(
      resolution({
        kind: "serialized_unit",
        secondary: "TW-000123",
        display_name: "Torque wrench 1/2in",
        serial_status: "in_stock",
        is_tool: true,
      }),
      { name: "Torque wrench 1/2in", currency: "USD", last_unit_cost_minor: null },
    );
    expect(draft.description).toBe("Torque wrench 1/2in (TW-000123)");
    expect(draft.unitCost).toBe("");
    expect(draft.note).toContain("serial TW-000123");
    expect(draft.note).toContain("tool");
    expect(draft.note).toContain("no cost on file");
  });
});

describe("print targets", () => {
  it("parses host and host:port printer addresses", () => {
    expect(parsePrinterAddress("10.0.0.9")).toEqual({ host: "10.0.0.9", port: 9100 });
    expect(parsePrinterAddress("zebra.local:9101")).toEqual({ host: "zebra.local", port: 9101 });
    expect(parsePrinterAddress("")).toBeNull();
    expect(parsePrinterAddress("host:not-a-port")).toBeNull();
    expect(parsePrinterAddress("host:99999")).toBeNull();
  });

  it("renders a deterministic print document at physical millimetres", () => {
    const label = {
      purpose: "stock_item",
      width_mm: 50.8,
      height_mm: 25.4,
      dpi: 203,
      lines: [
        {
          token: "name",
          text: "Cat6 patch cable",
          y_mm: 1.5,
          height_mm: 4.6,
          font_mm: 3.3,
          truncated: false,
        },
      ],
      barcode: { token: "sku", data: "C6-1M", y_mm: 12.0, height_mm: 7.1, width_mm: 47.8 },
      qr: { data: "marula://stock-item/x", y_mm: 9.9, size_mm: 14.0 },
      dropped_lines: 0,
    };
    const input = {
      label,
      qrSvg: '<svg xmlns="http://www.w3.org/2000/svg"></svg>',
      code128Svg: '<svg xmlns="http://www.w3.org/2000/svg"><rect x="0"/></svg>',
      title: "Cat6 patch cable",
    };
    const html = labelPrintHtml(input);
    expect(html).toBe(labelPrintHtml(input));
    expect(html).toContain("@page { size: 50.8mm 25.4mm; margin: 0; }");
    expect(html).toContain("top:1.5mm;height:4.6mm;font-size:3.30mm");
    expect(html).toContain("Cat6 patch cable");
    expect(html).toContain("data:image/svg+xml");
    expect(html).not.toContain("undefined");
  });

  it("escapes label text instead of injecting markup", () => {
    const html = labelPrintHtml({
      label: {
        purpose: "stock_item",
        width_mm: 50,
        height_mm: 25,
        dpi: 203,
        lines: [
          {
            token: "name",
            text: "<script>alert(1)</script>",
            y_mm: 1.5,
            height_mm: 4,
            font_mm: 2.9,
            truncated: false,
          },
        ],
        barcode: null,
        qr: null,
        dropped_lines: 0,
      },
      qrSvg: null,
      code128Svg: null,
      title: "x",
    });
    expect(html).toContain("&lt;script&gt;");
    expect(html).not.toContain("<script>");
  });

  it("data-uris SVGs rather than inlining them as markup", () => {
    const uri = svgDataUri('<svg xmlns="http://www.w3.org/2000/svg"/>');
    expect(uri.startsWith("data:image/svg+xml;charset=utf-8,")).toBe(true);
  });
});
