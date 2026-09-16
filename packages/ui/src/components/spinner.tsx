"use client";
/**
 * Spinner — indeterminate progress ring announced via role="status".
 * Animation is transform-only and honors prefers-reduced-motion.
 */
import type { HTMLAttributes } from "react";
import { cn } from "../lib/cn";

export interface SpinnerProps extends HTMLAttributes<HTMLSpanElement> {
  /** Accessible label; defaults to "Loading". */
  label?: string;
}

export function Spinner({
  className,
  label = "Loading",
  ...props
}: SpinnerProps): React.JSX.Element {
  return (
    <span role="status" aria-label={label} className={cn("m-spinner", className)} {...props} />
  );
}
