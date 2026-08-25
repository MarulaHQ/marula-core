/**
 * Tooltip — Radix tooltip on a bordered overlay surface.
 */
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { forwardRef, type ComponentProps, type ReactNode } from "react";
import { cn } from "../lib/cn";

export const TooltipProvider = TooltipPrimitive.Provider;
export const TooltipRoot = TooltipPrimitive.Root;
export const TooltipTrigger = TooltipPrimitive.Trigger;

export type TooltipContentProps = ComponentProps<typeof TooltipPrimitive.Content>;

export const TooltipContent = forwardRef<HTMLDivElement, TooltipContentProps>(
  ({ className, sideOffset = 4, ...props }, ref) => (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        ref={ref}
        sideOffset={sideOffset}
        className={cn("m-tooltip-content", className)}
        {...props}
      />
    </TooltipPrimitive.Portal>
  ),
);

TooltipContent.displayName = "TooltipContent";

/** Convenience wrapper pairing a trigger with content. */
export function Tooltip({
  trigger,
  children,
}: {
  trigger: ReactNode;
  children: ReactNode;
}): React.JSX.Element {
  return (
    // Dense workbench UX: the tooltip hides as soon as the pointer leaves the
    // trigger instead of tolerating a hoverable gap over the content.
    <TooltipRoot disableHoverableContent>
      <TooltipTrigger asChild>{trigger}</TooltipTrigger>
      <TooltipContent>{children}</TooltipContent>
    </TooltipRoot>
  );
}
