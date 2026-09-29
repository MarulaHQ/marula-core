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

import type { MarulaClient } from "@marula-core/sdk";

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
  // Wave 32 lane 6 added `dispatch` on web; the parity test catches a mobile
  // table that missed it. Lane 8 added `toolboxes` for the toolbox
  // configuration surface.
  dispatch: ["stock_item", "serialized_unit"],
  toolboxes: ["tool_kit"],
  tools: ["serialized_unit"],
  assets: ["device_asset"],
  /** The mobile Scan tab accepts everything and routes by kind. */
  "scan-any": [
    "stock_item",
    "serialized_unit",
    "stock_location",
    "device_asset",
    "tool_kit",
  ],
  // Mobile-only field surfaces (Wave 33 lane 2). The web table stays the
  // shared core; these rows name what the phone's field flows accept.
  "field-checkout": ["serialized_unit", "tool_kit"],
  "field-return": ["serialized_unit", "tool_kit"],
  "stock-move": ["stock_item", "serialized_unit", "stock_location"],
  "ticket-consume": ["stock_item", "serialized_unit"],
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
    return `Nothing matches “${resolution.input}”. Charge the scan bar with a label, a SKU, a serial, a location code, an asset tag, or a toolbox code.`;
  }
  const candidates = resolution.candidates ?? [];
  if (candidates.length > 0) {
    return `${candidates.length} records match — pick the exact one (a scan never guesses).`;
  }
  const expected = (SURFACE_ACCEPTS[surface] ?? []).map(kindLabel).join(", ");
  return `${kindLabel(resolution.kind)} does not apply here — this surface scans ${expected.toLowerCase()}.`;
}

export async function resolveScan(
  client: MarulaClient,
  text: string,
): Promise<ScanResolution> {
  return unwrap(
    await client.api.POST("/api/v1/scan/resolve", {
      body: { text },
    }),
  );
}

/**
 * Turn an ambiguous resolution + the candidate the user picked into the same
 * shape a single hit would have produced. A candidate row only carries its id
 * and labels; the per-kind id field is filled here so downstream staging code
 * never has to special-case ambiguity. The kind rides through unchanged.
 */
export function pickCandidate(
  resolution: ScanResolution,
  entityId: string,
): ScanResolution {
  const candidate = (resolution.candidates ?? []).find(
    (row) => row.entity_id === entityId,
  );
  const picked: ScanResolution = {
    ...resolution,
    entity_id: entityId,
    display_name: candidate?.display_name ?? resolution.display_name,
    secondary: candidate?.secondary ?? resolution.secondary,
    // A candidate row carries no canonical link; the resolved id does.
    deep_link: null,
    candidates: [],
  };
  switch (resolution.kind) {
    case "stock_item":
      picked.stock_item_id = entityId;
      break;
    case "serialized_unit":
      picked.serial_id = entityId;
      break;
    case "stock_location":
      picked.location_id = entityId;
      break;
    case "device_asset":
      picked.device_asset_id = entityId;
      break;
    default:
      break;
  }
  return picked;
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
    case "tool_kit": {
      // The toolbox stencil: one scan opens the kit, where the whole-kit
      // check-out / check-in transactions live (Wave 32 lane 3).
      const id = resolution.entity_id;
      return id !== null && id !== undefined ? `/tool-kit/${id}` : null;
    }
    default:
      return null;
  }
}
