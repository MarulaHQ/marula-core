"use client";
/**
 * Separator — Radix separator styled as a 1px token-colored rule.
 */
import * as SeparatorPrimitive from "@radix-ui/react-separator";
import { forwardRef, type ComponentProps } from "react";
import { cn } from "../lib/cn";

export type SeparatorProps = ComponentProps<typeof SeparatorPrimitive.Root>;

export const Separator = forwardRef<HTMLDivElement, SeparatorProps>(
  ({ className, orientation = "horizontal", ...props }, ref) => (
    <SeparatorPrimitive.Root
      ref={ref}
      orientation={orientation}
      className={cn(
        "m-separator",
        orientation === "vertical"
          ? "m-separator-vertical"
          : "m-separator-horizontal",
        className,
      )}
      {...props}
    />
  ),
);

Separator.displayName = "Separator";
