"use client";
/**
 * Field wiring: Label, FieldError, FieldHint.
 * Purpose: label-above-field layout with accessible error/hint association
 * helpers; validation uses an icon plus color, never color alone (§6).
 */
import * as LabelPrimitive from "@radix-ui/react-label";
import { forwardRef, type ComponentProps, type ReactNode } from "react";
import { cn } from "../lib/cn";

export const Label = forwardRef<
  HTMLLabelElement,
  ComponentProps<typeof LabelPrimitive.Root>
>(({ className, ...props }, ref) => (
  <LabelPrimitive.Root
    ref={ref}
    className={cn("m-label", className)}
    {...props}
  />
));

Label.displayName = "Label";

/** Inline danger message with warning icon for field-level validation. */
export function FieldError({
  children,
  id,
  ...rest
}: {
  children: ReactNode;
  /** Pass as `aria-describedby` target on the control. */
  id?: string;
} & Omit<ComponentProps<"p">, "children">): ReactNode {
  return (
    <p id={id} {...rest} className="m-field-error" role="alert">
      <svg
        aria-hidden="true"
        width="12"
        height="12"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
      >
        <path d="M12 3 2 21h20L12 3z" strokeLinejoin="round" />
        <path d="M12 10v5" strokeLinecap="round" />
        <circle cx="12" cy="18" r="0.5" fill="currentColor" />
      </svg>
      {children}
    </p>
  );
}

/** Secondary helper text below a control. */
export function FieldHint({
  children,
  id,
}: {
  children: ReactNode;
  /** Pass as `aria-describedby` target on the control. */
  id?: string;
}): ReactNode {
  return (
    <p id={id} className="m-field-hint">
      {children}
    </p>
  );
}
