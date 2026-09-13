// OutboxDock — the IF6 surfacing surface.
//
// Purpose: one quiet docked chip showing the durable offline queue count
// plus the most recent per-op accepted/rejected results (friendly copy
// from the central reason-code map when rejected — never the raw reason
// code or server JSON). Chrome-quiet per docs/theme.md: tokens only, no
// motion, dark-mode parity via the shared token set.
//
// Non-purpose: no queue management (clearing/retrying is not offered —
// replay is order-safe and automatic), no toasts.

"use client";

import { StatusTag } from "@meridian/ui";
import { useState } from "react";
import { useOutbox } from "../../lib/use-instant";
import { reasonCopy } from "../../lib/error-copy";

function outcomeTone(status: string): "success" | "danger" | "warning" {
  if (status === "accepted") return "success";
  if (status === "rejected") return "danger";
  return "warning";
}

function outcomeLabel(status: string, reason?: string): string {
  if (status === "accepted") return "saved";
  if (status === "rejected") return reasonCopy(reason ?? "conflict");
  return "queued";
}

export function OutboxDock() {
  const { pending, outcomes } = useOutbox();
  const [expanded, setExpanded] = useState(false);
  if (pending === 0 && outcomes.length === 0) return null;

  return (
    <div className="grid gap-1" data-testid="instant-outbox">
      <button
        type="button"
        className="flex items-center gap-1 text-xs"
        style={{ color: "var(--m-text-muted)" }}
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
      >
        {pending > 0 && (
          <StatusTag tone="warning">
            {`${pending} offline ${pending === 1 ? "change" : "changes"} pending`}
          </StatusTag>
        )}
        {outcomes.length > 0 && (
          <StatusTag tone={outcomeTone(outcomes[outcomes.length - 1]?.status ?? "queued")}>
            {outcomeLabel(
              outcomes[outcomes.length - 1]?.status ?? "queued",
              outcomes[outcomes.length - 1]?.reason,
            )}
          </StatusTag>
        )}
      </button>
      {expanded && (
        <ul
          className="grid max-h-40 gap-1 overflow-auto rounded-[2px] border p-2 text-xs"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          aria-label="Offline change results"
        >
          {outcomes
            .slice()
            .reverse()
            .map((o) => (
              <li key={o.key} className="flex items-center gap-2">
                <StatusTag tone={outcomeTone(o.status)}>
                  {outcomeLabel(o.status, o.reason)}
                </StatusTag>
                <span className="truncate">{o.describe}</span>
              </li>
            ))}
        </ul>
      )}
    </div>
  );
}
