import * as React from "react"
import { cn } from "../../lib/cn"

/**
 * A native checkbox, tinted with accent-color. The mockups use the native control on purpose:
 * it is accessible for free and matches the pixels. Do not replace it with a custom one.
 */
export function Checkbox({ className, ...props }: Omit<React.ComponentProps<"input">, "type">) {
  return <input data-slot="checkbox" type="checkbox" className={cn("m-0 size-[18px] cursor-pointer accent-primary", className)} {...props} />
}
