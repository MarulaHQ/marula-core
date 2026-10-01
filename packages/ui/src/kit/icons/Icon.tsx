import type { IconWeight } from "@phosphor-icons/react"
import { ICONS, type IconName } from "./icon-map"

export type IconProps = { name: IconName; size?: number; weight?: IconWeight; className?: string; "aria-label"?: string }

/** One way to draw an icon. Decorative by default; pass aria-label when it stands alone. */
export function Icon({ name, size = 20, weight = "regular", className, ...rest }: IconProps) {
  const C = ICONS[name] as React.ComponentType<{ size?: number; weight?: IconWeight; className?: string; "aria-hidden"?: boolean; "aria-label"?: string; role?: string }>
  const label = rest["aria-label"]
  return <C size={size} weight={weight} className={className} aria-hidden={label ? undefined : true} aria-label={label} role={label ? "img" : undefined} />
}
export { ICONS }
export type { IconName }
