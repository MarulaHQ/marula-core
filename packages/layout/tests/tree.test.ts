/**
 * @module Tests for immutable tree operations: insertion, removal with
 * ancestor repair, tab management, and recursive minimum computation.
 */

import { describe, expect, it } from "vitest";
import { DEFAULT_MIN_PANE_PX, MIN_SPLIT_PX } from "../src/constraints";
import type { PaneNode, SplitNode } from "../src/types";
import {
  activateTab,
  closeTab,
  createPane,
  createSplit,
  findNode,
  insertPane,
  moveTab,
  openTab,
  removePane,
  setSplitSizes,
  subtreeAxisMinPx,
  unwrapSingleChildSplits,
} from "../src/tree";

function pane(title: string, options?: Parameters<typeof createPane>[0]): PaneNode {
  const node = createPane({ tabs: [title], ...options });
  return { ...node, activeTabId: node.tabs[0]?.id ?? null };
}

describe("factories", () => {
  it("createSplit normalizes sizes and rejects single-child splits", () => {
    const split = createSplit("row", [pane("A"), pane("B")], [2, 2]);
    expect(split.sizes).toEqual([0.5, 0.5]);
    expect(() => createSplit("row", [pane("A")])).toThrow();
  });

  it("createPane wires active tabs and generates ids", () => {
    const paneNode = createPane({ tabs: ["One", "Two"], activeTabId: null });
    expect(paneNode.tabs).toHaveLength(2);
    expect(paneNode.activeTabId).toBeNull();
  });
});

describe("insertPane", () => {
  it("replaces a pane root with a new split", () => {
    const leaf = pane("Only");
    const next = insertPane(leaf, leaf.id, pane("New"), { direction: "column", position: "after" });
    const split = next as SplitNode;
    expect(split.kind).toBe("split");
    expect(split.direction).toBe("column");
    expect(split.children.map((child) => child.id)).toEqual([leaf.id, split.children[1]?.id]);
    expect(split.sizes).toEqual([0.5, 0.5]);
  });

  it("inserts into an existing split and steals half the neighbor's fraction", () => {
    const a = pane("A");
    const b = pane("B", { minWidthPx: 200 });
    const root = createSplit("row", [a, b], [0.75, 0.25]);
    const fresh = pane("C");
    const next = insertPane(root, a.id, fresh, { direction: "row", position: "after" }) as SplitNode;

    expect(next.children.map((child) => child.kind)).toEqual(["pane", "pane", "pane"]);
    expect(next.sizes[0]).toBeCloseTo(0.375, 9);
    expect(next.sizes[1]).toBeCloseTo(0.375, 9);
    expect(next.sizes[2]).toBeCloseTo(0.25, 9);
    // Original nodes are untouched (immutability).
    expect(root.sizes).toEqual([0.75, 0.25]);
  });

  it("inserts before when asked", () => {
    const a = pane("A");
    const b = pane("B");
    const root = createSplit("row", [a, b]);
    const next = insertPane(root, b.id, pane("C"), { direction: "row", position: "before" }) as SplitNode;
    expect(next.children[1]?.kind).toBe("pane");
    expect(next.children).toHaveLength(3);
  });

  it("descends into nested splits to find the target", () => {
    const deep = pane("Deep");
    const branch = createSplit("column", [deep, pane("Sibling")]);
    const root = createSplit("row", [branch, pane("Other")]);
    const next = insertPane(root, deep.id, pane("Added"), { direction: "column", position: "after" });
    const updatedBranch = (next as SplitNode).children[0] as SplitNode;
    expect(updatedBranch.children).toHaveLength(3);
  });
});

describe("removePane", () => {
  it("removes a child and redistributes its fraction equally", () => {
    const root = createSplit("row", [pane("A"), pane("B"), pane("C")], [0.2, 0.5, 0.3]);
    const target = ((root as SplitNode).children[1] as PaneNode).id;
    const { root: next, removed } = removePane(root, target);
    expect(removed?.tabs[0]?.title).toBe("B");
    const split = next as SplitNode;
    expect(split.sizes).toEqual([0.4, 0.6]);
  });

  it("unwraps ancestors up to and including the root", () => {
    const survivor = pane("Survivor");
    const inner = createSplit("column", [survivor, pane("Doomed")]);
    const root = createSplit("row", [inner, pane("Side")]);
    // Removing "Side" leaves inner as sole child of root -> root unwraps to inner.
    const sideId = ((root as SplitNode).children[1] as PaneNode).id;
    const { root: afterSide } = removePane(root, sideId);
    expect(afterSide).toBe(inner);

    // Then removing one child of inner leaves a single pane at the root.
    const doomedId = ((inner as SplitNode).children[1] as PaneNode).id;
    const { root: finalRoot } = removePane(inner, doomedId);
    expect(finalRoot).toBe(survivor);
  });

  it("is a no-op for unknown ids", () => {
    const root = createSplit("row", [pane("A"), pane("B")]);
    const result = removePane(root, "missing");
    expect(result.root).toBe(root);
    expect(result.removed).toBeNull();
  });
});

describe("tab operations", () => {
  it("openTab appends, activates, and reports the id actually used", () => {
    const existing = pane("First");
    const { root, tab } = openTab(existing, existing.id, "Second");
    const restored = root as PaneNode;
    expect(restored.tabs.map((t) => t.title)).toEqual(["First", "Second"]);
    expect(restored.activeTabId).toBe(tab.id);
    expect(tab.title).toBe("Second");
  });

  it("activateTab ignores unknown tabs", () => {
    const node = pane("A");
    expect(activateTab(node, node.id, "nope")).toBe(node);
  });

  it("closeTab activates a neighbor when the active tab closes", () => {
    const node = pane("A");
    const { root: withB, tab: second } = openTab(node, node.id, "B");
    const { root: withC, tab: third } = openTab(withB, node.id, "C");
    const closed = closeTab(withC, node.id, third.id) as PaneNode;
    expect(closed.tabs).toHaveLength(2);
    expect(closed.activeTabId).toBe(second?.id);
  });

  it("closeTab keeps the pane alive after the last tab closes", () => {
    const node = pane("Solo");
    const tabId = (node.tabs[0] as { id: string }).id;
    const closed = closeTab(node, node.id, tabId) as PaneNode;
    expect(closed.kind).toBe("pane");
    expect(closed.tabs).toHaveLength(0);
    expect(closed.activeTabId).toBeNull();
  });

  it("moveTab transfers activation to the destination and fixes the source", () => {
    const source = pane("S");
    const target = pane("T");
    const movingId = (source.tabs[0] as { id: string }).id;
    const root = createSplit("row", [source, target]);

    const next = moveTab(root, { tabId: movingId, fromPaneId: source.id, toPaneId: target.id }) as SplitNode;
    const nextSource = next.children[0] as PaneNode;
    const nextTarget = next.children[1] as PaneNode;
    expect(nextSource.tabs).toHaveLength(0);
    expect(nextSource.activeTabId).toBeNull();
    expect(nextTarget.tabs.map((t) => t.title)).toEqual(["T", "S"]);
    expect(nextTarget.activeTabId).toBe(movingId);
  });

  it("moveTab reorders within a pane by index", () => {
    const node = pane("A");
    const { root: withB, tab: b } = openTab(node, node.id, "B");
    const { root: withC } = openTab(withB, node.id, "C");
    const reordered = moveTab(withC, {
      tabId: b.id,
      fromPaneId: node.id,
      toPaneId: node.id,
      index: 0,
    }) as PaneNode;
    expect(reordered.tabs.map((t) => t.title)).toEqual(["B", "A", "C"]);
  });
});

describe("setSplitSizes / updateNode / findNode", () => {
  it("setSplitSizes normalizes and ignores invalid input", () => {
    const root = createSplit("row", [pane("A"), pane("B")]);
    const resized = setSplitSizes(root, root.id, [3, 1]) as SplitNode;
    expect(resized.sizes).toEqual([0.75, 0.25]);
    expect(setSplitSizes(root, root.id, [-1, 2])).toBe(root);
    expect(setSplitSizes(root, "missing", [0.5, 0.5])).toBe(root);
  });

  it("findNode locates deep nodes", () => {
    const deep = pane("Deep");
    const root = createSplit("row", [createSplit("column", [deep, pane("x")]), pane("y")]);
    expect(findNode(root, deep.id)?.id).toBe(deep.id);
    expect(findNode(root, "absent")).toBeNull();
  });
});

describe("unwrapSingleChildSplits", () => {
  it("flattens degenerate structures without touching healthy ones", () => {
    const healthy = createSplit("row", [pane("A"), pane("B")]);
    expect(unwrapSingleChildSplits(healthy)).toBe(healthy);
    expect(unwrapSingleChildSplits(pane("Leaf")).kind).toBe("pane");

    const degenerate = createSplit("row", [
      createSplit("column", [pane("Inner"), pane("Keep")]),
      pane("Remove me"),
    ]);
    const removed = removePane(degenerate, ((degenerate as SplitNode).children[1] as PaneNode).id);
    expect(removed.root.kind).toBe("split");
  });
});

describe("subtreeAxisMinPx", () => {
  it("uses declared minimums or the engine default", () => {
    const constrained = pane("C", { minWidthPx: 240, minHeightPx: 90 });
    expect(subtreeAxisMinPx(constrained, "row")).toBe(240);
    expect(subtreeAxisMinPx(constrained, "column")).toBe(90);
    expect(subtreeAxisMinPx(pane("Free"), "row")).toBe(DEFAULT_MIN_PANE_PX);
  });

  it("sums recursively across nested splits and floors tiny splits", () => {
    const wide = pane("Wide", { minWidthPx: 400 });
    const narrow = pane("Narrow", { minWidthPx: 150 });
    const inner = createSplit("row", [wide, narrow]);
    const root = createSplit("row", [inner, pane("Tail")]);
    // Inner contributes 550; Tail contributes the default 120.
    expect(subtreeAxisMinPx(root, "row")).toBe(550 + DEFAULT_MIN_PANE_PX);

    const collapsedChild = pane("Zero", { minWidthPx: 0, minHeightPx: 0 });
    const tinySplit: SplitNode = {
      kind: "split",
      id: "tiny",
      direction: "row",
      children: [collapsedChild],
      sizes: [1],
    };
    expect(subtreeAxisMinPx(tinySplit, "row")).toBe(MIN_SPLIT_PX);
  });
});
