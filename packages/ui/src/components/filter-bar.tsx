"use client";
/**
 * FilterBar, FilterChips and FilterTrigger — Wave 41 lane 2b, D12: what the user
 * is looking at, said back to them.
 *
 * Purpose: hold the active filter clauses as removable CHIPS, and open a builder
 * for adding one. Two things earn this shape over a single dropdown. A ledger is
 * filtered by several things at once, and a dropdown hides all but the last. A
 * chip is also the only honest representation of a compound filter: `status is
 * open AND assignee is me` is two facts, and collapsing them into one label means
 * nobody can tell what the table is showing or take one of the two off.
 *
 * Three exports, because the template puts the two halves in different places
 * (`DESIGN-NOTES-tickets.md` §2 law 1): the chips read in the middle of the
 * toolbar, and the `Filter` button sits on the right beside Columns and Export.
 * `FilterBar` still renders both together for the surfaces that have not moved
 * yet, so this split changed no existing caller.
 *
 * The chip row never wraps. §6 gives the rule: when the chips stop fitting, the
 * rest collapse into a `+N more` chip, and `+ Add filter` ends the row. A wrapped
 * chip row is what pushed the table down on the surfaces the audit found.
 *
 * Non-purpose: this filters nothing. It reports clauses; the surface applies
 * them. Keeping the two apart is what lets a surface offer these controls on a
 * client-side table and a server-side one without branching.
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

/** The chip's three parts, so the value can carry the semibold the pack gives it
 * (`DESIGN-NOTES-tickets.md` §6: the chip reads as a sentence, value in bold). */
export function clauseParts(
  clause: FilterClause,
  fields: FilterField[],
): { lead: string; value: string | null } {
  const op = OPERATOR_LABEL[clause.op];
  const field = labelFor(fields, clause);
  if (clause.op === "is_set" || clause.op === "is_not_set") {
    return { lead: `${field} ${op}`, value: null };
  }
  const value = (clause.value ?? "")
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
    .join(", ");
  return { lead: `${field} ${op}`, value };
}

export interface FilterChipsProps {
  fields: FilterField[];
  clauses: FilterClause[];
  onChange: (next: FilterClause[]) => void;
  /** Opens the builder. The `+ Add filter` chip calls it, as do the pack's. */
  onAdd?: () => void;
  /** Rendered when nothing is filtered and no `onAdd` chip is wanted. */
  emptyHint?: string;
  /** Hide the dashed `+ Add filter` chip (a surface that puts it elsewhere). */
  hideAdd?: boolean;
  /**
   * How many chips show before the rest collapse into `+N more`. The pack leaves
   * this to the row width; a fixed number is deterministic, never wraps, and
   * `+N more` expands the remainder in place.
   */
  maxChips?: number;
  label?: string;
  className?: string;
}

export function FilterChips({
  fields,
  clauses,
  onChange,
  onAdd,
  emptyHint = "No filters",
  hideAdd = false,
  maxChips = 3,
  label = "Filters",
  className,
}: FilterChipsProps): React.JSX.Element {
  const [expanded, setExpanded] = useState(false);
  const remove = useCallback(
    (id: string) => onChange(clauses.filter((c) => c.id !== id)),
    [clauses, onChange],
  );

  const overflow = !expanded && clauses.length > maxChips;
  const visible = overflow ? clauses.slice(0, maxChips) : clauses;
  const hiddenCount = clauses.length - visible.length;

  if (clauses.length === 0 && hideAdd) {
    return <span className={cn("m-filter-empty", className)}>{emptyHint}</span>;
  }

  return (
    <div className={cn("m-filter-chips-wrap", className)}>
      {clauses.length === 0 ? (
        <span className="m-filter-empty">{emptyHint}</span>
      ) : (
        <ul className="m-filter-chips" aria-label={`${label}, active`}>
          {visible.map((clause) => {
            const { lead, value } = clauseParts(clause, fields);
            return (
              <li key={clause.id} className="m-filter-chip">
                <span className="m-filter-chip-text">
                  {lead}
                  {value === null ? null : (
                    <>
                      {" "}
                      <b className="m-filter-chip-value">{value}</b>
                    </>
                  )}
                </span>
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
            );
          })}
          {hiddenCount > 0 ? (
            <li className="m-filter-chip m-filter-chip-more">
              <button
                type="button"
                className="m-filter-more"
                onClick={() => setExpanded(true)}
                aria-label={`Show ${hiddenCount} more filters`}
              >
                +{hiddenCount} more
              </button>
            </li>
          ) : null}
        </ul>
      )}
      {hideAdd || onAdd === undefined ? null : (
        <button type="button" className="m-filter-add" onClick={onAdd}>
          <span aria-hidden="true">+</span> Add filter
        </button>
      )}
    </div>
  );
}

export interface FilterTriggerProps {
  fields: FilterField[];
  clauses: FilterClause[];
  onChange: (next: FilterClause[]) => void;
  /** Controlled open state. Omit to let the trigger own it. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** The button's own word. The pack prints "Filter". */
  triggerLabel?: string;
  label?: string;
  className?: string;
}

/** The right-hand builder trigger: the pack's `Filter` button and its popover. */
export function FilterTrigger({
  fields,
  clauses,
  onChange,
  open,
  onOpenChange,
  triggerLabel = "Filter",
  label = "Filters",
  className,
}: FilterTriggerProps): React.JSX.Element {
  const [selfOpen, setSelfOpen] = useState(false);
  const isOpen = open ?? selfOpen;
  const setOpen = useCallback(
    (next: boolean) => {
      if (open === undefined) setSelfOpen(next);
      onOpenChange?.(next);
    },
    [open, onOpenChange],
  );

  const root = useRef<HTMLDivElement>(null);
  const [field, setField] = useState(fields[0]?.key ?? "");
  const [op, setOp] = useState<FilterOperator>("eq");
  const [value, setValue] = useState("");

  useEffect(() => {
    if (!isOpen) return;
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
  }, [isOpen, setOpen]);

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
  }, [canAdd, clauses, field, onChange, op, opNeedsValue, setOpen, value]);

  return (
    <div className={cn("m-filter-builder-root", className)} ref={root}>
      <button
        type="button"
        className="m-toolbar-button"
        aria-expanded={isOpen}
        onClick={() => setOpen(!isOpen)}
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
          <path d="M3 5h18l-7 8v6l-4 2v-8L3 5Z" strokeLinejoin="round" />
        </svg>
        {triggerLabel}
        {clauses.length === 0 ? null : (
          <span className="m-toolbar-count" aria-hidden="true">
            {clauses.length}
          </span>
        )}
      </button>
      {isOpen ? (
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
  );
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
  maxChips?: number;
}

/**
 * Chips and trigger together, in one element. Kept for the surfaces that have not
 * moved onto the toolbar's two slots yet — byte-for-byte the shape those callers
 * already render, so the split changed nothing for them.
 */
export function FilterBar({
  fields,
  clauses,
  onChange,
  label = "Filters",
  className,
  emptyHint = "No filters",
  trailing,
  maxChips,
}: FilterBarProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  return (
    <div className={cn("m-filter-bar", className)}>
      <FilterChips
        fields={fields}
        clauses={clauses}
        onChange={onChange}
        onAdd={() => setOpen(true)}
        /* This shape renders the builder trigger on the same row, so the dashed
           `+ Add filter` chip would be a second control for the same popover. The
           split slot (`DataToolbar`'s `chips` + `filter`) is where both belong. */
        hideAdd
        emptyHint={emptyHint}
        label={label}
        maxChips={maxChips}
      />
      <FilterTrigger
        fields={fields}
        clauses={clauses}
        onChange={onChange}
        open={open}
        onOpenChange={setOpen}
        label={label}
      />
      {trailing}
    </div>
  );
}
