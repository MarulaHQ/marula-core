/**
 * Kbd — inline keyboard shortcut chip (`⌘K`), mono font, squared.
 */
import { forwardRef, type HTMLAttributes } from "react";
import { cn } from "../lib/cn";

export type KbdProps = HTMLAttributes<HTMLElement>;

export const Kbd = forwardRef<HTMLElement, KbdProps>(
  ({ className, ...props }, ref) => (
    <kbd ref={ref} className={cn("m-kbd", className)} {...props} />
  ),
);

Kbd.displayName = "Kbd";
