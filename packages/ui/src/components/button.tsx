/**
 * Button — Workbench §6: 2px radius, weight 500, solid Ember primary,
 * bordered secondary, outline destructive. Ember marks interactivity only.
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
