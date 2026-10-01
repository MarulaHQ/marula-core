import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "../../lib/cn"

const strip = cva("flex h-11 shrink-0 items-center gap-2.5 px-4 font-medium [&_b]:font-bold", {
  variants: { tone: { attention: "bg-mc-attn text-mc-attn-fg", clean: "bg-mc-green-soft text-mc-green-text" } },
  defaultVariants: { tone: "attention" },
})

/**
 * A full-width strip above a table: "3 of 12 flagged. Each one has a fix in its row."  [Review all]
 * attention is blush; clean is green and says out loud that nothing was found. Put a <Button variant="strip" size="sm"> inside.
 */
export function Strip({ className, tone, ...props }: React.ComponentProps<"div"> & VariantProps<typeof strip>) {
  return <div data-slot="strip" role="status" className={cn(strip({ tone }), className)} {...props} />
}
export function StripSpacer() {
  return <span className="flex-1" />
}
