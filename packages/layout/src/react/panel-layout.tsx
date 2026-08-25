/**
 * @module `PanelLayout` — the React renderer for a pane tree.
 *
 * Purpose: turn the controller's tree into CSS grid DOM (nested rows/columns,
 * `minmax()` tracks as a CSS-level backstop for minimum sizes, dividers
 * between children), render tab strips and delegated pane content, and apply
 * the narrow-viewport collapse (top-level row splits stack into columns below
 * `collapseBelowPx`, measured with a ResizeObserver so pointer math stays in
 * sync with the rendered direction).
 * Non-purpose: no content of its own — pane bodies are host-rendered via
 * props; the engine never imports app or framework code.
 */

import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { subtreeAxisMinPx } from "../tree";
import type { LayoutNode, PaneNode, PaneTab, SplitDirection, SplitNode } from "../types";
import { Divider } from "./divider";
import { TabsStrip } from "./tabs";
import type { PanelLayoutController } from "./use-panel-layout";

export interface PanelLayoutProps {
  controller: PanelLayoutController;
  /** Renders the body of the active tab. The engine owns everything around it. */
  renderPaneContent(args: { pane: PaneNode; tab: PaneTab }): ReactNode;
  /** Renders an empty pane (no tabs). Default: a quiet placeholder. */
  renderEmptyPane?(pane: PaneNode): ReactNode;
  /** Root inline-size below which top-level rows collapse to columns. Default 720. */
  collapseBelowPx?: number;
  /** Accessible name of the layout region. */
  label?: string;
  className?: string;
}

const DEFAULT_COLLAPSE_BELOW_PX = 720;

interface RenderContext {
  controller: PanelLayoutController;
  renderPaneContent: PanelLayoutProps["renderPaneContent"];
  renderEmptyPane: PanelLayoutProps["renderEmptyPane"];
}

export function PanelLayout({
  controller,
  renderPaneContent,
  renderEmptyPane,
  collapseBelowPx = DEFAULT_COLLAPSE_BELOW_PX,
  label = "Workspace panes",
  className,
}: PanelLayoutProps) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [narrow, setNarrow] = useState(false);

  useEffect(() => {
    const element = rootRef.current;
    if (element === null || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry === undefined) return;
      setNarrow(entry.contentRect.width < collapseBelowPx);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [collapseBelowPx]);

  const context: RenderContext = { controller, renderPaneContent, renderEmptyPane };
  const root = controller.root;

  return (
    <div
      ref={rootRef}
      className={["meridian-layout", className].filter(Boolean).join(" ")}
      role="region"
      aria-label={label}
      data-collapsed={narrow ? "true" : undefined}
    >
      {root === null ? null : (
        <SplitView
          node={root}
          depth={0}
          forceDirection={
            narrow && root.kind === "split" && root.direction === "row" ? "column" : undefined
          }
          context={context}
        />
      )}
    </div>
  );
}

function SplitView(props: {
  node: LayoutNode;
  depth: number;
  forceDirection?: SplitDirection | undefined;
  context: RenderContext;
}) {
  const { node, depth, forceDirection, context } = props;
  if (node.kind === "pane") {
    return <PaneView node={node} context={context} />;
  }

  // Only the root split honors the narrow-collapse flip; nested splits keep
  // their authored direction so deep layouts stay predictable.
  const direction = depth === 0 && forceDirection !== undefined ? forceDirection : node.direction;
  const isRow = direction === "row";

  const tracks: string[] = [];
  node.children.forEach((child, index) => {
    if (index > 0) {
      tracks.push("var(--meridian-layout-divider-size)");
    }
    const minPx = subtreeAxisMinPx(child, direction);
    const fraction = Number((node.sizes[index] as number).toFixed(6));
    tracks.push(`minmax(${minPx}px, ${fraction}fr)`);
  });

  const template = tracks.join(" ");

  const items: ReactNode[] = [];
  node.children.forEach((child, index) => {
    if (index > 0) {
      items.push(
        <Divider
          key={`divider-${child.id}`}
          split={node}
          index={index - 1}
          direction={direction}
          onResize={(sizes) => context.controller.setSplitSizes(node.id, sizes)}
        />,
      );
    }
    items.push(
      child.kind === "pane" ? (
        <PaneView key={child.id} node={child} context={context} />
      ) : (
        <SplitView key={child.id} node={child} depth={depth + 1} context={context} />
      ),
    );
  });

  return (
    <div
      className="meridian-split"
      data-direction={direction}
      data-depth={depth}
      style={
        isRow
          ? { gridTemplateColumns: template, gridTemplateRows: "100%" }
          : { gridTemplateRows: template, gridTemplateColumns: "100%" }
      }
    >
      {items}
    </div>
  );
}

function PaneView({ node, context }: { node: PaneNode; context: RenderContext }) {
  const activeTab =
    node.tabs.find((tab) => tab.id === node.activeTabId) ?? null;

  return (
    <section
      id={node.id}
      className="meridian-pane"
      data-pane-id={node.id}
      style={{
        ...(node.minWidthPx !== undefined ? { minWidth: `${node.minWidthPx}px` } : {}),
        ...(node.minHeightPx !== undefined ? { minHeight: `${node.minHeightPx}px` } : {}),
      }}
    >
      <TabsStrip
        pane={node}
        onActivate={(tabId) => context.controller.activateTab(node.id, tabId)}
        onClose={(tabId) => context.controller.closeTab(node.id, tabId)}
      />
      <div
        id={`meridian-panel-${node.id}`}
        role="tabpanel"
        aria-labelledby={activeTab === null ? undefined : `meridian-tab-${activeTab.id}`}
        className="meridian-pane__body"
      >
        {activeTab !== null
          ? (context.renderPaneContent?.({ pane: node, tab: activeTab }) ?? null)
          : (context.renderEmptyPane?.(node) ?? <div className="meridian-pane__empty">Empty pane</div>)}
      </div>
    </section>
  );
}
