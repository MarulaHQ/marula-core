/**
 * Table primitives — first-class dense tables: sticky header, 24px rows,
 * token borders only. Layout behavior (resize, saved views) lives in apps.
 */
import {
  forwardRef,
  type HTMLAttributes,
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

export type ThProps = ThHTMLAttributes<HTMLTableCellElement>;

export const Th = forwardRef<HTMLTableCellElement, ThProps>(
  ({ className, ...props }, ref) => (
    <th ref={ref} scope="col" className={cn(className)} {...props} />
  ),
);

Th.displayName = "Th";

export type TdProps = TdHTMLAttributes<HTMLTableCellElement>;

export const Td = forwardRef<HTMLTableCellElement, TdProps>(
  ({ className, ...props }, ref) => (
    <td ref={ref} className={cn(className)} {...props} />
  ),
);

Td.displayName = "Td";

export type TrProps = HTMLAttributes<HTMLTableRowElement>;

/** Body row; `selected` renders the accent-soft selection state. */
export function Tr({ selected, className, ...props }: TrProps & { selected?: boolean }) {
  return <tr data-selected={selected ? "true" : undefined} className={cn(className)} {...props} />;
}

Tr.displayName = "Tr";
