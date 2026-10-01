import { MARULA_ICONS, type IconName } from "./marula-icons"

export type IconProps = { name: IconName; size?: number; className?: string; "aria-label"?: string }

/**
 * The one way to draw an icon. These are the mockups' own icons (a custom 24px line set), not a third-party library.
 * Decorative by default; pass aria-label when the icon stands alone. Colour follows the text colour (currentColor).
 * Sizes used in the design: 14 (pills and chips), 16 (buttons, rows), 18 (top bar), 20 (rail, lists), 24 (empty states).
 */
export function Icon({ name, size = 20, className, ...rest }: IconProps) {
  const Svg = MARULA_ICONS[name]
  const label = rest["aria-label"]
  return <Svg size={size} className={className} aria-hidden={label ? undefined : true} aria-label={label} role={label ? "img" : undefined} />
}
export { MARULA_ICONS as ICONS }
export type { IconName }
