import * as React from "react"
import { cn } from "../../lib/cn"

/** A keyboard hint: <Kbd>J</Kbd>. Inside a button, pass tone="inherit" so it takes the button colour. */
export function Kbd({ className, tone = "muted", ...props }: React.ComponentProps<"span"> & { tone?: "muted" | "inherit" }) {
  return (
    <span
      data-slot="kbd"
      className={cn(
        "rounded-[5px] border px-[5px] text-[12px] font-semibold leading-[18px]",
        tone === "muted" ? "border-border text-muted-foreground" : "border-current text-inherit opacity-80",
        className
      )}
      {...props}
    />
  )
}
