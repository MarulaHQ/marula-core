// Typed scan routing — the mobile twin of apps/web/lib/scan/routing.ts.
//
// Purpose: the resolver (POST /api/v1/scan/resolve) answers WHAT was
// scanned; this module holds what each mobile surface DOES with it. The
// accepted-kinds vocabulary and the rejection copy are byte-identical to the
// web table (tests/scan-parity.test.ts enforces it), so a scan means the
// same thing on the phone and in the browser (Wave 32 lane 5 contract).
//
// Non-purpose: no camera, no transport. It turns a resolved scan into a
// decision; screens render it.

import type { MeridianClient } from "@meridian/sdk";

import { unwrap } from "./http";
import type { ScanResolution } from "./models";

export type ScanKind =
  | "stock_item"
  | "serialized_unit"
  | "stock_location"
  | "device_asset"
  | "tool_kit"
  | "unknown";

/** Which resolved kinds each surface acts on (frozen vocabulary). */
export const SURFACE_ACCEPTS: Record<string, ScanKind[]> = {
  "ticket-parts": ["stock_item", "serialized_unit"],
  receiving: ["stock_item", "serialized_unit", "stock_location"],
  "stock-locations": ["stock_item", "stock_location"],
  dispatch: ["stock_item", "serialized_unit"],
  tools: ["serialized_unit"],
  assets: ["device_asset"],
  toolboxes: ["tool_kit"],
  /** The mobile Scan tab accepts everything and routes by kind. */
  "scan-any": [
    "stock_item",
    "serialized_unit",
    "stock_location",
    "device_asset",
  ],
};

export const KIND_LABELS: Record<string, string> = {
  stock_item: "Stock item",
  serialized_unit: "Serialized unit",
  stock_location: "Location",
  device_asset: "Device asset",
  tool_kit: "Toolbox",
  unknown: "Not recognized",
};

export function kindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? kind;
}

export function accepts(surface: string, resolution: ScanResolution): boolean {
  const accepted = SURFACE_ACCEPTS[surface] ?? [];
  return resolution.kind !== "unknown" && accepted.includes(resolution.kind as ScanKind);
}

/** One-line evidence for the scan chip: kind · name · secondary. */
export function scanSummary(resolution: ScanResolution): string {
  const parts = [kindLabel(resolution.kind)];
  if (resolution.display_name !== "") parts.push(resolution.display_name);
  if (resolution.secondary != null && resolution.secondary !== "") {
    parts.push(resolution.secondary);
  }
  return parts.join(" · ");
}

/** Why a scan was not used here — a sentence, never a shrug. */
export function rejectionCopy(surface: string, resolution: ScanResolution): string {
  if (resolution.kind === "unknown") {
    return `Nothing matches “${resolution.input}”. Charge the scan bar with a label, a SKU, a serial, a location code, a toolbox code, or an asset tag.`;
  }
  const candidates = resolution.candidates ?? [];
  if (candidates.length > 0) {
    return `${candidates.length} records match — pick the exact one (a scan never guesses).`;
  }
  const expected = (SURFACE_ACCEPTS[surface] ?? []).map(kindLabel).join(", ");
  return `${kindLabel(resolution.kind)} does not apply here — this surface scans ${expected.toLowerCase()}.`;
}

export async function resolveScan(
  client: MeridianClient,
  text: string,
): Promise<ScanResolution> {
  return unwrap(
    await client.api.POST("/api/v1/scan/resolve", {
      body: { text },
    }),
  );
}

/** The in-app route a resolved scan opens (mobile Scan tab). */
export function scanRoute(resolution: ScanResolution): string | null {
  switch (resolution.kind) {
    case "stock_item": {
      const id = resolution.entity_id ?? resolution.stock_item_id;
      return id !== null && id !== undefined ? `/stock-item/${id}` : null;
    }
    case "serialized_unit": {
      const id = resolution.serial_id;
      return id !== null && id !== undefined ? `/stock-serial/${id}` : null;
    }
    case "stock_location": {
      const id = resolution.location_id;
      return id !== null && id !== undefined ? `/stock-location/${id}` : null;
    }
    case "device_asset": {
      const id = resolution.device_asset_id;
      return id !== null && id !== undefined ? `/asset/${id}` : null;
    }
    default:
      return null;
  }
}
