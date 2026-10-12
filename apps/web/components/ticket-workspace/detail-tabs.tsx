// Detail-view tab bar + the pinned original request surface.
//
// Purpose: the ticket detail stops competing for one long column —
// Original request / Notes / Tasks / Details / Related tabs. The original
// request is ALSO pinned above the tab strip (never lost in the note
// stream): number, title, requester client line, and the request body in
// full, styled as its own surface.
// Non-purpose: no keyboard-trap management beyond roving focus defaults.

"use client";

import type { components } from "@marula-core/sdk";
import {
  Badge,
} from "@marula-core/ui";
import type { ReactNode } from "react";

type Ticket = components["schemas"]["TicketWithFields"];

export type DetailTabId = "original" | "notes" | "tasks" | "details" | "related" | "links" | "money";

export const DETAIL_TABS: { id: DetailTabId; label: string }[] = [
  { id: "original", label: "Original request" },
  { id: "notes", label: "Notes" },
  { id: "tasks", label: "Tasks" },
  { id: "details", label: "Details" },
  { id: "links", label: "Links" },
  { id: "money", label: "Money" },
  { id: "related", label: "Related" },
];

export function DetailTabs({
  active,
  onChange,
}: {
  active: DetailTabId;
  onChange(tab: DetailTabId): void;
}): ReactNode {
  return (
    <div role="tablist" aria-label="Ticket detail sections" className="flex flex-wrap gap-1 border-b pb-1" style={{ borderColor: "var(--m-border)" }}>
      {DETAIL_TABS.map((tab) => {
        const selected = active === tab.id;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={selected}
            data-testid={`detail-tab-${tab.id}`}
            className="rounded-[var(--m-radius-container)] border-b-2 px-2.5 py-1 text-xs transition-opacity duration-100"
            style={{
              borderColor: selected ? "var(--m-accent)" : "transparent",
              color: selected ? "var(--m-text)" : "var(--m-text-muted)",
              fontWeight: selected ? 600 : 400,
              opacity: selected ? 1 : 0.85,
            }}
            onClick={() => onChange(tab.id)}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}

export function OriginalRequest({ ticket }: { ticket: Ticket }): ReactNode {
  return (
    <section
      className="grid content-start gap-1.5 rounded-[var(--m-radius-container)] border p-3"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid="original-request"
      aria-label="Original request"
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          {ticket.number}
        </span>
        <Badge tone="neutral">Original request</Badge>
        <time className="ml-auto text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          {ticket.created_at !== null && ticket.created_at !== undefined
            ? new Date(ticket.created_at).toLocaleString()
            : null}
        </time>
      </div>
      <p className="whitespace-pre-wrap text-sm">{ticket.body ?? "(No request body recorded.)"}</p>
    </section>
  );
}
