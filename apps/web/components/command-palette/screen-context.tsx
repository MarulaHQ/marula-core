// Screen context for the chat panel and the palette's Ask mode.
//
// Purpose: say what the chat can see. v1 derives the surface from the route,
// and accepts a detail a page can publish later (its view name, visible row
// count, selection count) without the palette reaching into the page. The
// provider is deliberately small: pages that do not feed it still produce an
// honest chip ("This screen: Tickets"), and no count is ever invented.
//
// Non-purpose: no data fetching and no DOM scraping. A number that appears in
// the chip is a number a page declared.

"use client";

import { usePathname } from "next/navigation";
import { useEffect, useSyncExternalStore } from "react";

export type ScreenContext = {
  /** The surface's name, e.g. "Tickets". */
  label: string;
  /** The named view on that surface, e.g. "My queue". */
  view?: string;
  /** Rows the person can see right now. */
  rowCount?: number;
  /** Rows the person has selected right now. */
  selectedCount?: number;
};

const ROUTE_LABELS: ReadonlyArray<readonly [string, string]> = [
  ["/today", "Today"],
  ["/tickets", "Tickets"],
  ["/clients", "Clients"],
  ["/assets", "Assets"],
  ["/stock", "Stock"],
  ["/purchasing", "Purchasing"],
  ["/kb", "Knowledge"],
  ["/insight", "Insight"],
  ["/dispatch", "Dispatch"],
  ["/boards", "Boards"],
  ["/approvals", "Approvals"],
  ["/billing", "Billing"],
  ["/quotes", "Quotes"],
  ["/contracts", "Contracts"],
  ["/projects", "Projects"],
  ["/calendar", "Calendar"],
  ["/reports", "Reports"],
  ["/dashboards", "Dashboards"],
  ["/settings", "Settings"],
  ["/search", "Search"],
];

/** The route's surface name; falls back to the first path segment. */
export function screenLabelFor(pathname: string): string {
  for (const [prefix, label] of ROUTE_LABELS) {
    if (pathname === prefix || pathname.startsWith(`${prefix}/`)) return label;
  }
  const first = pathname.split("/").filter(Boolean)[0];
  if (first === undefined) return "Marula";
  return first.charAt(0).toUpperCase() + first.slice(1);
}

type Override = { owner: number; context: ScreenContext };
let override: Override | null = null;
let ownerSeq = 0;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getOverride(): Override | null {
  return override;
}

function getServerOverride(): Override | null {
  return null;
}

/**
 * Publish the current screen's detail. Call it from a page that owns the
 * data; the cleanup clears the context so a stale count cannot follow the
 * person to another surface. The last registered page wins (one shell page is
 * mounted at a time).
 */
export function useScreenContextRegistration(context: ScreenContext | null): void {
  const values = context;
  useEffect(() => {
    if (values === null) return;
    ownerSeq += 1;
    const owner = ownerSeq;
    override = { owner, context: values };
    emit();
    return () => {
      if (override?.owner === owner) {
        override = null;
        emit();
      }
    };
    // The object identity changes every render on purpose: a count is state.
  }, [values]);
}

/** Imperative form for a caller that is not a component. */
export function setScreenContext(context: ScreenContext | null): void {
  ownerSeq += 1;
  override = context === null ? null : { owner: ownerSeq, context };
  emit();
}

/** The context the chip renders: a page's declaration beats the route guess. */
export function useScreenContext(): ScreenContext {
  const pathname = usePathname();
  const published = useSyncExternalStore(subscribe, getOverride, getServerOverride);
  if (published !== null) return published.context;
  return { label: screenLabelFor(pathname) };
}

/** The chip's fragments, in the pack's order: surface, view, rows, selected. */
export function describeScreenContext(context: ScreenContext): string[] {
  const parts = [context.label];
  if (context.view !== undefined && context.view !== "") parts.push(context.view);
  if (context.rowCount !== undefined) {
    parts.push(`${context.rowCount} ${context.rowCount === 1 ? "row" : "rows"}`);
  }
  if (context.selectedCount !== undefined && context.selectedCount > 0) {
    parts.push(`${context.selectedCount} selected`);
  }
  return parts;
}

/** The sentence a turn carries as its system prompt. */
export function contextPrompt(context: ScreenContext): string {
  const parts = describeScreenContext(context);
  return `You assist a service-desk technician. The person is on this screen: ${parts.join(" · ")}. Answer from what this screen shows, and say when you cannot see something.`;
}
