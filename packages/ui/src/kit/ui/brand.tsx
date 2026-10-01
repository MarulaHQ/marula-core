import * as React from "react"
import { cn } from "../../lib/cn"

/** The marula fruit mark. */
export function MarulaMark({ size = 34, className }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" aria-hidden="true" className={className}>
      <path d="M32 17 L32 11" stroke="var(--mc-stem)" strokeWidth="2.5" strokeLinecap="round" fill="none" />
      <path d="M32 12 C37 4 49 3 55 7 C50 14 39 16 32 12 Z" fill="var(--mc-leaf)" />
      <circle cx="32" cy="38" r="22" fill="#E8B83A" />
      <ellipse cx="24" cy="31" rx="6" ry="4" fill="#F6EFD9" opacity=".75" />
    </svg>
  )
}

/** Wordmark: the mark and "marula" in the serif. The serif is for this and for marketing only; the interface is Figtree. */
export function Wordmark({ className, size = 26 }: { className?: string; size?: number }) {
  return (
    <span className={cn("inline-flex items-center gap-2.5 font-display font-medium tracking-[-0.03em]", className)} style={{ fontSize: size }}>
      <MarulaMark size={Math.round(size * 1.3)} />
      marula
    </span>
  )
}

/** The line drawing of a marula used in empty states. Quiet on purpose. */
export function FruitLine({ size = 80, className }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 72 72" fill="none" aria-hidden="true" className={className}>
      <circle cx="34" cy="42" r="20" stroke="var(--mc-stem)" strokeWidth="2.5" />
      <path d="M34 22c1-8 8-12 16-10-1 8-8 12-16 10z" stroke="var(--input)" strokeWidth="2.5" strokeLinejoin="round" />
      <path d="M22 12v6M19 15h6M60 30v5M57.5 32.5h5" stroke="var(--input)" strokeWidth="2.5" strokeLinecap="round" />
    </svg>
  )
}
