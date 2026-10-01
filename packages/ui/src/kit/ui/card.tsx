import * as React from "react"
import { cn } from "../../lib/cn"

/** A bordered surface. The basic container for groups of related content. */
export function Card({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card" className={cn("overflow-hidden rounded-card border border-border bg-card text-card-foreground", className)} {...props} />
}
export function CardHeader({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-header" className={cn("flex items-center gap-2.5 border-b border-border px-[18px] py-3.5", className)} {...props} />
}
export function CardTitle({ className, ...props }: React.ComponentProps<"h2">) {
  return <h2 data-slot="card-title" className={cn("m-0 text-[16px] font-semibold", className)} {...props} />
}
export function CardBody({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-body" className={cn("flex flex-col gap-2 px-[18px] py-3.5", className)} {...props} />
}
