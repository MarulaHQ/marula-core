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
        {/* The viewport is the popup's scroll container (Radix gives it
            `overflow: hidden auto`), and both of axe's relevant rules land on
            it. `scrollable-region-focusable` (serious) fires once the option
            list is tall enough to scroll, because the options are not tabbable.
            A bare `tabIndex` does not fix it: a focusable generic element is
            never flattened by axe, so it becomes an owned child of the listbox
            and `aria-required-children` (critical) fires instead — a listbox may
            own only `group` and `option`. `role="group"` is the role a listbox
            may own, so axe keeps counting the options below it, and the tab stop
            satisfies the scrollable rule. Guarded by
            tests/components/select-a11y.test.tsx and
            e2e/flows/select-a11y.spec.ts. Instances stay keyboard-driven:
            Radix focuses the selected item on open. */}
        <SelectPrimitive.Viewport role="group" tabIndex={0}>
          {children}
        </SelectPrimitive.Viewport>
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
