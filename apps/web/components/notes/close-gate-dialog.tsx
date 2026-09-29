// Tasks-gate dialog — the close gate's centered decision UI (docs/notes-
// tasks-workbar.md §4.2). Rendered on the typed 409 `tasks_unassigned`
// from ANY closing surface (raw API callers get the same server rule).
//
// Two spec-required resolution paths, exact: Option A "Close all tasks
// automatically" (one batched envelope completes every listed item +
// appends each REPORTED auto note, then re-issues the original close) and
// Option B "Manually assign statuses and notes to each remaining task"
// (navigational: opens the Notes & Tasks panel pre-scrolled to the first
// unassigned row; NO silent auto-continue — the close re-attempt is
// manual, deliberate). "Not now" cancels; closing never proceeds
// implicitly. Option B is the emphasized first button on purpose:
// automatic status writing is a convenience, not the default ethic.
//
// Non-purpose: no editing of item states here beyond the generated batch;
// the strip is the surface for individual decisions.

"use client";

import type { components } from "@marula-core/sdk";
import { useEffect, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { useDialogA11y } from "../../lib/a11y/use-dialog-a11y";
import { announce } from "../../components/a11y/live-region";
import { requestWorkbarPanel } from "./workbar-bridge";

type TaskGateItem = components["schemas"]["TaskGateItem"];

export interface GateBlocks {
  ticketId: string;
  ticketNumber: string;
  /** Unassigned items from the typed 409, in checklist order. */
  items: TaskGateItem[];
  /** REQUIRED-item texts blocked by the pre-close gate, when BOTH gates
   * fired together (§4.1: response carries whichever blocked first; UI
   * renders both surfaces stacked). */
  alsoRequired: string[];
  /** The close re-issue target (the statusId that was attempted). */
  statusId: string;
  /** Re-issues the ORIGINAL close after Option A's batch writes. */
  onRetryClose(statusId: string): Promise<void>;
  onDismiss(): void;
}

export function TasksGateDialog({
  ticketId,
  ticketNumber,
  items,
  alsoRequired,
  statusId,
  onRetryClose,
  onDismiss,
}: GateBlocks): ReactNode {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Modal semantics: trap + Escape + focus restore via the shared hook.
  const dialogRef = useDialogA11y<HTMLDivElement>(true, onDismiss);
  useEffect(() => {
    if (error) announce(error, "assertive");
  }, [error]);

  async function closeAllAutomatically(): Promise<void> {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const { error: batchError } = await api.api.POST("/api/v1/tickets/{id}/checklist/complete-all", {
        params: { path: { id: ticketId } },
        body: {
          task_ids: items.map((i) => i.item_id),
          instant: { idempotency_key: `close-all:${ticketId}:${new Date().toISOString().slice(0, 19)}` },
        },
      });
      if (batchError) {
        setError("The batch close did not land — nothing was changed. Try again.");
        return;
      }
      await onRetryClose(statusId);
      onDismiss();
    } catch {
      setError("The batch close did not land — the ticket stays open. Try again.");
    } finally {
      setBusy(false);
    }
  }

  function goManual(): void {
    // Navigational Option B: panel pre-scrolled to the first unassigned
    // row's action cluster; user works down the list. The panel follows
    // the CURRENT session; the close re-attempt is manual afterwards.
    requestWorkbarPanel({
      ticketId,
      focusItemId: items[0]?.item_id ?? undefined,
      intent: "tasks",
    });
    onDismiss();
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ background: "var(--m-scrim)" }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="tasks-gate-title"
      data-testid="tasks-gate-dialog"
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        className="grid w-full max-w-md gap-3 rounded-xl border p-4 outline-none"
        style={{ background: "var(--m-overlay-surface)", borderColor: "var(--m-border)", boxShadow: "var(--m-shadow-overlay)" }}
      >
        <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
          Tasks still open on {ticketNumber}
        </h2>
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          Every task needs a completed / N-A status before this ticket can close.
        </p>
        {alsoRequired.length > 0 && (
          <p className="text-[11px]" style={{ color: "var(--m-warning)" }}>
            Also blocking: {alsoRequired.length} required checklist item{alsoRequired.length === 1 ? "" : "s"} still open.
          </p>
        )}
        <ul className="m-0 max-h-48 list-none space-y-1 overflow-y-auto" aria-label="Unassigned tasks">
          {items.map((i) => (
            <li key={i.item_id} className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text)" }}>
              <span aria-hidden className="flex h-4 w-4 items-center justify-center rounded-[3px] border text-[9px]" style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}>
                ☐
              </span>
              <span className="min-w-0 flex-1 truncate">{i.text}</span>
              {i.required && (
                <span className="rounded px-1 text-[9px] uppercase" style={{ background: "var(--m-bg-subtle)", color: "var(--m-text-muted)" }}>
                  Required
                </span>
              )}
            </li>
          ))}
        </ul>
        {error !== null && (
          <p className="text-[11px]" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
            {error}
          </p>
        )}
        <div className="grid gap-2">
          <button
            type="button"
            data-testid="tasks-gate-manual"
            disabled={busy}
            className="cursor-pointer rounded-lg border px-3 py-2 text-xs font-semibold"
            style={{ borderColor: "var(--m-accent)", color: "var(--m-on-accent)", background: "var(--m-accent)" }}
            onClick={goManual}
          >
            Manually assign statuses and notes to each remaining task
          </button>
          <button
            type="button"
            data-testid="tasks-gate-close-all"
            disabled={busy}
            className="cursor-pointer rounded-lg border px-3 py-2 text-xs"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text)" , background: "var(--m-bg-subtle)" }}
            onClick={() => void closeAllAutomatically()}
          >
            Close all tasks automatically
            <span className="block pt-0.5 text-[10px] font-normal" style={{ color: "var(--m-text-muted)" }}>
              This marks {items.length} task{items.length === 1 ? "" : "s"} complete and adds a note for each. Closed by: auto-close at close gate.
            </span>
          </button>
          <button
            type="button"
            data-testid="tasks-gate-not-now"
            disabled={busy}
            className="cursor-pointer border-0 bg-transparent py-1 text-[11px]"
            style={{ color: "var(--m-text-muted)" }}
            onClick={onDismiss}
          >
            Not now
          </button>
        </div>
      </div>
    </div>
  );
}

