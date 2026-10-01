import * as React from "react"
import { cn } from "../../lib/cn"
import { FruitLine } from "./brand"

/**
 * An empty state: a drawing, one heading, one sentence, at most one action.
 * Rule: an empty state for something the person has no AI grant for never mentions or hints at AI.
 */
export function EmptyState({ title, children, action, className }: { title: string; children?: React.ReactNode; action?: React.ReactNode; className?: string }) {
  return (
    <div data-slot="empty-state" className={cn("flex flex-col items-center gap-2 px-6 py-7 text-center", className)}>
      <FruitLine />
      <h2 className="m-0 mt-1.5 text-[18px] font-semibold">{title}</h2>
      {children ? <p className="m-0 max-w-[360px] text-muted-foreground">{children}</p> : null}
      {action ? <div className="mt-2.5 flex gap-2.5">{action}</div> : null}
    </div>
  )
}
