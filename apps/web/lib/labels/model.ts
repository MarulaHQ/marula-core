// Label model + geometry constants (Wave 32 lane 5, `p32-labels-scanning`).
//
// The SERVER owns rendering: it resolves entity values, computes element
// geometry in millimetres, renders the QR as inline SVG and emits the ZPL.
// This module only mirrors the closed vocabularies for pickers (the same
// values the backend validates) and holds the handful of layout constants
// the on-screen preview needs to place elements — the padding and the
// right-aligned QR rule, both of which the backend layout uses too.
//
// Geometry rule of record: padding 1.5 mm on every edge; text + Code128
// start at the left padding; the QR is bottom-right.

import type { components } from "@marula-core/sdk";

export type LabelTemplate = components["schemas"]["LabelTemplateView"];
export type LabelRender = components["schemas"]["LabelRenderResponse"];
export type ResolvedLabel = components["schemas"]["ResolvedLabel"];
export type LabelEntity = components["schemas"]["LabelEntityView"];
export type ScanResolution = components["schemas"]["ScanResolutionView"];
export type ScanCandidate = components["schemas"]["ScanCandidateView"];

export const LABEL_PADDING_MM = 1.5;

export const LABEL_PURPOSES = [
  "stock_item",
  "stock_location",
  "serialized_unit",
  "device_asset",
  "tool_kit",
] as const;
export type LabelPurpose = (typeof LABEL_PURPOSES)[number];

export const PURPOSE_LABELS: Record<string, string> = {
  stock_item: "Stock item",
  stock_location: "Location bin",
  serialized_unit: "Serialized tool / unit",
  device_asset: "Loan device",
  tool_kit: "Toolbox",
};

/** Size presets as the backend defines them (id → label + mm). */
export const SIZE_PRESETS: { id: string; label: string; widthMm?: number; heightMm?: number }[] = [
  { id: "zebra_2x1", label: 'Zebra 2" × 1"', widthMm: 50.8, heightMm: 25.4 },
  { id: "zebra_3x1", label: 'Zebra 3" × 1"', widthMm: 76.2, heightMm: 25.4 },
  { id: "zebra_4x6", label: 'Zebra 4" × 6"', widthMm: 101.6, heightMm: 152.4 },
  { id: "dymo_address", label: "DYMO address 89 × 28 mm", widthMm: 89, heightMm: 28.6 },
  { id: "dymo_multipurpose", label: "DYMO multi-purpose 57 × 32 mm", widthMm: 57, heightMm: 32 },
  { id: "dymo_square", label: "DYMO square 25 × 25 mm", widthMm: 25.4, heightMm: 25.4 },
  { id: "custom", label: "Custom size" },
];

export const DPI_OPTIONS = [203, 300, 600];

/** Allowed tokens per purpose — mirrors `core_domain::entities::allowed_tokens`. */
export const ALLOWED_TOKENS: Record<string, string[]> = {
  stock_item: ["name", "sku", "part_number", "org_name"],
  stock_location: ["name", "code", "org_name"],
  serialized_unit: ["name", "sku", "serial", "location", "org_name"],
  device_asset: ["name", "tag", "serial", "org_name"],
  tool_kit: ["name", "code", "org_name"],
};

/** Tokens the Code128 symbology may encode per purpose. */
export const BARCODE_TOKENS: Record<string, string[]> = {
  stock_item: ["sku", "part_number"],
  stock_location: ["code"],
  serialized_unit: ["serial", "sku"],
  device_asset: ["tag", "serial"],
  tool_kit: ["code"],
};

export const TOKEN_LABELS: Record<string, string> = {
  name: "Name",
  sku: "SKU",
  part_number: "Part number",
  serial: "Serial number",
  tag: "Asset tag",
  code: "Code",
  location: "Current location",
  org_name: "Organization",
};

export function tokenLabel(token: string): string {
  return TOKEN_LABELS[token] ?? token;
}

export function purposeLabel(purpose: string): string {
  return PURPOSE_LABELS[purpose] ?? purpose;
}

/** Where the QR square sits on the rendered label (mirrors the ZPL path). */
export function qrPositionMm(label: ResolvedLabel): { xMm: number; yMm: number } | null {
  if (label.qr == null) return null;
  return {
    xMm: label.width_mm - LABEL_PADDING_MM - label.qr.size_mm,
    yMm: label.qr.y_mm,
  };
}

/** Deep-link kind → the surface an in-app scan should open. */
export function deepLinkTarget(deepLink: string | null | undefined): string | null {
  if (!deepLink) return null;
  const match = /^meridian:\/\/([a-z-]+)\//.exec(deepLink);
  return match?.[1] ?? null;
}
