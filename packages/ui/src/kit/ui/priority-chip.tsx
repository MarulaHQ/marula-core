import * as React from "react"
import { cn } from "../../lib/cn"

const LEVELS = {
  P1: "bg-destructive text-destructive-foreground",
  P2: "bg-mc-p2 text-mc-ink",
  P3: "bg-muted text-foreground",
  P4: "bg-transparent text-muted-foreground shadow-[inset_0_0_0_1px_var(--border)]",
} as const
export type Priority = keyof typeof LEVELS

/** P1 to P4. P1 is filled red because a P1 is a real emergency. */
export function PriorityChip({ level, className, ...props }: { level: Priority } & Omit<React.ComponentProps<"span">, "children">) {
  return (
    <span data-slot="priority-chip" className={cn("inline-flex h-[22px] min-w-[30px] items-center justify-center rounded-md px-1.5 text-[12px] font-bold", LEVELS[level], className)} {...props}>
      {level}
    </span>
  )
}
