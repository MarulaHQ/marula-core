/**
 * @module Immutable pane-tree construction and mutation operations.
 *
 * Purpose: factories and pure tree transformations (insert/remove/split panes,
 * tab management, size updates) that always return new roots, plus invariant
 * repair (splits keep >= 2 children) and recursive minimum-extent computation.
 * Non-purpose: no persistence format logic (see `serialize.ts`) and no React
 * state management (see `react/use-panel-layout.ts`).
 */

import { DEFAULT_MIN_PANE_PX, MIN_SPLIT_PX, sum } from "./constraints";
import type {
  LayoutNode,
  NodeId,
  PaneNode,
  PaneTab,
  SplitDirection,
  SplitNode,
  TabId,
} from "./types";

let idCounter = 0;

/** Generate a unique node/tab identifier. Uses crypto.randomUUID when present. */
export function createId(prefix: string): string {
  const cryptoRef = globalThis.crypto;
  if (cryptoRef && typeof cryptoRef.randomUUID === "function") {
    return `${prefix}-${cryptoRef.randomUUID()}`;
  }
  idCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${idCounter.toString(36)}`;
}

/** Create a leaf pane. Pass `tabs` to seed the strip; ids are generated when omitted. */
export function createPane(options?: {
  tabs?: readonly (string | PaneTab)[];
  activeTabId?: TabId | null;
  minWidthPx?: number;
  minHeightPx?: number;
}): PaneNode {
  const tabs: PaneTab[] = (options?.tabs ?? []).map((tab) =>
    typeof tab === "string" ? { id: createId("tab"), title: tab } : tab,
  );
  const activeTabId = options?.activeTabId !== undefined ? options.activeTabId : null;
  return {
    kind: "pane",
    id: createId("pane"),
    tabs,
    activeTabId: tabs.some((tab) => tab.id === activeTabId) ? activeTabId : null,
    minWidthPx: options?.minWidthPx,
    minHeightPx: options?.minHeightPx,
  };
}

/**
 * Create a split node. Children must be at least two; sizes default to equal
 * fractions and are normalized defensively.
 */
export function createSplit(
  direction: SplitDirection,
  children: readonly LayoutNode[],
  sizes?: readonly number[],
): SplitNode {
  if (children.length < 2) {
    throw new Error("createSplit: a split requires at least two children");
  }
  const normalized =
    sizes !== undefined &&
    sizes.length === children.length &&
    sizes.every((size) => Number.isFinite(size) && size > 0)
      ? normalizeFractions(sizes)
      : equalFractions(children.length);
  return { kind: "split", id: createId("split"), direction, children, sizes: normalized };
}

export function equalFractions(count: number): number[] {
  return Array.from({ length: count }, () => 1 / count);
}

function normalizeFractions(sizes: readonly number[]): number[] {
  const total = sizes.reduce((acc, size) => acc + size, 0);
  if (!(total > 0)) return equalFractions(sizes.length);
  return sizes.map((size) => size / total);
}

/** Pre-order visit of every node. Return `false` from `visit` to skip that subtree. */
export function visitTree(node: LayoutNode, visit: (node: LayoutNode) => void | false): void {
  if (visit(node) === false) return;
  if (node.kind === "split") {
    for (const child of node.children) {
      visitTree(child, visit);
    }
  }
}

/** Depth-first search by node id. */
export function findNode(root: LayoutNode, id: NodeId): LayoutNode | null {
  let found: LayoutNode | null = null;
  visitTree(root, (node) => {
    if (node.id === id) {
      found = node;
      return false;
    }
    return undefined;
  });
  return found;
}

export function findPane(root: LayoutNode, id: NodeId): PaneNode | null {
  const node = findNode(root, id);
  return node !== null && node.kind === "pane" ? node : null;
}

/** Return a new tree with `updater` applied to the node matching `id`, if present. */
export function updateNode(
  root: LayoutNode,
  id: NodeId,
  updater: (node: LayoutNode) => LayoutNode,
): LayoutNode {
  if (root.id === id) return updater(root);
  if (root.kind !== "split") return root;
  let changed = false;
  const children = root.children.map((child) => {
    const next = updateNode(child, id, updater);
    if (next !== child) changed = true;
    return next;
  });
  return changed ? { ...root, children } : root;
}

/** Replace sizes of one split, normalizing defensively. No-op when id is unknown. */
export function setSplitSizes(root: LayoutNode, splitId: NodeId, sizes: readonly number[]): LayoutNode {
  if (!sizes.every((size) => Number.isFinite(size) && size >= 0) || !(sum(sizes) > 0)) {
    return root;
  }
  return updateNode(root, splitId, (node) =>
    node.kind === "split" && node.children.length === sizes.length
      ? { ...node, sizes: normalizeFractions(sizes) }
      : node,
  );
}

/**
 * Insert `newPane` adjacent to the pane identified by `targetPaneId`.
 * Splitting the root replaces it with a new split; otherwise the parent split
 * absorbs the new child, stealing half the neighbor's fraction.
 */
export function insertPane(
  root: LayoutNode,
  targetPaneId: NodeId,
  newPane: PaneNode,
  options: { direction: SplitDirection; position: "before" | "after" },
): LayoutNode {
  const { direction, position } = options;
  if (root.kind === "pane") {
    if (root.id !== targetPaneId) return root;
    const children = position === "before" ? [newPane, root] : [root, newPane];
    return createSplit(direction, children);
  }

  const index = root.children.findIndex((child) => child.id === targetPaneId);
  if (index === -1) {
    let changed = false;
    const children = root.children.map((child) => {
      const next = insertPane(child, targetPaneId, newPane, options);
      if (next !== child) changed = true;
      return next;
    });
    return changed ? { ...root, children } : root;
  }

  // Repair nested single-child splits before splicing so invariants hold.
  const repaired = unwrapSingleChildSplits(root);
  if (repaired !== root) {
    return insertPane(repaired, targetPaneId, newPane, options);
  }
  if (repaired.kind !== "split") {
    return insertPane(repaired, targetPaneId, newPane, options);
  }

  const targetIndex = repaired.children.findIndex((child) => child.id === targetPaneId);
  if (targetIndex === -1 || repaired.children[targetIndex]?.kind !== "pane") {
    return root;
  }

  // The neighbor cedes half of its fraction to the inserted pane.
  const half = (repaired.sizes[targetIndex] as number) / 2;
  const insertAt = position === "before" ? targetIndex : targetIndex + 1;
  const children = [...repaired.children];
  children.splice(insertAt, 0, newPane);
  const sizes = [...repaired.sizes];
  sizes[targetIndex] = half;
  sizes.splice(insertAt, 0, half);
  return { ...repaired, children, sizes };
}

/**
 * Remove the pane identified by `paneId` and repair ancestors whose child
 * count dropped below two (unwrapping their sole remaining child, possibly up
 * to the root). Remaining siblings absorb the freed fraction equally.
 */
export function removePane(root: LayoutNode, paneId: NodeId): { root: LayoutNode; removed: PaneNode | null } {
  const target = findPane(root, paneId);
  if (target === null || root.kind === "pane") {
    return { root, removed: null };
  }
  const stripped = stripPane(root, paneId);
  if (stripped === null) {
    return { root, removed: null };
  }
  return { root: unwrapSingleChildSplits(stripped), removed: target };
}

function stripPane(node: LayoutNode, paneId: NodeId): LayoutNode | null {
  if (node.kind === "pane") return node.id === paneId ? null : node;

  const kept: LayoutNode[] = [];
  const keptSizes: number[] = [];
  let removedHere = false;
  let changedHere = false;
  for (let index = 0; index < node.children.length; index += 1) {
    const child = node.children[index] as LayoutNode;
    if (child.kind === "pane" && child.id === paneId) {
      removedHere = true;
      continue;
    }
    const nextChild = child.kind === "split" ? stripPane(child, paneId) : child;
    if (nextChild === null) {
      removedHere = true;
      continue;
    }
    if (nextChild !== child) changedHere = true;
    kept.push(nextChild);
    keptSizes.push(node.sizes[index] as number);
  }

  if (!removedHere && !changedHere) return node;
  if (kept.length === 0) return null;
  const total = keptSizes.reduce((acc, size) => acc + size, 0);
  const sizes =
    total > 0 ? keptSizes.map((size) => size / total) : equalFractions(kept.length);
  return { ...node, children: kept, sizes };
}

/** Collapse any split with fewer than two children into its sole survivor, recursively. */
export function unwrapSingleChildSplits(node: LayoutNode): LayoutNode {
  if (node.kind === "pane") return node;
  const children = node.children.map(unwrapSingleChildSplits);
  const sizes = node.sizes;
  const total = sum(sizes);
  const normalizedSizes = total > 0 ? sizes.map((size) => size / total) : equalFractions(children.length);

  if (children.length === 0) {
    return createPane();
  }
  if (children.length === 1) {
    return children[0] as LayoutNode;
  }
  const unchanged = children.every((child, index) => child === node.children[index]);
  return unchanged && normalizedSizes.every((size, index) => size === sizes[index])
    ? node
    : { ...node, children, sizes: normalizedSizes };
}

/**
 * Minimum extent a subtree occupies along an axis, in CSS pixels.
 * Panes contribute their declared axis minimum (or the engine default);
 * splits contribute the sum of their children's minimums. Divider track widths
 * are intentionally excluded — the approximation is documented engine behavior.
 */
export function subtreeAxisMinPx(node: LayoutNode, axis: "row" | "column"): number {
  if (node.kind === "pane") {
    const declared = axis === "row" ? node.minWidthPx : node.minHeightPx;
    return Math.max(0, declared ?? DEFAULT_MIN_PANE_PX);
  }
  const childTotal = node.children.reduce(
    (total, child) => total + subtreeAxisMinPx(child, axis),
    0,
  );
  return Math.max(MIN_SPLIT_PX, childTotal);
}

// ---------------------------------------------------------------------------
// Tab operations
// ---------------------------------------------------------------------------

/** Activate one tab within its pane. No-op when either id is unknown. */
export function activateTab(root: LayoutNode, paneId: NodeId, tabId: TabId): LayoutNode {
  return updateNode(root, paneId, (node) =>
    node.kind === "pane" && node.tabs.some((tab) => tab.id === tabId)
      ? { ...node, activeTabId: tabId }
      : node,
  );
}

/**
 * Add a tab to a pane (appended last) and activate it. A title string is
 * accepted for convenience; pass a full {@link PaneTab} to control the id.
 */
export function openTab(
  root: LayoutNode,
  paneId: NodeId,
  tab: string | PaneTab,
): { root: LayoutNode; tab: PaneTab } {
  const resolved: PaneTab = typeof tab === "string" ? { id: createId("tab"), title: tab } : tab;
  const nextRoot = updateNode(root, paneId, (node) => {
    if (node.kind !== "pane") return node;
    const existing = node.tabs.find((candidate) => candidate.id === resolved.id);
    const tabs = existing
      ? node.tabs.map((candidate) => (candidate.id === resolved.id ? resolved : candidate))
      : [...node.tabs, resolved];
    return { ...node, tabs, activeTabId: resolved.id };
  });
  return { root: nextRoot, tab: resolved };
}

/** Close a tab. The pane stays even when its last tab closes (empty state). */
export function closeTab(root: LayoutNode, paneId: NodeId, tabId: TabId): LayoutNode {
  return updateNode(root, paneId, (node) => {
    if (node.kind !== "pane" || !node.tabs.some((tab) => tab.id === tabId)) return node;
    const index = node.tabs.findIndex((tab) => tab.id === tabId);
    const tabs = node.tabs.filter((tab) => tab.id !== tabId);
    let activeTabId = node.activeTabId;
    if (activeTabId === tabId) {
      const neighbor = tabs[Math.min(index, tabs.length - 1)];
      activeTabId = neighbor ? neighbor.id : null;
    }
    return { ...node, tabs, activeTabId };
  });
}

/**
 * Move a tab between panes (or reorder within one pane by passing the same
 * pane twice). The tab becomes active in the destination; the source activates
 * a neighbor when it lost its active tab. Id collisions in the destination are
 * resolved by reissuing an id.
 */
export function moveTab(
  root: LayoutNode,
  params: { tabId: TabId; fromPaneId: NodeId; toPaneId: NodeId; index?: number },
): LayoutNode {
  const { tabId, fromPaneId, toPaneId } = params;
  const source = findPane(root, fromPaneId);
  const target = findPane(root, toPaneId);
  if (source === null || target === null) return root;
  const moving = source.tabs.find((tab) => tab.id === tabId);
  if (moving === undefined) return root;

  const detached = closeTab(root, fromPaneId, tabId);
  const relocated: PaneTab =
    toPaneId !== fromPaneId && target.tabs.some((tab) => tab.id === tabId)
      ? { ...moving, id: createId("tab") }
      : moving;

  return updateNode(detached, toPaneId, (node) => {
    if (node.kind !== "pane") return node;
    const withoutMoved = node.tabs.filter((tab) => tab.id !== relocated.id);
    const insertAt = clampIndex(params.index ?? withoutMoved.length, withoutMoved.length);
    const tabs = [...withoutMoved];
    tabs.splice(insertAt, 0, relocated);
    return { ...node, tabs, activeTabId: relocated.id };
  });
}

function clampIndex(index: number, length: number): number {
  if (!Number.isFinite(index)) return length;
  return Math.max(0, Math.min(Math.trunc(index), length));
}
