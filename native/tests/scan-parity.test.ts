// Parity proof: the mobile scan routing vocabulary matches the web table
// (apps/web/lib/scan/routing.ts). Mobile adds one surface (`scan-any`) for
// the Scan tab; every shared key must match exactly, and the copy must read
// the same.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type { ScanResolution } from "@/core/models";
import { KIND_LABELS, SURFACE_ACCEPTS, rejectionCopy, scanSummary } from "@/core/scan";

const here = path.dirname(fileURLToPath(import.meta.url));
const webSource = fs.readFileSync(
  path.resolve(here, "../../apps/web/lib/scan/routing.ts"),
  "utf8",
);

function blockOf(name: string): string {
  const start = webSource.indexOf(`export const ${name}`);
  const open = webSource.indexOf("{", start);
  const end = webSource.indexOf("\n};", open);
  return webSource.slice(open + 1, end);
}

function resolution(overrides: Partial<ScanResolution>): ScanResolution {
  return {
    input: "RSV-1234",
    kind: "stock_item",
    display_name: "Example item",
    is_tool: false,
    secondary: "SKU-1",
    deep_link: "marula://stock-item/0192",
    entity_id: "0192",
    stock_item_id: "0192",
    serial_id: null,
    location_id: null,
    device_asset_id: null,
    item_kind: "consumable",
    serial_status: null,
    candidates: [],
    ...overrides,
  } as ScanResolution;
}

describe("scan routing parity with web", () => {
  it("keeps the shared SURFACE_ACCEPTS rows identical", () => {
    const block = blockOf("SURFACE_ACCEPTS");
    const re = /("?[\w-]+"?)\s*:\s*\[([^\]]*)\]/g;
    const shared: Record<string, string[]> = {};
    let match = re.exec(block);
    while (match !== null) {
      const key = (match[1] as string).replace(/"/g, "");
      const kinds = (match[2] as string)
        .split(",")
        .map((kind) => kind.trim().replace(/"/g, ""))
        .filter((kind) => kind.length > 0);
      shared[key] = kinds;
      match = re.exec(block);
    }
    for (const [surface, kinds] of Object.entries(shared)) {
      expect(SURFACE_ACCEPTS[surface], `surface ${surface}`).toEqual(kinds);
    }
  });

  it("keeps the kind labels identical", () => {
    const block = blockOf("KIND_LABELS");
    const re = /([A-Za-z_][\w]*)\s*:\s*"([^"]*)"/g;
    let match = re.exec(block);
    const labels: Record<string, string> = {};
    while (match !== null) {
      labels[match[1] as string] = match[2] as string;
      match = re.exec(block);
    }
    expect(labels).toEqual(KIND_LABELS);
  });

  it("renders the same rejection copy for unknown, ambiguous, and wrong-surface scans", () => {
    const unknown = resolution({ kind: "unknown", display_name: "", input: "???", secondary: null });
    expect(rejectionCopy("tools", unknown)).toBe(
      "Nothing matches “???”. Charge the scan bar with a label, a SKU, a serial, a location code, an asset tag, or a toolbox code.",
    );
    const ambiguous = resolution({
      candidates: [{ entity_id: "a" }, { entity_id: "b" }] as unknown as ScanResolution["candidates"],
    });
    expect(rejectionCopy("tools", ambiguous)).toBe(
      "2 records match — pick the exact one (a scan never guesses).",
    );
    const wrongKind = resolution({ kind: "stock_location" });
    expect(rejectionCopy("tools", wrongKind)).toBe(
      "Location does not apply here — this surface scans serialized unit.",
    );
  });

  it("summarizes a resolution as kind · name · secondary", () => {
    expect(scanSummary(resolution({}))).toBe("Stock item · Example item · SKU-1");
  });
});
