/**
 * @module @marula-core/layout — multi-pane layout engine public API.
 *
 * Purpose: the complete import surface for host apps: tree types and
 * factories, pure constraint math and tree operations, serialization with
 * defensive repair, pluggable persistence adapters, and the React controller
 * plus renderer.
 * Non-purpose: no domain knowledge of any kind — this package arranges panes
 * and knows nothing about tickets, timers, or Meridian's product surfaces.
 */

// Types
export {
  LAYOUT_STATE_VERSION,
  type LayoutNode,
  type NodeId,
  type PaneNode,
  type PaneTab,
  type SerializedLayoutNode,
  type SerializedLayoutState,
  type SizeBounds,
  type SplitDirection,
  type SplitNode,
  type TabId,
} from "./types";

// Pure model
export {
  DEFAULT_MIN_PANE_PX,
  MIN_SPLIT_PX,
  applyPairResizePx,
  clamp,
  dividerRangePx,
  fractionsToPx,
  pxToFractions,
  renormalizeFractions,
  sum,
} from "./constraints";
export {
  activateTab,
  closeTab,
  createId,
  createPane,
  createSplit,
  equalFractions,
  findNode,
  findPane,
  insertPane,
  moveTab,
  openTab,
  removePane,
  setSplitSizes,
  subtreeAxisMinPx,
  unwrapSingleChildSplits,
  updateNode,
  visitTree,
} from "./tree";

// Serialization
export {
  LayoutStateDecodeError,
  decodeLayoutState,
  encodeLayoutState,
  type DecodeResult,
} from "./serialize";

// Persistence
export {
  LocalStoragePanelStateStore,
  MemoryPanelStateStore,
  PanelPersistError,
  StorageUnavailableError,
  type LocalStorageStoreOptions,
  type PanelStateStore,
  type StorageLike,
} from "./persist";

// React layer
export { usePanelLayout, type PanelLayoutController, type UsePanelLayoutOptions } from "./react/use-panel-layout";
export { PanelLayout, type PanelLayoutProps } from "./react/panel-layout";
export { Divider, type DividerProps } from "./react/divider";
export { TabsStrip, type TabsStripProps } from "./react/tabs";
