"use client";
/**
 * Button — Marula §6: 10px radius, weight 500, and a primary that flips with
 * the mode (ink with gold text in light, gold with ink text in dark).
 *
 * The `assistant` variant is the one control that lives INSIDE the assistant
 * surface, so it inverts with it: the card is the single inverted surface on
 * a screen, and an action on that card is gold on ink rather than the
 * surface's own cream, because gold-on-cream is 1.7:1 and unreadable.
 */
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../lib/cn";

const buttonVariants = cva("m-btn", {
  variants: {
    variant: {
      primary: "m-btn-primary",
      secondary: "m-btn-secondary",
      ghost: "m-btn-ghost",
      destructive: "m-btn-destructive",
      "destructive-solid": "m-btn-destructive-solid",
      assistant: "m-btn-assistant",
      "assistant-outline": "m-btn-assistant-outline",
    },
    size: {
      md: "",
      sm: "m-btn-sm",
    },
  },
  defaultVariants: {
    variant: "secondary",
    size: "md",
  },
});

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  /** Render as the immediate child element (e.g. a router link). */
  asChild?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, type, ...props }, ref) => {
    const Component = asChild ? Slot : "button";
    return (
      <Component
        ref={ref}
        className={cn(buttonVariants({ variant, size }), className)}
        {...(asChild ? {} : { type: type ?? "button" })}
        {...props}
      />
    );
  },
);

Button.displayName = "Button";
