"use client";
/**
 * Select — Radix Select styled to match inputs; highlighted items use the
 * greyscale canvas, keeping Ember scarce (§2 rule 4).
 */
import * as SelectPrimitive from "@radix-ui/react-select";
import { forwardRef, type ComponentProps, type ReactNode } from "react";
import { cn } from "../lib/cn";

export const Select = SelectPrimitive.Root;
export const SelectValue = SelectPrimitive.Value;

/**
 * A titled band inside the menu, for a picker whose options come from a nested
 * source (a catalogue, a set of families) rather than one flat list. The label
 * is a caption, never a selectable row: it stays out of the keyboard order.
 */
export const SelectGroup = SelectPrimitive.Group;

export type SelectLabelProps = ComponentProps<typeof SelectPrimitive.Label>;

export function SelectLabel({ className, children, ...props }: SelectLabelProps): ReactNode {
  return (
    <SelectPrimitive.Label className={cn("m-select-label", className)} {...props}>
      {children}
    </SelectPrimitive.Label>
  );
}

export type SelectTriggerProps = ComponentProps<typeof SelectPrimitive.Trigger>;

export const SelectTrigger = forwardRef<HTMLButtonElement, SelectTriggerProps>(
  ({ className, children, ...props }, ref) => (
    <SelectPrimitive.Trigger
      ref={ref}
      className={cn("m-select-trigger", className)}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon>
        <svg aria-hidden="true" width="10" height="10" viewBox="0 0 16 16" fill="none">
          <path d="m3 6 5 5 5-5" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  ),
);

SelectTrigger.displayName = "SelectTrigger";

export type SelectContentProps = ComponentProps<typeof SelectPrimitive.Content>;

export function SelectContent({
  className,
  children,
  ...props
}: SelectContentProps): ReactNode {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content className={cn("m-select-content", className)} {...props}>
        <SelectPrimitive.Viewport>{children}</SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

export type SelectItemProps = ComponentProps<typeof SelectPrimitive.Item>;

export function SelectItem({
  className,
  children,
  ...props
}: SelectItemProps): ReactNode {
  return (
    <SelectPrimitive.Item className={cn("m-select-item", className)} {...props}>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      <SelectPrimitive.ItemIndicator>
        <svg aria-hidden="true" width="11" height="11" viewBox="0 0 16 16" fill="none">
          <path
            d="M3 8.5 6.5 12 13 4.5"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
}
