/**
 * @module Serialization of the pane tree into the versioned persistence envelope.
 *
 * Purpose: `encodeLayoutState` produces deterministic JSON;
 * `decodeLayoutState` validates untrusted input and REPAIRS recoverable
 * defects (bad sizes, dangling tabs, duplicate ids, degenerate splits),
 * reporting every repair as a warning instead of failing the load.
 * Non-purpose: no storage I/O and no migration framework — future envelope
 * versions get their own upgraders behind this same entry point.
 */

import { createId } from "./tree";
import {
  LAYOUT_STATE_VERSION,
  type LayoutNode,
  type PaneNode,
  type PaneTab,
  type SerializedLayoutState,
  type SplitDirection,
  type SplitNode,
} from "./types";

/** Thrown when persisted state is unrecoverable (not JSON, wrong shape at the root). */
export class LayoutStateDecodeError extends Error {
  constructor(
    message: string,
    options?: { cause?: unknown },
  ) {
    super(message, options);
    this.name = "LayoutStateDecodeError";
  }
}

/** Round fractions to six decimals so persisted JSON stays tidy and stable. */
function round6(value: number): number {
  return Math.round(value * 1e6) / 1e6;
}

/** Encode an in-memory tree into the versioned JSON envelope. */
export function encodeLayoutState(root: LayoutNode, savedAt = new Date().toISOString()): string {
  const state: SerializedLayoutState = {
    version: LAYOUT_STATE_VERSION,
    savedAt,
    root: roundTreeFractions(root),
  };
  return JSON.stringify(state);
}

function roundTreeFractions(node: LayoutNode): LayoutNode {
  if (node.kind === "pane") return node;
  return {
    ...node,
    sizes: node.sizes.map(round6),
    children: node.children.map(roundTreeFractions),
  };
}

export interface DecodeResult {
  state: SerializedLayoutState;
  /** Human-readable list of repairs applied during validation. Empty for clean input. */
  warnings: string[];
}

/**
 * Decode and validate untrusted persisted state. Accepts a JSON string or an
 * already-parsed value. Recoverable defects are repaired and reported via
 * `warnings`; structurally hopeless input throws {@link LayoutStateDecodeError}.
 */
export function decodeLayoutState(raw: string | unknown): DecodeResult {
  let parsed: unknown = raw;
  if (typeof raw === "string") {
    try {
      parsed = JSON.parse(raw);
    } catch (error) {
      throw new LayoutStateDecodeError("persisted layout state is not valid JSON", { cause: error });
    }
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new LayoutStateDecodeError("persisted layout state must be a JSON object");
  }
  const envelope = parsed as Record<string, unknown>;
  const warnings: string[] = [];

  const version = envelope["version"];
  if (version !== LAYOUT_STATE_VERSION) {
    warnings.push(
      `unknown state version ${String(version)}; parsed as compatible with version ${LAYOUT_STATE_VERSION}`,
    );
  }

  const savedAt =
    typeof envelope["savedAt"] === "string" && !Number.isNaN(Date.parse(envelope["savedAt"]))
      ? (envelope["savedAt"] as string)
      : new Date().toISOString();
  if (savedAt !== envelope["savedAt"]) {
    warnings.push(`invalid savedAt timestamp replaced with ${savedAt}`);
  }

  const seenIds = new Set<string>();
  const root = sanitizeNode(envelope["root"], "root", seenIds, warnings);
  if (root === null) {
    throw new LayoutStateDecodeError("persisted layout state has no usable root node");
  }

  return { state: { version: LAYOUT_STATE_VERSION, savedAt, root }, warnings };
}

function sanitizeNode(
  raw: unknown,
  path: string,
  seenIds: Set<string>,
  warnings: string[],
): LayoutNode | null {
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
    warnings.push(`${path}: dropped non-object node`);
    return null;
  }
  const record = raw as Record<string, unknown>;
  const kind = record["kind"];

  if (kind === "split") {
    return sanitizeSplit(record, path, seenIds, warnings);
  }
  if (kind === "pane") {
    return sanitizePane(record, path, seenIds, warnings);
  }
  warnings.push(`${path}: unknown node kind ${JSON.stringify(kind ?? null)}, dropped`);
  return null;
}

function uniqueId(raw: unknown, path: string, fallbackPrefix: string, seenIds: Set<string>, warnings: string[]): string {
  const candidate = typeof raw === "string" && raw.length > 0 ? raw : createId(fallbackPrefix);
  if (typeof raw !== "string" || raw.length === 0) {
    warnings.push(`${path}: missing id, generated replacement`);
  } else if (seenIds.has(candidate)) {
    warnings.push(`${path}: duplicate id ${candidate}, regenerated`);
    return createId(fallbackPrefix);
  }
  seenIds.add(candidate);
  return candidate;
}

function sanitizeSplit(
  record: Record<string, unknown>,
  path: string,
  seenIds: Set<string>,
  warnings: string[],
): LayoutNode | null {
  const id = uniqueId(record["id"], path, "split", seenIds, warnings);

  const direction: SplitDirection = record["direction"] === "column" ? "column" : "row";
  if (record["direction"] !== "row" && record["direction"] !== "column") {
    warnings.push(`${path}: invalid direction, defaulted to "row"`);
  }

  const rawChildren = Array.isArray(record["children"]) ? record["children"] : [];
  if (!Array.isArray(record["children"])) {
    warnings.push(`${path}: children missing or not an array`);
  }
  const children: LayoutNode[] = [];
  rawChildren.forEach((childRaw, index) => {
    const child = sanitizeNode(childRaw, `${path}.children[${index}]`, seenIds, warnings);
    if (child !== null) children.push(child);
  });

  if (children.length === 0) {
    warnings.push(`${path}: split has no surviving children, dropped`);
    return null;
  }
  // A split with one survivor degrades to that survivor; ancestors re-check.
  if (children.length === 1) {
    warnings.push(`${path}: split collapsed to its single remaining child`);
    return children[0] as LayoutNode;
  }

  const sizes = sanitizeSizes(record["sizes"], children.length, path, warnings);
  const node: SplitNode = { kind: "split", id, direction, children, sizes };
  return unwrapIfDegenerate(node, warnings);
}

/** Recursively collapse nested splits that ended up with fewer than two children. */
function unwrapIfDegenerate(node: LayoutNode, warnings: string[]): LayoutNode {
  if (node.kind === "pane") return node;
  const children = node.children.map((child) => unwrapIfDegenerate(child, warnings));
  if (children.length === 1) {
    warnings.push(`${node.id}: nested split collapsed to its single remaining child`);
    return children[0] as LayoutNode;
  }
  return { ...node, children };
}

function sanitizeSizes(raw: unknown, childCount: number, path: string, warnings: string[]): number[] {
  const even = (): number[] => Array.from({ length: childCount }, () => 1 / childCount);
  if (!Array.isArray(raw)) {
    warnings.push(`${path}: sizes missing, reset to equal fractions`);
    return even();
  }
  const values = raw.map((value) => (typeof value === "number" ? value : Number.NaN));
  if (
    values.length !== childCount ||
    values.some((value) => !Number.isFinite(value) || value <= 0)
  ) {
    warnings.push(`${path}: sizes invalid or length mismatch, reset to equal fractions`);
    return even();
  }
  const total = values.reduce((acc, value) => acc + value, 0);
  return total > 0 ? values.map((value) => value / total) : even();
}

function sanitizePane(
  record: Record<string, unknown>,
  path: string,
  seenIds: Set<string>,
  warnings: string[],
): PaneNode | null {
  const id = uniqueId(record["id"], path, "pane", seenIds, warnings);

  const rawTabs = Array.isArray(record["tabs"]) ? record["tabs"] : [];
  if (!Array.isArray(record["tabs"])) {
    warnings.push(`${path}: tabs missing or not an array, treated as empty`);
  }
  const tabSeen = new Set<string>();
  const tabs: PaneTab[] = [];
  rawTabs.forEach((tabRaw, index) => {
    if (tabRaw === null || typeof tabRaw !== "object" || Array.isArray(tabRaw)) {
      warnings.push(`${path}.tabs[${index}]: dropped non-object tab`);
      return;
    }
    const tabRecord = tabRaw as Record<string, unknown>;
    const tabId =
      typeof tabRecord["id"] === "string" && tabRecord["id"].length > 0 && !tabSeen.has(tabRecord["id"])
        ? tabRecord["id"]
        : createId("tab");
    if (typeof tabRecord["id"] !== "string" || tabRecord["id"].length === 0) {
      warnings.push(`${path}.tabs[${index}]: missing id, generated replacement`);
    } else if (tabSeen.has(tabRecord["id"])) {
      warnings.push(`${path}.tabs[${index}]: duplicate tab id, regenerated`);
    }
    tabSeen.add(tabId);
    const title = typeof tabRecord["title"] === "string" ? tabRecord["title"] : "";
    if (typeof tabRecord["title"] !== "string") {
      warnings.push(`${path}.tabs[${index}]: non-string title coerced to empty`);
    }
    tabs.push({ id: tabId, title });
  });

  const activeTabId =
    typeof record["activeTabId"] === "string" && tabSeen.has(record["activeTabId"])
      ? (record["activeTabId"] as string)
      : null;
  if (record["activeTabId"] != null && activeTabId === null) {
    warnings.push(`${path}: activeTabId does not reference a known tab, cleared`);
  }

  const minWidthPx = sanitizeMinExtent(record["minWidthPx"], `${path}.minWidthPx`, warnings);
  const minHeightPx = sanitizeMinExtent(record["minHeightPx"], `${path}.minHeightPx`, warnings);

  return {
    kind: "pane",
    id,
    tabs,
    activeTabId,
    ...(minWidthPx !== undefined ? { minWidthPx } : {}),
    ...(minHeightPx !== undefined ? { minHeightPx } : {}),
  };
}

function sanitizeMinExtent(raw: unknown, path: string, warnings: string[]): number | undefined {
  if (raw === undefined || raw === null) return undefined;
  const value = typeof raw === "number" ? raw : Number.NaN;
  if (!Number.isFinite(value) || value < 0) {
    warnings.push(`${path}: invalid minimum extent, dropped`);
    return undefined;
  }
  return value;
}
