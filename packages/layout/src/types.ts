/**
 * @module Public type vocabulary of the layout engine.
 *
 * Purpose: define the declarative pane tree (nested row/column splits with
 * tabbed panes), the versioned serialization envelope, and the size-constraint
 * primitives shared by the math, persistence, and React layers.
 * Non-purpose: no runtime logic and no React types live here — behavior sits
 * in `tree.ts`, `constraints.ts`, `serialize.ts`, and `react/`.
 */

/** Identifier of any node in the pane tree. Unique within one tree instance. */
export type NodeId = string;

/** Identifier of a tab inside a pane. Unique within its pane. */
export type TabId = string;

/**
 * Axis of a split node.
 * `row` lays children left-to-right (vertical dividers),
 * `column` stacks children top-to-bottom (horizontal dividers).
 */
export type SplitDirection = "row" | "column";

/** One content slot inside a pane's tab strip. Content rendering is delegated to the host app. */
export interface PaneTab {
  id: TabId;
  /** Human label shown in the tab strip. Rendered as plain text, never HTML. */
  title: string;
}

/**
 * A leaf pane holding a tab strip plus the active tab's content.
 * Carries minimum-size constraints along both axes; the constraint solver
 * picks the axis-relevant value for each enclosing split.
 */
export interface PaneNode {
  kind: "pane";
  id: NodeId;
  tabs: readonly PaneTab[];
  /** Currently displayed tab; `null` when the pane is empty or nothing is active. */
  activeTabId: TabId | null;
  /** Minimum width in CSS pixels when constrained along a horizontal axis. */
  minWidthPx?: number | undefined;
  /** Minimum height in CSS pixels when constrained along a vertical axis. */
  minHeightPx?: number | undefined;
}

/**
 * An interior split node. Children are laid out along `direction`; `sizes`
 * holds one positive fraction per child, summing to approximately 1.
 * Fractions (not pixels) keep persisted layouts resolution-independent.
 */
export interface SplitNode {
  kind: "split";
  id: NodeId;
  direction: SplitDirection;
  children: readonly LayoutNode[];
  sizes: readonly number[];
}

/** Any node of the pane tree. */
export type LayoutNode = SplitNode | PaneNode;

/** Envelope version written by this engine. Decoders accept older/unknown versions best-effort. */
export const LAYOUT_STATE_VERSION = 1 as const;

/** Serialized form of a pane tree — plain JSON, safe to hand to any store. */
export type SerializedLayoutNode = LayoutNode;

/** Versioned envelope persisted through {@link PanelStateStore}. */
export interface SerializedLayoutState {
  version: number;
  /** ISO-8601 timestamp of the last save (UTC). */
  savedAt: string;
  root: SerializedLayoutNode;
}

/**
 * Minimum/maximum extent of one child along a split axis, in CSS pixels.
 * `maxPx` is optional; an absent maximum means unbounded.
 */
export interface SizeBounds {
  minPx: number;
  maxPx?: number | undefined;
}
