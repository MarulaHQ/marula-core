"use client";
/**
 * Table primitives — first-class dense tables: sticky header, token-driven
 * rows, and two header behaviours the data surfaces need and the primitive
 * used to leave to them.
 *
 * The reorder and the resize are HERE, not in each surface, for the reason
 * the density ladder needs them at all: a column order a user sets has to
 * survive the page it was set on, and a primitive that only knows how to
 * render `children` cannot know what order those children are in. A surface
 * that wants the behaviour without this component builds its own table, and
 * the fleet, stock and billing ledgers are exactly the surfaces that cannot
 * afford to.
 *
 * Both behaviours are pointer-only by design, and both have a keyboard route
 * because a drag is not an operation a keyboard user can perform at all:
 *
 * - Reorder: focus a header, then Alt+Arrow to move it a column either way.
 * - Resize: focus the divider, then Arrow (or Shift+Arrow) to grow or shrink.
 *
 * Non-purpose: no sorting, no selection, no fetching. A header cell is still a
 * `Th`, a row is still a `Tr`, and a surface that sorts renders its own sort
 * affordance inside the cell it owns.
 */
import {
  forwardRef,
  useCallback,
  useRef,
  useState,
  type HTMLAttributes,
  type KeyboardEvent,
  type PointerEvent,
  type TdHTMLAttributes,
  type ThHTMLAttributes,
  type TableHTMLAttributes,
} from "react";
import { cn } from "../lib/cn";

export const Table = forwardRef<HTMLTableElement, TableHTMLAttributes<HTMLTableElement>>(
  ({ className, ...props }, ref) => (
    <table ref={ref} className={cn("m-table", className)} {...props} />
  ),
);

Table.displayName = "Table";

export type ThProps = ThHTMLAttributes<HTMLTableCellElement> & {
  /** Stable column key. The reorder maths and the resize maths both key off
   * the header's own identity rather than its position in the DOM. */
  columnKey?: string;
  /** Current width in pixels, when the table is resizable. */
  width?: number;
  /** Where this column sits, for the keyboard reorder and for the a11y
   * announcement. Supplied by `SortableHeaderRow`. */
  index?: number;
  columnCount?: number;
  onReorder?: (from: number, to: number) => void;
  onResize?: (columnKey: string, width: number) => void;
  /** Hide the drag affordance without unmounting the behaviour (a table that
   * is not reordering, e.g. a print view). */
  sortable?: boolean;
  resizable?: boolean;
  /** Let the column shrink below its stored width. A table that lays its
   * columns out as fractions of its own width passes this so a stored width
   * is a proportion, not a floor that pushes the table past the viewport. */
  fluid?: boolean;
};

const MIN_COLUMN_WIDTH = 64;
const MAX_COLUMN_WIDTH = 640;
const RESIZE_STEP = 16;
const RESIZE_STEP_LARGE = 64;

/** A header cell that can be dragged to reorder, and whose right-hand divider
 * can be dragged to resize. */
export const Th = forwardRef<HTMLTableCellElement, ThProps>(
  (
    {
      className,
      columnKey,
      width,
      index,
      columnCount,
      onReorder,
      onResize,
      sortable = true,
      resizable = true,
      fluid = false,
      style,
      onKeyDown,
      ...props
    },
    ref,
  ) => {
    const [dragging, setDragging] = useState(false);
    const [dropSide, setDropSide] = useState<"before" | "after" | null>(null);
    const dragFrom = useRef<number | null>(null);
    const resizing = useRef(false);
    const widthAtStart = useRef<number | null>(null);

    const move = useCallback(
      (from: number, to: number) => {
        if (!onReorder || from === to) return;
        onReorder(from, to);
      },
      [onReorder],
    );

    const onHeaderKeyDown = (e: KeyboardEvent<HTMLTableCellElement>): void => {
      // Alt+Arrow reorders. A bare Arrow is left to the surface: on a sortable
      // table Arrow sorts, and a primitive that swallowed it would break
      // every surface that wants sorting.
      if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
        if (index === undefined || !onReorder) return;
        e.preventDefault();
        const step = e.key === "ArrowLeft" ? -1 : 1;
        move(index, index + step);
        return;
      }
      if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        if (!onResize || width === undefined) return;
        e.preventDefault();
        e.stopPropagation();
        const step = (e.shiftKey ? RESIZE_STEP_LARGE : RESIZE_STEP) * (e.key === "ArrowRight" ? 1 : -1);
        onResize(columnKey ?? String(index), clampWidth(width + step));
        return;
      }
      onKeyDown?.(e);
    };

    const onGripPointerDown = (e: PointerEvent<HTMLSpanElement>): void => {
      if (index === undefined) return;
      dragFrom.current = index;
      e.currentTarget.setPointerCapture(e.pointerId);
    };

    const onThPointerMove = (e: PointerEvent<HTMLTableCellElement>): void => {
      if (dragFrom.current === null || index === undefined) return;
      // Half the cell is "before", half is "after"; the indicator is drawn on
      // the edge the column would land against.
      const box = e.currentTarget.getBoundingClientRect();
      setDropSide(e.clientX - box.left < box.width / 2 ? "before" : "after");
    };

    const commitDrop = (): void => {
      if (dragFrom.current !== null && dropSide && index !== undefined) {
        move(dragFrom.current, dropSide === "before" ? index : index + 1);
      }
      dragFrom.current = null;
      setDragging(false);
      setDropSide(null);
    };

    const onDividerPointerDown = (e: PointerEvent<HTMLSpanElement>): void => {
      if (!onResize || !columnKey) return;
      resizing.current = true;
      widthAtStart.current = width ?? null;
      e.currentTarget.setPointerCapture(e.pointerId);
      e.stopPropagation();
    };

    const onDividerPointerMove = (e: PointerEvent<HTMLSpanElement>): void => {
      if (!resizing.current || !onResize || !columnKey) return;
      // Resize is measured against the pointer, not the cell, so the column
      // tracks the cursor instead of lagging a frame behind it.
      const cell = (e.currentTarget.parentElement ?? null) as HTMLElement | null;
      if (!cell) return;
      const next = e.clientX - cell.getBoundingClientRect().left;
      onResize(columnKey, clampWidth(next));
    };

    const onDividerPointerUp = (): void => {
      resizing.current = false;
      widthAtStart.current = null;
    };

    const reorderable = Boolean(sortable && onReorder && index !== undefined && columnCount !== undefined && columnCount > 1);
    const resizableHere = Boolean(resizable && onResize && width !== undefined);

    return (
      <th
        ref={ref}
        scope="col"
        aria-colindex={index === undefined ? undefined : index + 1}
        className={cn(
          reorderable && "is-reorderable",
          resizableHere && "is-resizable",
          dragging && "is-drag-source",
          dropSide === "before" && "is-drop-before",
          dropSide === "after" && "is-drop-after",
          className,
        )}
        /* A fluid table gets its widths from the colgroup's fractions. An
           inline pixel width here would win the fixed-layout pass and put the
           table back at the sum of its stored widths, which is the overhang
           this prop exists to remove. */
        style={
          width === undefined || fluid
            ? style
            : { ...style, width: `${width}px`, minWidth: `${width}px` }
        }
        tabIndex={reorderable || resizableHere ? 0 : undefined}
        onKeyDown={onHeaderKeyDown}
        onPointerMove={onThPointerMove}
        onPointerUp={commitDrop}
        onPointerLeave={() => setDropSide(null)}
        {...props}
      >
        {reorderable ? (
          <span
            className="m-th-grip"
            aria-hidden="true"
            onPointerDown={onGripPointerDown}
            onPointerUp={() => setDragging(false)}
          >
            ⠿
          </span>
        ) : null}
        {props.children}
        {resizableHere ? (
          <span
            className="m-th-resizer"
            role="separator"
            aria-orientation="vertical"
            aria-label={typeof props.children === "string" ? `Resize ${props.children}` : "Resize column"}
            /* A FOCUSABLE separator is a widget, not a decorative divider, so it
               owes the value attributes. axe-core reports the omission as a
               critical `aria-required-attr` violation, which is how the ticket
               ledger's first axe run failed. The bounds are read from the same
               clampWidth the keydown handler uses, so the announced value can
               never disagree with what the arrow keys actually do. */
            aria-valuenow={clampWidth(width ?? MIN_COLUMN_WIDTH)}
            aria-valuemin={MIN_COLUMN_WIDTH}
            aria-valuemax={MAX_COLUMN_WIDTH}
            tabIndex={0}
            onPointerDown={onDividerPointerDown}
            onPointerMove={onDividerPointerMove}
            onPointerUp={onDividerPointerUp}
            onKeyDown={(e) => {
              if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
              e.preventDefault();
              e.stopPropagation();
              const step = (e.shiftKey ? RESIZE_STEP_LARGE : RESIZE_STEP) * (e.key === "ArrowRight" ? 1 : -1);
              onResize?.(columnKey as string, clampWidth((width ?? 0) + step));
            }}
          />
        ) : null}
      </th>
    );
  },
);

Th.displayName = "Th";

function clampWidth(width: number): number {
  return Math.min(MAX_COLUMN_WIDTH, Math.max(MIN_COLUMN_WIDTH, Math.round(width)));
}

export type TdProps = TdHTMLAttributes<HTMLTableCellElement>;

export const Td = forwardRef<HTMLTableCellElement, TdProps>(
  ({ className, ...props }, ref) => (
    <td ref={ref} className={cn(className)} {...props} />
  ),
);

Td.displayName = "Td";

export type TrProps = HTMLAttributes<HTMLTableRowElement>;

/**
 * Body row; `selected` renders the accent fill and gold edge bar, `current` the
 * keyboard-cursor ring. Both land on the `tr` (the pack's `.tr.sel` /
 * `.tr.cur`), and `packages/ui/src/components.css` paints each once per row.
 */
export function Tr({ selected, current, className, ...props }: TrProps & { selected?: boolean; current?: boolean }) {
  return (
    <tr
      data-selected={selected ? "true" : undefined}
      data-current={current ? "true" : undefined}
      className={cn(className)}
      {...props}
    />
  );
}

Tr.displayName = "Tr";
