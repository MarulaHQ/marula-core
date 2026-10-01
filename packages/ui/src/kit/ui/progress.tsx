import * as React from "react"
import { Progress as ProgressPrimitive } from "radix-ui"
import { cn } from "../../lib/cn"

/** A progress bar must have an accessible name, so the type requires one of these. */
type Named = { "aria-label": string } | { "aria-labelledby": string }

/** A thin progress bar (setup guides). Always say the numbers next to it too: "1 of 4 done". */
export function Progress({ value = 0, className, ...props }: React.ComponentProps<typeof ProgressPrimitive.Root> & Named) {
  return (
    <ProgressPrimitive.Root data-slot="progress" value={value} className={cn("h-2 overflow-hidden rounded-pill bg-muted", className)} {...props}>
      <ProgressPrimitive.Indicator className="block h-full bg-mc-green" style={{ width: `${value ?? 0}%` }} />
    </ProgressPrimitive.Root>
  )
}
