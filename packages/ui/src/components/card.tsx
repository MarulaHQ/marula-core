/**
 * Card — bordered surface container; 3px max radius, border over shadow (§5).
 */
import { forwardRef, type HTMLAttributes, type ReactNode } from "react";
import { cn } from "../lib/cn";

export const Card = forwardRef<HTMLDivElement, HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div ref={ref} className={cn("m-card", className)} {...props} />
  ),
);

Card.displayName = "Card";

export function CardHeader({ children }: { children: ReactNode }): ReactNode {
  return <div className="m-card-header">{children}</div>;
}

export function CardContent({ children }: { children: ReactNode }): ReactNode {
  return <div className="m-card-content">{children}</div>;
}
