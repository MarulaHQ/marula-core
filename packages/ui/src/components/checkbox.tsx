"use client";
/**
 * Checkbox — Radix behavior, 16px squared box; checked state fills Ember
 * (interactive affordance) with on-accent checkmark.
 */
import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import { forwardRef, type ComponentProps, type ReactNode } from "react";
import { cn } from "../lib/cn";

export type CheckboxProps = Omit<
  ComponentProps<typeof CheckboxPrimitive.Root>,
  "asChild"
>;

export const Checkbox = forwardRef<HTMLButtonElement, CheckboxProps>(
  ({ className, ...props }, ref) => (
    <CheckboxPrimitive.Root
      ref={ref}
      className={cn("m-checkbox", className)}
      {...props}
    >
      <CheckboxPrimitive.Indicator>
        {props.checked === "indeterminate" ? (
          <DashIcon />
        ) : (
          <CheckIcon />
        )}
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  ),
);

Checkbox.displayName = "Checkbox";

function CheckIcon(): ReactNode {
  return (
    <svg aria-hidden="true" width="11" height="11" viewBox="0 0 16 16" fill="none">
      <path
        d="M3 8.5 6.5 12 13 4.5"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function DashIcon(): ReactNode {
  return (
    <svg aria-hidden="true" width="11" height="11" viewBox="0 0 16 16" fill="none">
      <path d="M3.5 8h9" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" />
    </svg>
  );
}
