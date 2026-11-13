import * as React from "react"
import { cn } from "../../lib/cn"

/** A quiet information box: "The invoice will be marked Paid. A payment can be reversed." */
export function Callout({ className, icon, children, ...props }: React.ComponentProps<"div"> & { icon?: React.ReactNode }) {
  return (
    <div data-slot="callout" className={cn("flex gap-2.5 rounded-ctl bg-muted px-3.5 py-3 text-[13px] leading-[1.5]", className)} {...props}>
      {icon}
      <span>{children}</span>
    </div>
  )
}

/** An error banner for a request that did not land, with its action on the
 *  right. The status danger pair, not the destructive red: nothing here
 *  destroyed anything (§3.4). */
export function Banner({ className, icon, action, children, ...props }: React.ComponentProps<"div"> & { icon?: React.ReactNode; action?: React.ReactNode }) {
  return (
    <div data-slot="banner" role="alert" className={cn("flex gap-2.5 rounded-ctl bg-mc-st-breach px-3.5 py-3 text-[13px] leading-[1.5] text-mc-st-breach-fg", className)} {...props}>
      {icon}
      <span className="flex-1">{children}</span>
      {action}
    </div>
  )
}
