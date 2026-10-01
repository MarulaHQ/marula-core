import * as React from "react"
import { Tabs as TabsPrimitive } from "radix-ui"
import { cn } from "../../lib/cn"

/** Underline tabs for record pages (Overview, Contacts, Sites...). Counts go in <TabCount>. */
export const Tabs = TabsPrimitive.Root
export const TabsContent = TabsPrimitive.Content

export function TabsList({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.List>) {
  return <TabsPrimitive.List data-slot="tabs-list" className={cn("flex border-b border-border px-6 pt-1.5", className)} {...props} />
}
export function TabsTrigger({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.Trigger>) {
  return (
    <TabsPrimitive.Trigger
      data-slot="tabs-trigger"
      className={cn(
        "mr-[22px] cursor-pointer border-0 border-b-2 border-transparent bg-transparent px-0.5 py-3 text-[14px] font-medium text-muted-foreground",
        "data-[state=active]:border-primary data-[state=active]:font-semibold data-[state=active]:text-foreground",
        className
      )}
      {...props}
    />
  )
}
export function TabCount({ className, ...props }: React.ComponentProps<"span">) {
  return <span className={cn("font-medium text-muted-foreground", className)} {...props} />
}
