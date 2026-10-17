import * as React from "react"
import { ToggleGroup } from "radix-ui"
import { cn } from "../../lib/cn"

/**
 * Segmented control (views, density, theme). One choice, always one selected.
 * size "default" is the views height (ctl - 6px); "sm" is 30px (footer density); "viewer" is 32px.
 */
export function Segmented({ className, ...props }: React.ComponentProps<typeof ToggleGroup.Root>) {
  return <ToggleGroup.Root data-slot="segmented" className={cn("flex w-fit flex-wrap gap-1 rounded-ctl bg-muted p-[3px]", className)} {...props} />
}

const sizes = { default: "h-[calc(var(--mc-ctl)-6px)]", sm: "h-[30px]", viewer: "h-8" } as const

export function SegmentedItem({ className, size = "default", ...props }: React.ComponentProps<typeof ToggleGroup.Item> & { size?: keyof typeof sizes }) {
  return (
    <ToggleGroup.Item
      data-slot="segmented-item"
      className={cn(
        "inline-flex cursor-pointer items-center whitespace-nowrap rounded-[9px] border-0 bg-transparent px-[14px] text-[14px] font-medium text-muted-foreground",
        "data-[state=on]:bg-card data-[state=on]:font-semibold data-[state=on]:text-foreground data-[state=on]:shadow-[0_1px_2px_rgba(0,0,0,.08)]",
        sizes[size],
        className
      )}
      {...props}
    />
  )
}

/** The small count after a view name: "Today 12". */
export function SegmentedCount({ className, ...props }: React.ComponentProps<"span">) {
  return <span className={cn("ml-1.5 font-medium tabular-nums text-muted-foreground", className)} {...props} />
}
