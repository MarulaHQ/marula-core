import * as React from "react"
import { cn } from "../../lib/cn"
import { MarulaMark } from "./brand"
import { Kbd } from "./kbd"

/**
 * The top bar, 56px, on every staff surface: the logo (the fruit mark and the wordmark), search with the keyboard hint,
 * a spacer, then Online presence, the theme toggle, the notification bell and the avatar.
 * The bell lives here and never on a right edge, so it never competes with the assistant column.
 */
export function TopBar({ className, ...props }: React.ComponentProps<"header">) {
  return <header data-slot="top-bar" className={cn("flex h-14 items-center gap-5 border-b border-border bg-card px-4", className)} {...props} />
}

/** The mark and the word. It must always carry the mark: a top bar with only the word is a mistake. */
export function TopBarLogo({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div data-slot="top-bar-logo" className={cn("flex w-[200px] items-center gap-2 font-display text-[24px] tracking-[-0.03em]", className)} {...props}>
      <MarulaMark size={30} />
      marula
    </div>
  )
}

/**
 * The search field's shell. Exported because a surface that opens a search page
 * instead of filtering in place still draws the same field: the launcher is a
 * button wearing this, so the two can never drift into a lookalike.
 */
export const topBarSearchClasses =
  "flex h-10 min-w-0 flex-[0_1_520px] items-center gap-2.5 rounded-[10px] border border-border bg-background px-3 text-muted-foreground";

export function TopBarSearch({ icon, shortcut = "⌘K", placeholder = "Search tickets, clients, devices…", className, ...props }: { icon?: React.ReactNode; shortcut?: string } & Omit<React.ComponentProps<"input">, "type">) {
  return (
    <label data-slot="top-bar-search" className={cn(topBarSearchClasses, className)}>
      {icon}
      <span className="sr-only">Search</span>
      <input type="search" placeholder={placeholder} className="min-w-0 flex-1 border-0 bg-transparent px-0.5 py-px text-foreground outline-none placeholder:text-muted-foreground" {...props} />
      <Kbd>{shortcut}</Kbd>
    </label>
  )
}

export function TopBarSpacer() {
  return <div className="flex-1" />
}

/** "Online" with a green dot. The word is always there; the dot is not the only signal. */
export function Presence({ children = "Online" }: { children?: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2">
      <span aria-hidden="true" className="size-2 rounded-full bg-mc-green" />
      {children}
    </div>
  )
}
