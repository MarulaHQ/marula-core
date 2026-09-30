"use client";
/**
 * FilterBar — Wave 41 lane 2b, D12: what the user is looking at, said back
 * to them.
 *
 * Purpose: hold the active filter clauses as removable CHIPS, and open a
 * builder for adding one. Two things earn this shape over a single dropdown.
 * A ledger is filtered by several things at once, and a dropdown hides all
 * but the last. A chip is also the only honest representation of a compound
 * filter: `status is open AND assignee is me` is two facts, and collapsing
 * them into one label means nobody can tell what the table is showing or
 * take one of the two off.
 *
 * Non-purpose: this does not filter anything. It reports clauses; the surface
 * applies them. Keeping the two apart is what lets a surface offer these
 * controls on a client-side table and a server-side one without branching.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../lib/cn";

/** The operators the D12 document accepts. Kept in step with the Rust
 * `FilterOp` in `crates/core-domain/src/data_surface.rs`. */
export type FilterOperator =
  | "eq"
  | "ne"
  | "contains"
  | "gt"
  | "gte"
  | "lt"
  | "lte"
  | "in"
  | "is_set"
  | "is_not_set";

/** How a chip reads. `is_set` / `is_not_set` are presence tests and take no
 * value, which is why they read as a phrase rather than a comparison. */
export const OPERATOR_LABEL: Record<FilterOperator, string> = {
  eq: "is",
  ne: "is not",
  contains: "contains",
  gt: ">",
  gte: "at least",
  lt: "<",
  lte: "at most",
  in: "is any of",
  is_set: "is set",
  is_not_set: "is not set",
};

export interface FilterField {
  key: string;
  label: string;
  /** Operators this field can be filtered by. Defaults to all of them. */
  operators?: FilterOperator[];
}

export interface FilterClause {
  id: string;
  field: string;
  op: FilterOperator;
  /** Absent for the presence operators. */
  value?: string;
}

function labelFor(fields: FilterField[], clause: FilterClause): string {
  const field = fields.find((f) => f.key === clause.field);
  return field?.label ?? clause.field;
}

/** `Status is open`, with a list rendered as a readable list. */
export function clauseLabel(clause: FilterClause, fields: FilterField[]): string {
  const op = OPERATOR_LABEL[clause.op];
  if (clause.op === "is_set" || clause.op === "is_not_set") return `${labelFor(fields, clause)} ${op}`;
  const value = (clause.value ?? "")
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
    .join(", ");
  return `${labelFor(fields, clause)} ${op} ${value}`.trim();
}

export interface FilterBarProps {
  fields: FilterField[];
  clauses: FilterClause[];
  onChange: (next: FilterClause[]) => void;
  label?: string;
  className?: string;
  /** Rendered when nothing is filtered. */
  emptyHint?: string;
  /** Extra control after the builder trigger (a saved-filter picker). */
  trailing?: React.ReactNode;
}

export function FilterBar({
  fields,
  clauses,
  onChange,
  label = "Filters",
  className,
  emptyHint = "No filters",
  trailing,
}: FilterBarProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const [field, setField] = useState(fields[0]?.key ?? "");
  const [op, setOp] = useState<FilterOperator>("eq");
  const [value, setValue] = useState("");

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

  const active = fields.find((f) => f.key === field);
  const opNeedsValue = op !== "is_set" && op !== "is_not_set";
  // A value that is present, or an operator that needs none. Anything less is
  // a clause that can never match, and adding one would look like a bug.
  const canAdd = Boolean(field) && (!opNeedsValue || value.trim() !== "");

  const add = useCallback(() => {
    if (!canAdd) return;
    onChange([
      ...clauses,
      {
        id: `${field}:${op}:${value.trim()}`,
        field,
        op,
        value: opNeedsValue ? value.trim() : undefined,
      },
    ]);
    setValue("");
    setOpen(false);
  }, [canAdd, clauses, field, onChange, op, opNeedsValue, value]);

  const remove = useCallback(
    (id: string) => onChange(clauses.filter((c) => c.id !== id)),
    [clauses, onChange],
  );

  return (
    <div className={cn("m-filter-bar", className)} ref={root}>
      {clauses.length === 0 ? (
        <span className="m-filter-empty">{emptyHint}</span>
      ) : (
        <ul className="m-filter-chips" aria-label={`${label}, active`}>
          {clauses.map((clause) => (
            <li key={clause.id} className="m-filter-chip">
              <span className="m-filter-chip-text">{clauseLabel(clause, fields)}</span>
              <button
                type="button"
                className="m-filter-chip-remove"
                aria-label={`Remove filter: ${clauseLabel(clause, fields)}`}
                onClick={() => remove(clause.id)}
              >
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
                  <path d="M5 5l14 14M19 5 5 19" strokeLinecap="round" />
                </svg>
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="m-filter-builder-root">
        <button
          type="button"
          className="m-toolbar-button"
          aria-expanded={open}
          onClick={() => setOpen((v) => !v)}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
            <path d="M3 5h18l-7 8v6l-4 2v-8L3 5Z" strokeLinejoin="round" />
          </svg>
          Add filter
        </button>
        {open ? (
          <div className="m-filter-builder" role="dialog" aria-label={`Add ${label.toLowerCase()}`}>
            <label className="m-builder-field">
              <span className="m-builder-label">Field</span>
              <select
                className="m-input"
                value={field}
                onChange={(e) => {
                  setField(e.target.value);
                  const next = fields.find((f) => f.key === e.target.value);
                  const allowed = next?.operators ?? [];
                  const first = allowed[0];
                  if (first !== undefined && !allowed.includes(op)) setOp(first);
                }}
              >
                {fields.map((f) => (
                  <option key={f.key} value={f.key}>
                    {f.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="m-builder-field">
              <span className="m-builder-label">Is</span>
              <select
                className="m-input"
                value={op}
                onChange={(e) => setOp(e.target.value as FilterOperator)}
              >
                {(active?.operators ?? (Object.keys(OPERATOR_LABEL) as FilterOperator[])).map((o) => (
                  <option key={o} value={o}>
                    {OPERATOR_LABEL[o]}
                  </option>
                ))}
              </select>
            </label>
            {opNeedsValue ? (
              <label className="m-builder-field">
                <span className="m-builder-label">
                  {op === "in" ? "Values, comma separated" : "Value"}
                </span>
                <input
                  className="m-input"
                  value={value}
                  autoFocus
                  onChange={(e) => setValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") add();
                  }}
                />
              </label>
            ) : null}
            <div className="m-panel-actions">
              <button type="button" className="m-toolbar-button" onClick={() => setOpen(false)}>
                Cancel
              </button>
              <button
                type="button"
                className="m-btn m-btn-primary m-btn-sm"
                disabled={!canAdd}
                onClick={add}
              >
                Add
              </button>
            </div>
          </div>
        ) : null}
      </div>
      {trailing}
    </div>
  );
}
