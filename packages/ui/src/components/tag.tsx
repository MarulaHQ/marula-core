"use client";
/**
 * Tag / StatusTag — reimplemented (wave41 lane 20) as a thin wrapper over the
 * handoff kit's `Pill`.
 *
 * The old rule here was "squared labels, pills are forbidden". That was the
 * pre-Marula doctrine and it contradicts the mockups, which draw every status
 * label as a pill. Since the mockups and the kit are the reference, `Tag` now
 * renders the kit's `Pill`; the name and props stay so its 100+ call sites do
 * not have to change.
 *
 * Non-purpose: this does not decide which tone a status maps to — that stays
 * with the surface, exactly as before.
 */
import { Pill } from "../kit";
import type { StatusTone } from "../tokens";

/** App status tone -> kit Pill tone, keeping the same colour pair where one exists. */
const PILL_TONE = {
  success: "success",
  warning: "progress",
  danger: "breached",
  neutral: "neutral",
} as const satisfies Record<StatusTone, "success" | "progress" | "breached" | "neutral">;

export interface TagProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: StatusTone | "outline";
}

export function Tag({ tone = "neutral", className, children, ...props }: TagProps) {
  const kitTone = tone === "outline" ? "neutral" : PILL_TONE[tone];
  return (
    <Pill tone={kitTone} className={className} {...props}>
      {children}
    </Pill>
  );
}

/** Convenience wrapper mapping a domain status string to a semantic tone. */
export function StatusTag({ tone, children }: { tone: StatusTone; children: string }): React.JSX.Element {
  return <Tag tone={tone}>{children}</Tag>;
}
