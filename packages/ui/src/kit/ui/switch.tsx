import * as React from "react"
import { Switch as SwitchPrimitive } from "radix-ui"
import { cn } from "../../lib/cn"

/**
 * The toggle. Two rules from the design: a toggle inside a table STAGES the change (a save bar appears),
 * and a toggle on a page of its own APPLIES immediately and says so. The component does not decide; the screen does.
 */
export function Switch({ className, ...props }: React.ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        "group relative h-[26px] w-[44px] shrink-0 cursor-pointer rounded-pill border border-transparent bg-primary p-0",
        "data-[state=unchecked]:border-input data-[state=unchecked]:bg-muted",
        "disabled:cursor-not-allowed disabled:border-border disabled:bg-muted disabled:data-[state=unchecked]:border-border disabled:data-[state=unchecked]:bg-muted",
        className
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        className={cn(
          "pointer-events-none absolute top-[2px] block size-5 rounded-full",
          "data-[state=checked]:right-[3px] data-[state=checked]:bg-primary-foreground",
          "data-[state=unchecked]:left-[3px] data-[state=unchecked]:bg-input",
          "group-disabled:data-[state=checked]:bg-muted-foreground group-disabled:data-[state=unchecked]:bg-border"
        )}
      />
    </SwitchPrimitive.Root>
  )
}
