"use client";
/**
 * ColumnChooser — Wave 41 lane 2b, D12: the visible-column list.
 *
 * Purpose: let a user turn columns off and on without a round trip, in the
 * ORDER the table is actually showing them. That last part is the reason this
 * is a list and not a set of checkboxes: the list doubles as the reorder
 * surface, so a column the user has never seen is in the same list as the
 * ones they hide every day, and moving one is the same gesture as hiding one.
 *
 * The list is the visible order on entry and leaves in that order, so the
 * table never reorders behind a user's back.
 *
 * Non-purpose: this does not know the table's columns. The caller passes them
 * in render order; a column this does not receive cannot be shown, hidden or
 * moved, which is why a column the surface has since dropped simply stops
 * being offered instead of becoming an unremovable ghost.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../lib/cn";

export interface ChoosableColumn {
  key: string;
  label: string;
  visible: boolean;
  /** Drag-to-reorder start width, when the table has one. */
  width?: number;
  /** A column a user cannot turn off (the row's identity, say). */
  required?: boolean;
}

export interface ColumnChooserProps {
  columns: ChoosableColumn[];
  onChange: (next: ChoosableColumn[]) => void;
  label?: string;
  className?: string;
  /** Rendered as the trigger's own label. */
  triggerLabel?: string;
}

export function ColumnChooser({
  columns,
  onChange,
  label = "Columns",
  className,
  triggerLabel = "Columns",
}: ColumnChooserProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  // The list is the table's order. Held locally while the popover is open so
  // dragging does not refetch the table under the pointer, and committed on
  // close so an abandoned chooser leaves no trace.
  const [draft, setDraft] = useState<ChoosableColumn[]>(columns);
  const dragFrom = useRef<number | null>(null);
  const [dragOver, setDragOver] = useState<number | null>(null);

  useEffect(() => {
    if (open) setDraft(columns);
  }, [open, columns]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") setOpen(false);
    };
    const onPointer = (e: MouseEvent): void => {
      if (root.current && !root.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onPointer);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onPointer);
    };
  }, [open]);

  const toggle = useCallback(
    (key: string) => {
      setDraft((rows) =>
        rows.map((c) => (c.key === key && c.required ? c : { ...c, visible: !c.visible })),
      );
    },
    [],
  );

  const move = useCallback((from: number, to: number) => {
    setDraft((rows) => {
      if (from === to || from < 0 || to < 0 || from >= rows.length || to >= rows.length) return rows;
      const next = rows.slice();
      const [moved] = next.splice(from, 1);
      if (moved === undefined) return rows;
      next.splice(to, 0, moved);
      return next;
    });
  }, []);

  const visibleCount = columns.filter((c) => c.visible).length;

  return (
    <div className={cn("m-column-chooser", className)} ref={root}>
      <button
        type="button"
        className="m-toolbar-button"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((v) => !v)}
      >
        {triggerLabel}
        <span className="m-toolbar-count" aria-hidden="true">
          {visibleCount}
        </span>
      </button>
      {open ? (
        <div className="m-column-chooser-panel" role="dialog" aria-label={label}>
          <p className="m-panel-title">{label}</p>
          <ul className="m-column-list">
            {draft.map((column, index) => (
              <li
                key={column.key}
                className={cn(
                  "m-column-row",
                  dragOver === index && "is-drop-target",
                  dragFrom.current === index && "is-dragging",
                )}
                draggable
                onDragStart={(e) => {
                  dragFrom.current = index;
                  e.dataTransfer.effectAllowed = "move";
                }}
                onDragOver={(e) => {
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                  setDragOver(index);
                }}
                onDrop={(e) => {
                  e.preventDefault();
                  if (dragFrom.current !== null) move(dragFrom.current, index);
                  dragFrom.current = null;
                  setDragOver(null);
                }}
                onDragEnd={() => {
                  dragFrom.current = null;
                  setDragOver(null);
                }}
              >
                <span className="m-column-grip" aria-hidden="true">
                  ⠿
                </span>
                <label className="m-column-label">
                  <input
                    type="checkbox"
                    checked={column.visible}
                    disabled={column.required}
                    onChange={() => toggle(column.key)}
                  />
                  <span>{column.label}</span>
                </label>
              </li>
            ))}
          </ul>
          <div className="m-panel-actions">
            <button
              type="button"
              className="m-toolbar-button"
              onClick={() => {
                setDraft(columns);
                setOpen(false);
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              className="m-btn m-btn-primary m-btn-sm"
              onClick={() => {
                onChange(draft);
                setOpen(false);
              }}
            >
              Apply
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
