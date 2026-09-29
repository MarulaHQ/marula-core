/**
 * @module `usePanelLayout` — stateful layout controller with persistence.
 *
 * Purpose: own the live pane tree, apply user mutations immutably, and
 * round-trip the tree through a {@link PanelStateStore} keyed by user id
 * (debounced saves; defensive decode of whatever comes back).
 * Non-purpose: no DOM measurement or rendering — components in this package
 * consume the controller; persistence backends never touch React state.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import {
  closeTab as closeTabOp,
  activateTab as activateTabOp,
  createId,
  createPane,
  insertPane,
  moveTab as moveTabOp,
  openTab as openTabOp,
  removePane,
  setSplitSizes as setSplitSizesOp,
} from "../tree";
import { decodeLayoutState, encodeLayoutState } from "../serialize";
import {
  LocalStoragePanelStateStore,
  MemoryPanelStateStore,
  type PanelStateStore,
} from "../persist";
import type {
  LayoutNode,
  NodeId,
  PaneTab,
  SplitDirection,
  TabId,
} from "../types";

export interface UsePanelLayoutOptions {
  /** Partition key for persisted state. Switching it loads that user's layout. */
  userId: string;
  /**
   * Storage backend. Defaults to a localStorage adapter in the browser and an
   * in-memory store elsewhere. Must be referentially stable across renders.
   */
  store?: PanelStateStore;
  /** Tree used before anything is persisted or when persisted data is unusable. */
  fallback: LayoutNode;
  /** Debounce window for saves in milliseconds. Default 400. */
  saveDebounceMs?: number;
  /** Notified when loading fell back or saving failed. Default: console warning. */
  onPersistIssue?: (issue: string) => void;
}

export interface PanelLayoutController {
  /** `"loading"` until the persisted tree (or fallback) has been applied. */
  status: "loading" | "ready";
  root: LayoutNode | null;
  /** Repairs reported while decoding the last loaded payload. */
  loadWarnings: readonly string[];

  /** Replace one split's child fractions (normalized defensively). */
  setSplitSizes(splitId: NodeId, sizes: readonly number[]): void;

  /** Activate a tab within its pane. */
  activateTab(paneId: NodeId, tabId: TabId): void;
  /** Add a tab to a pane and activate it; returns the tab id actually used. */
  openTab(paneId: NodeId, tab: string | PaneTab): TabId | null;
  /** Close a tab; its pane remains even when empty. */
  closeTab(paneId: NodeId, tabId: TabId): void;
  /** Move a tab between panes (or reorder within one pane). */
  moveTab(params: { tabId: TabId; fromPaneId: NodeId; toPaneId: NodeId; index?: number }): void;

  /**
   * Insert a fresh pane beside `paneId` and return its id, or `null` when the
   * target is unknown. The new pane contains one untitled-content tab.
   */
  splitPane(
    paneId: NodeId,
    options?: { direction?: SplitDirection; position?: "before" | "after" },
  ): NodeId | null;
  /** Remove a pane; ancestors are repaired so splits keep at least two children. */
  removePane(paneId: NodeId): void;

  /** Drop persisted state and restore {@link UsePanelLayoutOptions.fallback}. */
  reset(): Promise<void>;
}

let sharedBrowserStore: PanelStateStore | undefined;

function defaultStore(): PanelStateStore {
  if (sharedBrowserStore === undefined) {
    const hasLocalStorage =
      typeof window !== "undefined" &&
      window.localStorage !== null &&
      typeof window.localStorage === "object";
    sharedBrowserStore = hasLocalStorage
      ? new LocalStoragePanelStateStore()
      : new MemoryPanelStateStore();
  }
  return sharedBrowserStore;
}

const DEFAULT_SAVE_DEBOUNCE_MS = 400;

export function usePanelLayout(options: UsePanelLayoutOptions): PanelLayoutController {
  const { userId } = options;
  const saveDebounceMs = options.saveDebounceMs ?? DEFAULT_SAVE_DEBOUNCE_MS;

  // Refs keep latest options without retriggering load/save effects.
  const storeRef = useRef<PanelStateStore>(options.store ?? defaultStore());
  storeRef.current = options.store ?? defaultStore();
  const fallbackRef = useRef<LayoutNode>(options.fallback);
  fallbackRef.current = options.fallback;
  const issueRef = useRef<(issue: string) => void>(options.onPersistIssue ?? defaultIssueReporter);
  issueRef.current = options.onPersistIssue ?? defaultIssueReporter;

  const [status, setStatus] = useState<"loading" | "ready">("loading");
  const [root, setRoot] = useState<LayoutNode | null>(null);
  const [loadWarnings, setLoadWarnings] = useState<readonly string[]>([]);
  const rootRef = useRef<LayoutNode | null>(null);
  const skipNextSaveRef = useRef(true);

  const applyRoot = useCallback((next: LayoutNode | null) => {
    rootRef.current = next;
    setRoot(next);
  }, []);

  useEffect(() => {
    let active = true;
    setStatus("loading");
    applyRoot(null);

    const finish = (nextRoot: LayoutNode, warnings: readonly string[]) => {
      if (!active) return;
      skipNextSaveRef.current = true;
      applyRoot(nextRoot);
      setLoadWarnings(warnings);
      setStatus("ready");
    };

    storeRef.current
      .load(userId)
      .then((payload) => {
        if (!active) return;
        if (payload === null) {
          finish(fallbackRef.current, []);
          return;
        }
        try {
          const { state, warnings } = decodeLayoutState(payload);
          finish(state.root, warnings);
        } catch (error) {
          issueRef.current(`layout state unusable, restored defaults (${String(error)})`);
          finish(fallbackRef.current, [String(error)]);
        }
      })
      .catch((error: unknown) => {
        issueRef.current(`layout store unavailable, restored defaults (${String(error)})`);
        finish(fallbackRef.current, [String(error)]);
      });

    return () => {
      active = false;
    };
  }, [userId, applyRoot]);

  useEffect(() => {
    if (status !== "ready" || root === null) return;
    if (skipNextSaveRef.current) {
      skipNextSaveRef.current = false;
      return;
    }
    const timer = setTimeout(() => {
      storeRef.current
        .save(userId, encodeLayoutState(root))
        .catch((error: unknown) => issueRef.current(`failed to persist layout (${String(error)})`));
    }, saveDebounceMs);
    return () => clearTimeout(timer);
  }, [root, status, userId, saveDebounceMs]);

  const mutate = useCallback(
    (transform: (current: LayoutNode) => LayoutNode) => {
      const current = rootRef.current;
      if (current === null) return;
      applyRoot(transform(current));
    },
    [applyRoot],
  );

  const setSplitSizes = useCallback(
    (splitId: NodeId, sizes: readonly number[]) => {
      mutate((current) => setSplitSizesOp(current, splitId, sizes));
    },
    [mutate],
  );

  const activateTab = useCallback(
    (paneId: NodeId, tabId: TabId) => {
      mutate((current) => activateTabOp(current, paneId, tabId));
    },
    [mutate],
  );

  const openTab = useCallback(
    (paneId: NodeId, tab: string | PaneTab): TabId | null => {
      const current = rootRef.current;
      if (current === null) return null;
      const resolved =
        typeof tab === "string" ? { id: createId("tab"), title: tab } : { ...tab };
      mutate((node) => openTabOp(node, paneId, resolved).root);
      return resolved.id;
    },
    [mutate],
  );

  const closeTab = useCallback(
    (paneId: NodeId, tabId: TabId) => {
      mutate((current) => closeTabOp(current, paneId, tabId));
    },
    [mutate],
  );

  const moveTabBetweenPanes = useCallback(
    (params: { tabId: TabId; fromPaneId: NodeId; toPaneId: NodeId; index?: number }) => {
      mutate((current) =>
        moveTabOp(current, {
          tabId: params.tabId,
          fromPaneId: params.fromPaneId,
          toPaneId: params.toPaneId,
          index: params.index,
        }),
      );
    },
    [mutate],
  );

  const splitPane = useCallback(
    (
      paneId: NodeId,
      mutationOptions?: { direction?: SplitDirection; position?: "before" | "after" },
    ): NodeId | null => {
      const current = rootRef.current;
      if (current === null || findPaneId(current, paneId) === null) return null;
      const newPane = createPane({ tabs: ["New pane"], activeTabId: undefined });
      const firstTab = newPane.tabs[0];
      if (firstTab === undefined) return null;
      const seeded = { ...newPane, activeTabId: firstTab.id };
      mutate((node) =>
        insertPane(node, paneId, seeded, {
          direction: mutationOptions?.direction ?? "row",
          position: mutationOptions?.position ?? "after",
        }),
      );
      return seeded.id;
    },
    [mutate],
  );

  const removePaneById = useCallback(
    (paneId: NodeId) => {
      mutate((current) => removePane(current, paneId).root);
    },
    [mutate],
  );

  const reset = useCallback(async () => {
    await storeRef.current.clear(userId).catch(() => undefined);
    skipNextSaveRef.current = true;
    applyRoot(fallbackRef.current);
    setLoadWarnings([]);
  }, [userId, applyRoot]);

  return {
    status,
    root,
    loadWarnings,
    setSplitSizes,
    activateTab,
    openTab,
    closeTab,
    moveTab: moveTabBetweenPanes,
    splitPane,
    removePane: removePaneById,
    reset,
  };
}

function findPaneId(root: LayoutNode, paneId: NodeId): NodeId | null {
  if (root.kind === "pane") return root.id === paneId ? root.id : null;
  for (const child of root.children) {
    const found = findPaneId(child, paneId);
    if (found !== null) return found;
  }
  return null;
}

function defaultIssueReporter(issue: string): void {
  console.warn(`[marula/layout] ${issue}`);
}
