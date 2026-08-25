/**
 * @module Tab strip rendered inside a pane.
 *
 * Purpose: display a pane's tabs with full keyboard operation (roving
 * tabindex, automatic activation on arrow keys, Home/End), tabpanel ARIA
 * wiring, and an optional per-tab close affordance. Pure presentation — all
 * mutations go through callbacks owned by the layout controller.
 * Non-purpose: no tab drag-and-drop between panes; programmatic moves are
 * exposed by the controller instead.
 */

import { useRef } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { PaneNode } from "../types";

export interface TabsStripProps {
  pane: PaneNode;
  /** Activates one tab through the layout controller. */
  onActivate(tabId: string): void;
  /** Closes one tab through the layout controller. */
  onClose(tabId: string): void;
}

export function TabsStrip({ pane, onActivate, onClose }: TabsStripProps) {
  const tabRefs = useRef<Array<HTMLDivElement | null>>([]);

  if (pane.tabs.length === 0) return null;

  const activeIndex = Math.max(
    0,
    pane.tabs.findIndex((tab) => tab.id === pane.activeTabId),
  );

  const focusAndActivate = (index: number) => {
    const bounded = Math.max(0, Math.min(index, pane.tabs.length - 1));
    const tab = pane.tabs[bounded];
    if (tab === undefined) return;
    onActivate(tab.id);
    tabRefs.current[bounded]?.focus();
  };

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    let next: number | null = null;
    if (event.key === "ArrowRight") next = activeIndex + 1;
    else if (event.key === "ArrowLeft") next = activeIndex - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = pane.tabs.length - 1;
    if (next === null) return;
    event.preventDefault();
    focusAndActivate(next);
  };

  return (
    <div className="meridian-tabs" role="tablist" aria-label="Panes" onKeyDown={handleKeyDown}>
      {pane.tabs.map((tab, index) => (
        <div
          key={tab.id}
          ref={(element) => {
            tabRefs.current[index] = element;
          }}
          role="tab"
          id={`meridian-tab-${tab.id}`}
          aria-selected={tab.id === pane.activeTabId}
          aria-controls={`meridian-panel-${pane.id}`}
          tabIndex={index === activeIndex ? 0 : -1}
          className="meridian-tab"
          data-active={tab.id === pane.activeTabId ? "true" : undefined}
          onClick={() => onActivate(tab.id)}
        >
          <span className="meridian-tab__title">{tab.title}</span>
          <button
            type="button"
            className="meridian-tab__close"
            aria-label={`Close ${tab.title || "tab"}`}
            onClick={(event) => {
              event.stopPropagation();
              onClose(tab.id);
            }}
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
