import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "../../lib/cn"

/**
 * Status pills. Colour is never the only signal: pass `dot` or an `icon` (a 14px node), and always include the word.
 *  new / progress / waiting / scheduled / resolved   ticket statuses
 *  breached    a real SLA breach. The only red pill. Never use it for "overdue" or "soon".
 *  attention   blush: soon, overdue (invoices), validation. The everyday "look at this".
 *  gold        where you act: yours, selected, current.
 */
export const pillVariants = cva("inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-pill text-[13px] font-semibold", {
  variants: {
    tone: {
      new: "bg-mc-st-new text-mc-st-new-fg",
      progress: "bg-mc-progress text-mc-progress-fg",
      waiting: "bg-mc-st-wait text-mc-st-wait-fg",
      scheduled: "border border-mc-st-sched-border bg-mc-st-sched text-mc-st-sched-fg",
      resolved: "bg-mc-st-resolved text-mc-st-resolved-fg",
      breached: "bg-mc-st-breach text-mc-st-breach-fg",
      attention: "bg-mc-attn text-mc-attn-fg",
      gold: "bg-mc-act text-mc-act-fg",
      success: "bg-mc-green-soft text-mc-green-text",
      neutral: "bg-muted text-foreground",
    },
    /** default follows density (24 / 22 / 20); lg is the 26px pill used on the portal and in headers */
    size: { default: "h-pill px-[10px]", lg: "h-[26px] px-[10px]" },
  },
  defaultVariants: { tone: "new", size: "default" },
})

export type PillProps = React.ComponentProps<"span"> & VariantProps<typeof pillVariants> & { dot?: boolean; icon?: React.ReactNode }

export function Pill({ className, tone, size, dot, icon, children, ...props }: PillProps) {
  return (
    <span data-slot="pill" className={cn(pillVariants({ tone, size }), className)} {...props}>
      {dot ? <span aria-hidden="true" className="size-[7px] rounded-full bg-current" /> : icon}
      {children}
    </span>
  )
}
