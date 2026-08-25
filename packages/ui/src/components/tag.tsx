/**
 * Tag / StatusTag — squared (2px) status labels per theme.md §6; pills are
 * forbidden. Colors come exclusively from the semantic status tokens.
 */
import { forwardRef, type HTMLAttributes } from "react";
import { cn } from "../lib/cn";
import type { StatusTone } from "../tokens";

const toneClass: Record<StatusTone, string> = {
  success: "m-tag-success",
  warning: "m-tag-warning",
  danger: "m-tag-danger",
  neutral: "m-tag-neutral",
};

export interface TagProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: StatusTone | "outline";
}

export const Tag = forwardRef<HTMLSpanElement, TagProps>(
  ({ className, tone = "neutral", ...props }, ref) => (
    <span
      ref={ref}
      className={cn(
        "m-tag",
        tone === "outline" ? "m-tag-outline" : toneClass[tone],
        className,
      )}
      {...props}
    />
  ),
);

Tag.displayName = "Tag";

/** Convenience wrapper mapping a domain status string to a semantic tone. */
export function StatusTag({
  tone,
  children,
}: {
  tone: StatusTone;
  children: string;
}): React.JSX.Element {
  return <Tag tone={tone}>{children}</Tag>;
}
