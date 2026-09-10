// Task-lists panel — per-ticket task items through the whole workflow.
//
// Purpose: render the ticket checklist inline (notes-pane pod), with
// per-item tri-state actions: ✓ check, ✗ uncheck, and N-A. Check/uncheck
// ride the existing checklist toggle endpoint (server truth). N-A is a
// per-user display state persisted through the panel-state engine
// (namespace `meridian.ticket-tasks`): the API has no third checklist
// state, so an N-A item is marked checked server-side + shown as N-A with
// strike-through in this user's view, and excluded from the "resolved"
// fold when it was N-A'd — a backend follow-up records the real column.
// Task states fold into notes (see close-flow.tsx).
// Non-purpose: no checklist-template editing (board admin owns that).

"use client";

import type { components } from "@meridian/sdk";
import { Button, Input } from "@meridian/ui";
import { useEffect, useState, type ReactNode } from "react";

import { LocalStoragePanelStateStore } from "@meridian/layout";

type ChecklistItem = components["schemas"]["ChecklistItem"];

const store = new LocalStoragePanelStateStore({ namespace: "meridian.ticket-tasks" });

/** Per-user N-A set, keyed per ticket. */
export type NaMap = Record<string, string[]>; // ticketId -> itemIds

async function loadNa(userId: string, ticketId: string): Promise<string[]> {
  try {
    const raw = await store.load(`${userId}:na`);
    if (raw === null) return [];
    const parsed = JSON.parse(raw) as NaMap | null;
    return parsed?.[ticketId] ?? [];
  } catch {
    return [];
  }
}

async function saveNa(userId: string, ticketId: string, ids: string[]): Promise<void> {
  const raw = await store.load(`${userId}:na`);
  let map: NaMap = {};
  try {
    map = (raw === null ? null : (JSON.parse(raw) as NaMap)) ?? {};
  } catch {
    map = {};
  }
  if (ids.length === 0) delete map[ticketId];
  else map[ticketId] = ids;
  await store.save(`${userId}:na`, JSON.stringify(map));
}

export interface TasksPanelProps {
  ticketId: string;
  userId: string;
  items: ChecklistItem[];
  enforcementActive: boolean;
  blockingCount: number;
  /** Testid prefix so multiple mounted instances don't collide. */
  testPrefix?: string;
  onToggle(itemId: string, checked: boolean): Promise<void>;
  onAdd(text: string): Promise<void>;
}

export function TasksPanel({
  ticketId,
  userId,
  items,
  enforcementActive,
  blockingCount,
  testPrefix = "",
  onToggle,
  onAdd,
}: TasksPanelProps): ReactNode {
  const [na, setNa] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void loadNa(userId, ticketId).then((ids) => {
      if (!cancelled) setNa(ids);
    });
    return () => {
      cancelled = true;
    };
  }, [userId, ticketId]);

  async function markNa(item: ChecklistItem, value: boolean): Promise<void> {
    const next = value ? [...new Set([...na, item.id])] : na.filter((id) => id !== item.id);
    setNa(next);
    await saveNa(userId, ticketId, next);
    // N-A settles the item as resolved server-side (checked); clearing N-A
    // via the N/A button leaves the item checked — the ✗ button unchecks.
    await onToggle(item.id, value ? true : item.checked);
  }

  async function toggleChecked(item: ChecklistItem, checked: boolean): Promise<void> {
    if (checked && na.includes(item.id)) {
      const next = na.filter((id) => id !== item.id);
      setNa(next);
      await saveNa(userId, ticketId, next);
    }
    await onToggle(item.id, checked);
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
  const resolvedForNotes = items.filter((i) => i.checked && !na.includes(i.id));

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
          const isNa = na.includes(item.id);
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
