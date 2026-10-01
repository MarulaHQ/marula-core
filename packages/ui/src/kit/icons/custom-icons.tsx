// Icons Phosphor has no equivalent for: the three density glyphs (3, 4 and 5 evenly spaced rows).
// Same props as a Phosphor icon, so <Icon name="rows-4" /> behaves like the rest.
import type { SVGProps } from "react"
type Props = Omit<SVGProps<SVGSVGElement>, "ref"> & { size?: number | string; weight?: string; mirrored?: boolean }
function make(name: string, d: React.ReactNode) {
  const C = ({ size = 20, weight: _w, mirrored: _m, ...rest }: Props) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.75} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...rest}>{d}</svg>
  )
  C.displayName = name
  return C
}
export const Rows3 = make("Rows3", <><path d="M4 6h16M4 12h16M4 18h16"/></>)
export const Rows4 = make("Rows4", <><path d="M4 5h16M4 9.7h16M4 14.3h16M4 19h16"/></>)
export const Rows5 = make("Rows5", <><path d="M4 4h16M4 8h16M4 12h16M4 16h16M4 20h16"/></>)
