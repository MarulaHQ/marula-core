import * as React from "react"
import { cn } from "../../lib/cn"

/**
 * Wrap a table in <DataGrid>, use <DataRow> for rows and <DataCell> / <DataHeaderCell> for cells.
 * They carry the grid ARIA roles (grid, row, columnheader, gridcell), which a div-based table needs to be readable by a screen reader.
 * The mockup HTML left these out (axe flags it); do not copy that. Put aria-sort on a <DataHeaderCell>, never on a plain div.
 */
export function DataGrid({ className, ...props }: React.ComponentProps<"div">) {
  return <div role="grid" data-slot="data-grid" className={className} {...props} />
}
export function DataCell({ className, ...props }: React.ComponentProps<"div">) {
  return <div role="gridcell" className={className} {...props} />
}
export function DataHeaderCell({ className, ...props }: React.ComponentProps<"div">) {
  return <div role="columnheader" className={className} {...props} />
}
/** The checkbox column: centred, no padding. */
export function DataCheckCell({ className, ...props }: React.ComponentProps<"div">) {
  return <div role="gridcell" className={cn("flex items-center justify-center px-0!", className)} {...props} />
}

/**
 * Table rows as CSS grid, so every list in the product lines up the same way and follows density.
 * Pass the same `columns` to the header and to every row, e.g. columns="44px 168px minmax(240px,1.3fr) 86px".
 *   selected  gold-tint row with a gold edge bar (bulk selection)
 *   current   the keyboard cursor: a 2px ink ring
 */
export function DataRow({ columns, header, selected, current, className, style, ...props }: React.ComponentProps<"div"> & { columns: string; header?: boolean; selected?: boolean; current?: boolean }) {
  return (
    <div
      role="row"
      data-slot={header ? "data-header" : "data-row"}
      data-selected={selected || undefined}
      data-current={current || undefined}
      style={{ gridTemplateColumns: columns, ...style }}
      className={cn(
        "relative grid items-center border-b border-border text-[length:var(--mc-fs)] last:border-b-0",
        header
          ? "sticky top-0 z-[4] min-h-hdr bg-background text-[12px] font-semibold uppercase tracking-[.06em] text-muted-foreground"
          : "min-h-row hover:bg-muted",
        selected && "bg-mc-act shadow-[inset_3px_0_0_#E8B83A] hover:bg-mc-act",
        current && (selected ? "shadow-[inset_3px_0_0_#E8B83A,inset_0_0_0_2px_var(--primary)]" : "shadow-[inset_0_0_0_2px_var(--primary)]"),
        "[&>*]:min-w-0 [&>*]:overflow-hidden [&>*]:text-ellipsis [&>*]:whitespace-nowrap [&>*]:px-[var(--mc-px)]",
        className
      )}
      {...props}
    />
  )
}
