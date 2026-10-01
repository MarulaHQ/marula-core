// Compact ticket status bar.
//
// Purpose: one slim bar at the top of the ticket detail carrying the
// current status as color + text (Workbench status tokens, light/dark
// parity) plus the SLA state, with status transitions happening FROM the
// bar (popover grouped by semantic category). Replaces the mid-ticket
// status block. Colors follow PSA semantics: amber waiting, green closed,
// red reserved for SLA breaches.
// Non-purpose: no board/status editing (admin surface owns that).
"use client";

import type { components } from "@marula-core/sdk";
import { Button, StatusTag } from "@marula-core/ui";
import { useEffect, useRef, useState, type ReactNode } from "react";

type BoardStatus = components["schemas"]["BoardStatus"];
type BoardTransitionsResponse = components["schemas"]["BoardTransitionsResponse"];
type TicketSlaStatus = components["schemas"]["TicketSlaStatus"];

const CATEGORY_GROUPS: {
  category: NonNullable<BoardStatus["category"]>;
  label: string;
  tone: "neutral" | "warning" | "success";
}[] = [
  { category: "open", label: "Open", tone: "neutral" },
  { category: "in_progress", label: "In Progress", tone: "neutral" },
  { category: "waiting", label: "Waiting", tone: "warning" },
  { category: "closed", label: "Closed", tone: "success" },
];

export function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
}

/// Client mirror of the server's transition verdict (core-domain `workflow`):
/// explicit matrix wins; otherwise the semantic floor governs — any→any
/// except identity and closed→open. The API remains authoritative.
export function transitionAllowed(
  statuses: BoardStatus[],
  workflow: BoardTransitionsResponse,
  fromId: string,
  toId: string,
): boolean {
  if (fromId === toId) return false;
  if ((workflow.mode ?? "floor") === "explicit") {
    return (workflow.edges ?? []).some(
      (e) => e.from_status_id === fromId && e.to_status_id === toId,
    );
  }
  const catOf = new Map(statuses.map((s) => [s.id, s.category]));
  const fromCat = catOf.get(fromId);
  const toCat = catOf.get(toId);
  if (fromCat === undefined || toCat === undefined) return false;
  return !(fromCat === "closed" && toCat === "open");
}

function formatMins(mins: number): string {
  if (mins >= 60) {
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m === 0 ? `${h}h` : `${h}h ${m}m`;
  }
  return `${mins}m`;
}

/// SLA badge state: amber when the clock is running low, red only for an
/// actual breach. Colors come from the shared Workbench status tokens.
export function slaBadge(sla: TicketSlaStatus): { tone: "neutral" | "warning" | "danger"; label: string } | null {
  if (sla.breached) return { tone: "danger", label: "SLA breached" };
  const mins = sla.minutes_remaining;
  if (mins === null || mins === undefined) return null;
  if (mins < 0) return { tone: "danger", label: `SLA over ${formatMins(-mins)}` };
  const label = `${sla.paused ? "SLA paused · " : ""}${formatMins(mins)} left`;
  if (mins <= 60 && !sla.paused) return { tone: "warning", label };
  return { tone: "neutral", label };
}

export interface TicketStatusBarProps {
  number: string;
  title: string;
  statusId: string | null;
  statuses: BoardStatus[];
  workflow: BoardTransitionsResponse;
  sla: TicketSlaStatus | null;
  busy?: boolean;
  onChangeStatus(statusId: string): void;
  /**
   * Opens the board-move flow from the status menu. Omit to hide the
   * affordance (no permission / merged ticket).
   */
  onMoveBoard?: () => void;
}

/** Chevron glyph for the transition trigger. */
function ChevronIcon(): ReactNode {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

export function TicketStatusBar({
  number,
  title,
  statusId,
  statuses,
  workflow,
  sla,
  busy,
  onChangeStatus,
  onMoveBoard,
}: TicketStatusBarProps): ReactNode {
  const [open, setOpen] = useState(false);
  const popRef = useRef<HTMLDivElement | null>(null);
  const current = statuses.find((s) => s.id === statusId) ?? null;
  const badge = sla === null ? null : slaBadge(sla);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent): void {
      if (popRef.current !== null && !popRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(e: KeyboardEvent): void {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const tone = categoryTone(current?.category ?? "open");

  return (
    <div
      data-testid="ticket-status-bar"
      className="flex items-center gap-2 border-b px-3 py-1.5"
      style={{
        borderColor: "var(--m-border)",
        background: `var(--m-status-${tone === "success" ? "success" : tone === "warning" ? "warning" : "neutral"}-bg)`,
      }}
    >
      <span
        aria-hidden
        className="h-2.5 w-2.5 shrink-0 rounded-full"
        style={{
          background: `var(--m-status-${tone === "success" ? "success" : tone === "warning" ? "warning" : "neutral"}-fg)`,
        }}
      />
      <span className="shrink-0 whitespace-nowrap text-xs" style={{ color: "var(--m-text-muted)" }}>
        {number}
      </span>
      <span
        data-testid="ticket-status-text"
        className="min-w-0 truncate text-xs font-medium"
        style={{ color: `var(--m-status-${tone === "success" ? "success" : tone === "warning" ? "warning" : "neutral"}-fg)` }}
      >
        {current?.name ?? "Unknown status"}
      </span>
      {badge !== null && <StatusTag tone={badge.tone}>{badge.label}</StatusTag>}
      <span className="min-w-0 flex-1 truncate text-xs" style={{ color: "var(--m-text-secondary)" }} title={title}>
        {title}
      </span>
      <div className="relative shrink-0" ref={popRef}>
        <Button
          size="sm"
          variant="ghost"
          disabled={busy}
          aria-haspopup="menu"
          aria-expanded={open}
          data-testid="ticket-status-change"
          onClick={() => setOpen((v) => !v)}
        >
          Change status <ChevronIcon />
        </Button>
        {open && (
          <div
            role="menu"
            aria-label="Move to status"
            className="absolute right-0 top-full z-30 mt-1 grid max-h-80 min-w-56 content-start gap-2 overflow-y-auto rounded-[var(--m-radius-container)] border p-2 shadow-md"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", transition: "opacity 100ms ease" }}
          >
            {CATEGORY_GROUPS.map(({ category, label, tone: groupTone }) => {
              const group = statuses.filter((s) => s.category === category);
              if (group.length === 0) return null;
              return (
                <div key={category} className="grid gap-1">
                  <p
                    className="text-[10px] font-semibold uppercase tracking-wide"
                    style={{
                      color:
                        groupTone === "success"
                          ? "var(--m-status-success-fg)"
                          : groupTone === "warning"
                            ? "var(--m-status-warning-fg)"
                            : "var(--m-text-muted)",
                    }}
                  >
                    {label}
                  </p>
                  <div className="flex flex-wrap gap-1">
                    {group.map((s) => {
                      const isActive = s.id === statusId;
                      const allowed =
                        isActive ||
                        (statusId !== null &&
                          transitionAllowed(statuses, workflow, statusId, s.id));
                      return (
                        <Button
                          key={s.id}
                          size="sm"
                          variant={isActive ? "secondary" : "ghost"}
                          disabled={!allowed}
                          title={allowed ? undefined : "This board's workflow does not allow that transition"}
                          onClick={() => {
                            setOpen(false);
                            if (!isActive) onChangeStatus(s.id);
                          }}
                        >
                          {s.name}
                        </Button>
                      );
                    })}
                  </div>
                </div>
              );
            })}
            {onMoveBoard !== undefined ? (
              <div className="mt-1 border-t pt-2" style={{ borderColor: "var(--m-border)" }}>
                <Button
                  size="sm"
                  variant="ghost"
                  data-testid="ticket-move-board"
                  onClick={() => {
                    setOpen(false);
                    onMoveBoard();
                  }}
                >
                  Move to another board…
                </Button>
              </div>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
