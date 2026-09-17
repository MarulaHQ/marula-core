// Task-lists panel — per-ticket task items through the whole workflow.
//
// Purpose: render the ticket checklist inline (notes-pane pod), with
// per-item tri-state actions: ✓ check, ✗ uncheck, and N-A. All three ride
// the checklist toggle endpoint; `na` is ORG-WIDE server truth (Wave 12
// lane 6) and mutually exclusive with checked — setting N-A clears the
// checkmark, checking an item clears N-A. A server-side mutual-exclusion
// 422 surfaces inline next to the item (role="alert"), never a dead
// action. N-A REQUIRED items are exempt from pre-close enforcement and
// derived progress (server derives this; the panel reflects the counts).
// Task states fold into notes (see close-flow.tsx).
// Non-purpose: no checklist-template editing (board admin owns that).

"use client";

import { rejectionText } from "../../lib/error-copy";
import type { components } from "@meridian/sdk";
import { Button, Input } from "@meridian/ui";
import { useState, type ReactNode } from "react";
import { NoteComposer } from "../notes/note-composer";

type ChecklistItem = components["schemas"]["ChecklistItem"];

export interface TaskToggleOutcome {
  ok: boolean;
  message?: string;
}

export interface TasksPanelProps {
  items: ChecklistItem[];
  enforcementActive: boolean;
  blockingCount: number;
  /** Testid prefix so multiple mounted instances don't collide. */
  testPrefix?: string;
  onSetState(
    itemId: string,
    state: { checked: boolean; na: boolean },
    /** Mandatory ledger reason (Wave 17 lane 11 §3.4): server composes
     * the N-A / reopen note in the same tx; typed 422 without it. */
    noteReason?: string,
  ): Promise<TaskToggleOutcome>;
  onAdd(text: string): Promise<void>;
}

export function TasksPanel({
  items,
  enforcementActive,
  blockingCount,
  testPrefix = "",
  onSetState,
  onAdd,
}: TasksPanelProps): ReactNode {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [rowError, setRowError] = useState<string | null>(null);
  // The mandatory reason prompt (§3.4): N-A and reopen-open flips expand
  // an inline reason field under the row — the SHARED note composer, not
  // a divergent text box. Escape collapses; ⌘↵ via the composer submits.
  const [reasonAsk, setReasonAsk] = useState<{ itemId: string; kind: "na" | "reopen"; text: string } | null>(null);

  async function markNa(item: ChecklistItem, na: boolean): Promise<void> {
    setRowError(null);
    // Reason first when flipping TO N-A (mandatory, §3.4); clearing N-A
    // keeps the item unchecked so it re-enters the open set.
    if (na) {
      setReasonAsk({ itemId: item.id, kind: "na", text: "" });
      return;
    }
    const result = await onSetState(item.id, { checked: false, na: false }, undefined);
    if (!result.ok) {
      setRowError(rejectionText(result, "Could not update that item"));
    }
  }

  async function toggleChecked(item: ChecklistItem, checked: boolean): Promise<void> {
    setRowError(null);
    // Mutual exclusion client-side: checking clears a prior N-A; unchecking
    // an OPEN item is a plain state (no reason needed). Unchecking a DONE
    // or reopening... a reopen (done -> open) needs its mandatory reason.
    const na = checked ? false : (item.na ?? false);
    const reopen = (!checked && !(item.na ?? false) && item.checked);
    if (reopen) {
      setReasonAsk({ itemId: item.id, kind: "reopen", text: "" });
      return;
    }
    const result = await onSetState(item.id, { checked, na }, undefined);
    if (!result.ok) {
      setRowError(rejectionText(result, "Could not update that item"));
    }
  }

  async function add(): Promise<void> {
    const text = draft.trim();
    if (text === "") return;
    setBusy(true);
    try {
      await onAdd(text);
      setDraft("");
    } finally {
      setBusy(false);
    }
  }

  /** The close flow's resolved list: checked items that are NOT N-A'd. */
  const resolvedForNotes = items.filter((i) => i.checked && i.na !== true);

  return (
    <section className="grid content-start gap-2" data-testid={`${testPrefix}pod-tasks`} aria-label="Task lists">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Tasks
        </h2>
        <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }} data-testid={`${testPrefix}tasks-progress`}>
          {resolvedForNotes.length}/{items.length} resolved
        </span>
      </div>
      {enforcementActive && blockingCount > 0 && (
        <p className="text-xs" style={{ color: "var(--m-warning)" }}>
          {blockingCount} required item{blockingCount === 1 ? "" : "s"} must be resolved before this ticket can close.
        </p>
      )}
      <ul className="grid gap-1">
        {items.map((item) => {
          const isNa = item.na === true;
          return (
            <li
              key={item.id}
              className="flex items-center gap-1.5 text-xs"
              style={isNa ? { textDecoration: "line-through", color: "var(--m-text-muted)" } : undefined}
              data-testid={`${testPrefix}task-item-${item.id}`}
            >
              <button
                type="button"
                className="rounded-[2px] border px-1 py-0.5 leading-none"
                style={{
                  borderColor: item.checked && !isNa ? "var(--m-status-success-fg)" : "var(--m-border)",
                  color: item.checked && !isNa ? "var(--m-status-success-fg)" : "var(--m-text-muted)",
                }}
                title={item.checked && !isNa ? "Uncheck" : "Check off"}
                aria-label={`${item.checked && !isNa ? "Uncheck" : "Check"} ${item.text}`}
                data-testid={`${testPrefix}task-check-${item.id}`}
                onClick={() => void toggleChecked(item, !(item.checked && !isNa))}
              >
                ✓
              </button>
              <button
                type="button"
                className="rounded-[2px] border px-1 py-0.5 leading-none"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
                title="Uncheck"
                aria-label={`Uncheck ${item.text}`}
                data-testid={`${testPrefix}task-uncheck-${item.id}`}
                onClick={() => void toggleChecked(item, false)}
              >
                ✗
              </button>
              <button
                type="button"
                className="rounded-[2px] border px-1 py-0.5 leading-none"
                style={{
                  borderColor: isNa ? "var(--m-warning)" : "var(--m-border)",
                  color: isNa ? "var(--m-warning)" : "var(--m-text-muted)",
                }}
                title={isNa ? "Clear N-A" : "Mark not applicable"}
                aria-label={`Mark ${item.text} ${isNa ? "applicable again" : "not applicable"}`}
                data-testid={`${testPrefix}task-na-${item.id}`}
                onClick={() => void markNa(item, !isNa)}
              >
                N/A
              </button>
              <span className="min-w-0 flex-1">{item.text}</span>
            </li>
          );
        })}
        {items.length === 0 && (
          <li className="text-xs" style={{ color: "var(--m-text-muted)" }}>No tasks yet.</li>
        )}
      </ul>
      {reasonAsk !== null && (() => {
        const ask = reasonAsk;
        return (
          <div
            className="rounded-lg border px-2 py-2"
            style={{ borderColor: "var(--m-border)", background: "var(--m-bg-subtle)" }}
            data-testid={`${testPrefix}reason-prompt-${ask.itemId}`}
          >
            <label className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
              Reason (required) — e.g. "Customer already resolved via vendor"
            </label>
            <NoteComposer
              value={ask.text}
              onChange={(text) => setReasonAsk({ ...ask, text })}
              onSubmit={(body) => {
                void (async () => {
                  const item = items.find((i) => i.id === ask.itemId);
                  if (item === undefined) return;
                  const result = await onSetState(
                    ask.itemId,
                    ask.kind === "na"
                      ? { checked: false, na: true }
                      : { checked: false, na: false },
                    body,
                  );
                  setReasonAsk(null);
                  if (!result.ok) {
                    setRowError(rejectionText(result, "Could not update that item"));
                  }
                })();
              }}
              compact
              showHints={false}
              ariaLabel="Reason for task state change"
              placeholder="Type the reason, then ⌘↵ to save"
              testPrefix={`${testPrefix}reason`}
            />
            <div className="flex gap-2 pt-1">
              <Button
                size="sm"
                disabled={ask.text.trim() === ""}
                data-testid={`${testPrefix}reason-save-${ask.itemId}`}
                onClick={() => {
                  const item = items.find((i) => i.id === ask.itemId);
                  if (item === undefined) return;
                  void (async () => {
                    const result = await onSetState(
                      ask.itemId,
                      ask.kind === "na"
                        ? { checked: false, na: true }
                        : { checked: false, na: false },
                      ask.text.trim(),
                    );
                    setReasonAsk(null);
                    if (!result.ok) {
                      setRowError(rejectionText(result, "Could not update that item"));
                    }
                  })();
                }}
              >
                Save &amp; mark {ask.kind === "na" ? "N/A" : "incomplete"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setReasonAsk(null)}
              >
                Cancel
              </Button>
            </div>
          </div>
        );
      })()}
      {rowError !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid={`${testPrefix}task-row-error`}>
          {rowError}
        </p>
      )}
      <div className="flex gap-2">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Add task…"
          onKeyDown={(e) => {
            if (e.key === "Enter") void add();
          }}
          aria-label="New task"
          data-testid={`${testPrefix}task-input`}
        />
        <Button size="sm" disabled={busy} onClick={() => void add()} data-testid={`${testPrefix}task-add`}>
          Add
        </Button>
      </div>
      <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        Resolved tasks fold into the final notes at close.
      </p>
    </section>
  );
}
