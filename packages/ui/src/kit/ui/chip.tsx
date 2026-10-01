import * as React from "react"
import { cn } from "../../lib/cn"
import { Icon } from "../icons/Icon"

const base = "inline-flex h-[calc(var(--mc-ctl)-8px)] shrink-0 items-center whitespace-nowrap rounded-pill border text-[13px]"

/** An applied filter: "Status  In progress  (x)". */
export function Chip({ label, value, onRemove, removeLabel, removeIcon, className }: { label?: string; value: React.ReactNode; onRemove?: () => void; removeLabel?: string; removeIcon?: React.ReactNode; className?: string }) {
  return (
    <span data-slot="chip" className={cn(base, "gap-1 border-border bg-card pl-3 pr-1", className)}>
      {label ? <span className="text-muted-foreground">{label}</span> : null}
      <span>{value}</span>
      {onRemove ? (
        <button
          type="button"
          onClick={onRemove}
          aria-label={removeLabel ?? `Remove filter ${label ?? ""}`.trim()}
          className="inline-flex size-[calc(var(--mc-ctl)-14px)] cursor-pointer items-center justify-center rounded-pill border-0 bg-transparent text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          {removeIcon ?? <Icon name="x" size={14} />}
        </button>
      ) : null}
    </span>
  )
}

/** The dashed "Add filter" chip. */
export function ChipAdd({ className, children, type = "button", ...props }: React.ComponentProps<"button">) {
  return (
    <button data-slot="chip-add" type={type} className={cn(base, "cursor-pointer gap-1.5 border-dashed border-input bg-transparent px-3 text-muted-foreground", className)} {...props}>
      {children}
    </button>
  )
}

/** "+3 more". */
export function ChipMore({ className, children, type = "button", ...props }: React.ComponentProps<"button">) {
  return (
    <button data-slot="chip-more" type={type} className={cn(base, "cursor-pointer gap-1 border-transparent bg-muted px-3 font-semibold", className)} {...props}>
      {children}
    </button>
  )
}
