// Typed scan routing (Wave 32 lane 5, `p32-labels-scanning`).
//
// The resolver (`POST /api/scan/resolve`) answers WHAT was scanned; this
// module holds what each surface DOES with that answer. The accepted kinds
// per surface are declared once, here, so the web scan bar and the mobile
// camera leg (Wave 33) share one routing vocabulary instead of inventing
// per-screen meanings.
//
// A surface either ACCEPTS a resolved kind (and fills/queues its action) or
// rejects it with a sentence naming what it expected. Nothing auto-submits:
// a scan fills or queues, the human commits.

import type { ScanResolution } from "../labels/model";

export type ScanKind =
  | "stock_item"
  | "serialized_unit"
  | "stock_location"
  | "device_asset"
  | "unknown";

/** Which resolved kinds each surface acts on. */
export const SURFACE_ACCEPTS: Record<string, ScanKind[]> = {
  "ticket-parts": ["stock_item", "serialized_unit"],
  receiving: ["stock_item", "serialized_unit", "stock_location"],
  "stock-locations": ["stock_item", "stock_location"],
  dispatch: ["stock_item", "serialized_unit"],
  tools: ["serialized_unit"],
  assets: ["device_asset"],
};

export const KIND_LABELS: Record<string, string> = {
  stock_item: "Stock item",
  serialized_unit: "Serialized unit",
  stock_location: "Location",
  device_asset: "Device asset",
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
  if (resolution.secondary != null && resolution.secondary !== "") parts.push(resolution.secondary);
  return parts.join(" · ");
}

/**
 * The scan chip's detail line: name · secondary, kind excluded — the chip
 * already renders the kind as its badge, so repeating it here reads twice.
 */
export function scanDetail(resolution: ScanResolution): string {
  const parts = [resolution.display_name, resolution.secondary ?? ""].filter(
    (part) => part !== "",
  );
  return parts.join(" · ");
}

/** Why a scan was not used here — a sentence, never a shrug. */
export function rejectionCopy(surface: string, resolution: ScanResolution): string {
  if (resolution.kind === "unknown") {
    return `Nothing matches “${resolution.input}”. Charge the scan bar with a label, a SKU, a serial, a location code, or an asset tag.`;
  }
  const candidates = resolution.candidates ?? [];
  if (candidates.length > 0) {
    return `${candidates.length} records match — pick the exact one (a scan never guesses).`;
  }
  const expected = (SURFACE_ACCEPTS[surface] ?? []).map(kindLabel).join(", ");
  return `${kindLabel(resolution.kind)} does not apply here — this surface scans ${expected.toLowerCase()}.`;
}

/** The cost-line draft a ticket-parts scan fills. Quantity stays 1. */
export interface TicketPartDraft {
  description: string;
  quantity: number;
  /** Major units as typed into the form (e.g. "24.50"), when known. */
  unitCost: string;
  currency: string;
  /** Human note shown next to the filled form, never submitted. */
  note: string;
}

export interface StockItemCost {
  name?: string;
  sku?: string | null;
  part_number?: string | null;
  currency?: string;
  last_unit_cost_minor?: number | null;
}

/**
 * Maps an accepted resolution (plus the item's cost, when the caller may
 * read it) to the ticket cost-line fields. Serialized units carry their
 * serial in the description so the line is traceable on the invoice.
 */
export function ticketPartDraft(
  resolution: ScanResolution,
  item: StockItemCost | null,
): TicketPartDraft {
  const base = item?.name ?? resolution.display_name;
  const serial = resolution.kind === "serialized_unit" ? resolution.secondary : null;
  const description = serial != null && serial !== "" ? `${base} (${serial})` : base;
  const minor = item?.last_unit_cost_minor;
  const unitCost =
    minor != null && Number.isFinite(minor) ? (minor / 100).toFixed(2) : "";
  const notes: string[] = [];
  if (serial != null && serial !== "") notes.push(`serial ${serial}`);
  if (resolution.serial_status != null && resolution.serial_status !== "in_stock") {
    notes.push(resolution.serial_status);
  }
  if (resolution.is_tool) notes.push("tool");
  if (unitCost === "") notes.push("no cost on file — enter it");
  return {
    description,
    quantity: 1,
    unitCost,
    currency: item?.currency ?? "USD",
    note: `Scan filled this line${notes.length > 0 ? ` (${notes.join(", ")})` : ""} — review, then add it.`,
  };
}
