import * as React from "react"
import { cn } from "../../lib/cn"

/** The dot colour says what kind of number it is. Words always come with it. */
const DOT = { gold: "bg-mc-gold", blush: "bg-mc-blush", red: "bg-destructive", green: "bg-mc-green", muted: "bg-input" } as const

/**
 * An attention card ("Yours today 12", "At risk 2"). gold = yours, blush = soon, red = a real breach (and only then the
 * card gets a red border), green = complete, muted = zero. Render it as a button when it filters something.
 */
export function StatCard({ label, figure, sub, dot = "muted", breach, className, ...props }: { label: string; figure: React.ReactNode; sub?: React.ReactNode; dot?: keyof typeof DOT; breach?: boolean } & Omit<React.ComponentProps<"button">, "children">) {
  return (
    <button
      data-slot="stat-card"
      type="button"
      className={cn("flex min-h-[112px] cursor-pointer flex-col gap-0.5 rounded-card border bg-card px-4 py-3.5 text-left hover:bg-muted", breach ? "border-destructive" : "border-border", className)}
      {...props}
    >
      <span className="flex items-center gap-2 text-[13px] font-semibold text-muted-foreground">
        <span aria-hidden="true" className={cn("size-2 rounded-full", DOT[dot])} />
        {label}
      </span>
      <span className="mt-1 text-[32px] font-semibold leading-[1.15] tabular-nums">{figure}</span>
      {sub ? <span className="text-[13px] leading-[1.4] text-muted-foreground">{sub}</span> : null}
    </button>
  )
}
