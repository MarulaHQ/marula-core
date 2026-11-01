import * as React from "react"
import { cn } from "../../lib/cn"
import { Icon } from "../icons/Icon"

/** Rail widths. The shell owns the grid column; set it from `expanded`: grid-template-columns: 64px | 188px, then the content. */
export const RAIL_WIDTH = { collapsed: 64, expanded: 188 } as const

const RailContext = React.createContext<{ expanded: boolean; toggle: () => void }>({ expanded: false, toggle: () => {} })
const useRail = () => React.useContext(RailContext)

/**
 * The main navigation rail. Collapsed it is 64px of icon tiles; expanded it is 188px with labels.
 * Controlled: you own `expanded` and persist it per person (the layout package already persists per-user panel state).
 * Order, top to bottom: the destinations, a spacer, Settings, and last the <RailToggle />.
 */
export function AppRail({ expanded = false, onExpandedChange, className, children, ...props }: { expanded?: boolean; onExpandedChange?: (expanded: boolean) => void; className?: string; children: React.ReactNode } & Omit<React.ComponentProps<"nav">, "children">) {
  const value = React.useMemo(() => ({ expanded, toggle: () => onExpandedChange?.(!expanded) }), [expanded, onExpandedChange])
  return (
    <RailContext.Provider value={value}>
      <nav aria-label="Main" data-slot="app-rail" data-expanded={expanded} className={cn("flex h-full flex-col items-center gap-1.5 border-r border-border bg-mc-side py-3", expanded && "items-stretch px-2.5", className)} {...props}>
        {children}
      </nav>
    </RailContext.Provider>
  )
}

const tile = "relative flex h-11 shrink-0 items-center rounded-ctl text-muted-foreground no-underline hover:bg-muted"
const tileSize = (expanded: boolean) => (expanded ? "w-auto justify-start gap-3 px-3 font-medium" : "w-11 justify-center")

/** A destination. Collapsed it needs its name for screen readers, so `label` is always required. Only Tickets carries a count.
 *  With `asChild`, the child (a router link, e.g. next/link) is cloned and carries the rail content, so client-side
 *  navigation keeps working; the child's own className is merged. */
export function RailItem({ icon, label, badge, current, href = "#", className, asChild, children, ...props }: { icon: React.ReactNode; label: string; badge?: React.ReactNode; current?: boolean; asChild?: boolean } & Omit<React.ComponentProps<"a">, "children"> & { children?: React.ReactNode }) {
  const { expanded } = useRail()
  const classes = cn(tile, tileSize(expanded), "aria-[current=page]:bg-mc-act aria-[current=page]:text-mc-act-fg aria-[current=page]:hover:bg-mc-act", expanded && "aria-[current=page]:font-semibold", className)
  const content = (
    <>
      {icon}
      {expanded ? <span className="flex-1 text-left">{label}</span> : null}
      {badge != null ? (
        <span className={cn("flex h-[18px] min-w-[18px] items-center justify-center rounded-pill bg-primary px-1 text-[12px] font-bold text-primary-foreground", expanded ? "ml-auto" : "absolute right-0 top-0.5")}>{badge}</span>
      ) : null}
      {children}
    </>
  )
  if (asChild && React.isValidElement(children)) {
    return React.cloneElement(children as React.ReactElement<Record<string, unknown>>, {
      ...props,
      href,
      "data-slot": "rail-item",
      "aria-current": current ? "page" : undefined,
      "aria-label": expanded ? undefined : label,
      className: cn(classes, (children.props as { className?: string }).className),
    })
  }
  return (
    <a
      data-slot="rail-item"
      href={href}
      aria-current={current ? "page" : undefined}
      aria-label={expanded ? undefined : label}
      className={classes}
      {...props}
    >
      {content}
    </a>
  )
}

export function RailSpacer() {
  return <div className="flex-1" />
}

/** The expand and collapse toggle. Always the last thing in the rail, under Settings. */
export function RailToggle({ icon, className, ...props }: { icon?: React.ReactNode } & Omit<React.ComponentProps<"button">, "children" | "onClick">) {
  const { expanded, toggle } = useRail()
  return (
    <button
      data-slot="rail-toggle"
      type="button"
      aria-expanded={expanded}
      aria-label={expanded ? "Collapse navigation" : "Expand navigation"}
      className={cn(tile, tileSize(expanded), "cursor-pointer border-0 bg-transparent", className)}
      /* `{...props}` first, and the toggle last, because Radix's
         `TooltipTrigger asChild` injects its own composed `onClick` into these
         props. Spread after `onClick` it replaced the toggle outright, and the
         rail's expand/collapse did nothing at all: the button was a Tooltip
         trigger that owned the rail's only control. The toggle is not a
         consumer prop (the type above omits `onClick`), so it must win. */
      {...props}
      onClick={toggle}
    >
      {icon ?? <Icon name="sidebar" size={20} />}
      {expanded ? <span className="flex-1 text-left">Collapse</span> : null}
    </button>
  )
}
