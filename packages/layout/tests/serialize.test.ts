/**
 * @module Tests for serialization: encode/decode round-trips and the repair
 * of hostile or stale persisted payloads.
 */

import { describe, expect, it } from "vitest";
import {
  LAYOUT_STATE_VERSION,
  type LayoutNode,
  type PaneNode,
  type SplitNode,
} from "../src/types";
import { createPane, createSplit } from "../src/tree";
import { LayoutStateDecodeError, decodeLayoutState, encodeLayoutState } from "../src/serialize";

function paneWithTab(title = "Tickets"): PaneNode {
  const pane = createPane({ tabs: [title] });
  return { ...pane, activeTabId: pane.tabs[0]?.id ?? null };
}

describe("encodeLayoutState / decodeLayoutState round trip", () => {
  it("preserves a nested tree exactly", () => {
    const sidebar = paneWithTab("Clients");
    const editor = createSplit("column", [paneWithTab("Ticket"), paneWithTab("Timer")], [0.7, 0.3]);
    const root = createSplit("row", [sidebar, editor], [0.25, 0.75]);

    const decoded = decodeLayoutState(encodeLayoutState(root));
    expect(decoded.warnings).toEqual([]);
    expect(decoded.state.version).toBe(LAYOUT_STATE_VERSION);
    expect(decoded.state.root).toEqual(root);
  });

  it("emits JSON with rounded fractions", () => {
    const root = createPane({ tabs: ["A"] });
    const raw = encodeLayoutState(root);
    expect(raw).not.toContain("0.33333333333");
    expect(() => JSON.parse(raw)).not.toThrow();
  });

  it("stamps a savedAt timestamp when omitted", () => {
    const state = JSON.parse(encodeLayoutState(paneWithTab())) as { savedAt: string };
    expect(Number.isNaN(Date.parse(state.savedAt))).toBe(false);
  });
});

describe("decodeLayoutState repair behavior", () => {
  const cloneRecord = (value: unknown): Record<string, unknown> =>
    JSON.parse(JSON.stringify(value)) as Record<string, unknown>;
  const basePane = (): Record<string, unknown> => cloneRecord(paneWithTab("A"));
  const envelope = (root: unknown): Record<string, unknown> => ({
    version: 1,
    savedAt: "2026-01-01T00:00:00Z",
    root,
  });

  it("rejects non-JSON strings", () => {
    expect(() => decodeLayoutState("{nope")).toThrow(LayoutStateDecodeError);
  });

  it("rejects envelopes without a usable root", () => {
    expect(() => decodeLayoutState({ version: 1, savedAt: "2026-01-01T00:00:00Z" })).toThrow(
      LayoutStateDecodeError,
    );
    expect(() => decodeLayoutState("just a string")).toThrow(LayoutStateDecodeError);
  });

  it("repairs missing sizes with equal fractions and warns", () => {
    const inner = createSplit("column", [basePane(), basePane()] as unknown as LayoutNode[]);
    const root: Record<string, unknown> = JSON.parse(JSON.stringify(inner));
    delete root["sizes"];
    const { state, warnings } = decodeLayoutState(envelope(root));
    const split = state.root as SplitNode;
    expect(split.sizes).toEqual([0.5, 0.5]);
    expect(warnings.some((warning) => warning.includes("sizes"))).toBe(true);
  });

  it("resets wrong-length or non-positive sizes", () => {
    const inner = createSplit(
      "row",
      [basePane(), basePane(), basePane()] as unknown as LayoutNode[],
      [0.5, 0.25, 0.25],
    );
    const broken = { ...cloneRecord(inner), sizes: [2, -1] } as unknown;
    const { state } = decodeLayoutState(envelope(broken));
    expect((state.root as SplitNode).sizes).toEqual([1 / 3, 1 / 3, 1 / 3]);
  });

  it("defaults an invalid direction to row", () => {
    const inner = createSplit("column", [basePane(), basePane()] as unknown as LayoutNode[]);
    const broken = { ...cloneRecord(inner), direction: "diagonal" } as unknown;
    const { state, warnings } = decodeLayoutState(envelope(broken));
    expect((state.root as SplitNode).direction).toBe("row");
    expect(warnings.some((warning) => warning.includes("direction"))).toBe(true);
  });

  it("collapses single-child splits recursively up to the root", () => {
    const survivor = basePane();
    const innermost = { kind: "split", id: "s3", direction: "row", children: [survivor] };
    const middle = { kind: "split", id: "s2", direction: "row", children: [innermost] };
    const { state, warnings } = decodeLayoutState({
      version: 1,
      savedAt: "2026-01-01T00:00:00Z",
      root: { kind: "split", id: "s1", direction: "row", children: [middle] },
    });
    expect((state.root as PaneNode).kind).toBe("pane");
    expect(warnings.filter((warning) => warning.includes("collapsed"))).toHaveLength(3);
  });

  it("rejects state whose root split lost every child", () => {
    expect(() =>
      decodeLayoutState({
        version: 1,
        savedAt: "2026-01-01T00:00:00Z",
        root: {
          kind: "split",
          id: "s1",
          direction: "row",
          children: [{ kind: "split", id: "s2", direction: "row", children: [] }],
        },
      }),
    ).toThrow(LayoutStateDecodeError);
  });

  it("regenerates duplicate node ids", () => {
    const first = basePane();
    const second = basePane();
    second["id"] = first["id"];
    const { state, warnings } = decodeLayoutState({
      version: 1,
      savedAt: "2026-01-01T00:00:00Z",
      root: { kind: "split", id: "s1", direction: "row", children: [first, second] },
    });
    const children = (state.root as SplitNode).children;
    expect(children[0]?.id).not.toBe(children[1]?.id);
    expect(warnings.some((warning) => warning.includes("duplicate id"))).toBe(true);
  });

  it("clears dangling activeTabId references", () => {
    const pane = basePane();
    pane["activeTabId"] = "ghost-tab";
    const { state, warnings } = decodeLayoutState(envelope(pane));
    expect((state.root as PaneNode).activeTabId).toBeNull();
    expect(warnings.some((warning) => warning.includes("activeTabId"))).toBe(true);
  });

  it("drops malformed tabs but keeps valid ones", () => {
    const pane = basePane();
    pane["tabs"] = [
      { id: "t1", title: "Keep" },
      "not-an-object",
      { title: "missing id" },
    ];
    const { state, warnings } = decodeLayoutState(envelope(pane));
    const restored = state.root as PaneNode;
    // The malformed entry is dropped; the id-less tab survives with a fresh id.
    expect(restored.tabs.map((tab) => tab.title)).toEqual(["Keep", "missing id"]);
    expect(restored.tabs[1]?.id).toMatch(/^tab-/);
    expect(warnings.some((warning) => warning.includes("generated replacement"))).toBe(true);
  });

  it("drops invalid minimum extents and keeps valid ones", () => {
    const pane = basePane();
    pane["minWidthPx"] = -5;
    pane["minHeightPx"] = 240;
    const { state } = decodeLayoutState(envelope(pane));
    const restored = state.root as PaneNode;
    expect(restored.minWidthPx).toBeUndefined();
    expect(restored.minHeightPx).toBe(240);
  });

  it("warns on unknown versions yet still parses", () => {
    const pane = basePane();
    pane["kind"] = "pane";
    const { state, warnings } = decodeLayoutState({
      version: 99,
      savedAt: "2026-01-01T00:00:00Z",
      root: pane,
    });
    expect(state.version).toBe(LAYOUT_STATE_VERSION);
    expect(warnings.some((warning) => warning.includes("version"))).toBe(true);
  });

  it("replaces invalid savedAt values", () => {
    const pane = basePane();
    const { state, warnings } = decodeLayoutState({
      version: 1,
      savedAt: "not-a-date",
      root: pane,
    });
    expect(Number.isNaN(Date.parse(state.savedAt))).toBe(false);
    expect(warnings.some((warning) => warning.includes("savedAt"))).toBe(true);
  });
});

describe("decode output feeds the renderer contract", () => {
  it("produces a tree every node of which satisfies engine invariants", () => {
    const messy: unknown = {
      version: 1,
      savedAt: "2026-08-25T12:00:00.000Z",
      root: {
        kind: "split",
        id: "root",
        direction: "row",
        sizes: [10, "bad", 30],
        children: [
          { kind: "pane", id: "p1", tabs: [{ id: "t1", title: "One" }], activeTabId: "t1" },
          { kind: "bogus" },
          {
            kind: "split",
            id: "nested",
            direction: "column",
            children: [{ kind: "pane", id: "p2", tabs: [], activeTabId: null }],
          },
        ],
      },
    };
    const { state } = decodeLayoutState(messy);
    const assertInvariants = (node: LayoutNode): void => {
      if (node.kind === "split") {
        expect(node.children.length).toBe(node.sizes.length);
        expect(node.children.length).toBeGreaterThanOrEqual(2);
        const total = node.sizes.reduce((sum, size) => sum + size, 0);
        expect(total).toBeCloseTo(1, 6);
        node.children.forEach(assertInvariants);
      }
    };
    assertInvariants(state.root);
    expect((state.root as SplitNode).children).toHaveLength(2);
  });
});
