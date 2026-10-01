"use client";
/**
 * Kbd — reimplemented (wave41 lane 20) over the handoff kit's `Kbd`, so a
 * keyboard hint is drawn the way the kit defines it everywhere. The kit draws
 * it in Figtree (the mockups carry no monospace for app content), not the old
 * mono, squared chip.
 */
import type { HTMLAttributes } from "react";
import { Kbd as KitKbd } from "../kit";

export type KbdProps = HTMLAttributes<HTMLElement> & { tone?: "muted" | "inherit" };

export function Kbd({ className, tone, children, ...props }: KbdProps) {
  return (
    <KitKbd tone={tone} className={className} {...props}>
      {children}
    </KitKbd>
  );
}
