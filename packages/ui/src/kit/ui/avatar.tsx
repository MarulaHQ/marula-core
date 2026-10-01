import * as React from "react"
import { cn } from "../../lib/cn"

/** Initials in a green circle. Size follows density (26 / 24 / 20); "lg" is 36px. "unassigned" is a dashed empty circle. */
export function Avatar({ initials, size = "default", unassigned, className, ...props }: { initials?: string; size?: "default" | "lg"; unassigned?: boolean } & Omit<React.ComponentProps<"span">, "children">) {
  return (
    <span
      data-slot="avatar"
      className={cn(
        "inline-flex shrink-0 items-center justify-center rounded-full font-semibold",
        size === "lg" ? "size-9 text-[13px]" : "size-av text-[12px]",
        unassigned ? "border-[1.5px] border-dashed border-input bg-transparent text-muted-foreground" : "bg-mc-avatar text-mc-avatar-fg",
        className
      )}
      {...props}
    >
      {unassigned ? null : initials}
    </span>
  )
}
