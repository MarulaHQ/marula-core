"use client";
/**
 * InsightState — the pack's ONE empty/error shape, composed at a call site.
 *
 * Purpose: the pack is explicit that empty and error states share one shape:
 * the fruit/leaf/canopy line mark, an 18px semibold ink title, a 13px muted
 * sentence that says what happened, and the way out as buttons. The error
 * ornament adds one small cross beside the mark, in the status danger pair:
 * a failed read is an error message, and error copy wears the status pair
 * (docs/theme.md §3.4). It is still not the destructive red — nothing here
 * was destroyed.
 *
 * Why not the shared `EmptyState`: it renders the mark plus a single message
 * and one action, which cannot express the pack's title/hint/two-action
 * arrangement. The mark itself is the shared component (`EmptyStateMark`),
 * so the ornament is identical everywhere.
 *
 * Non-purpose: this never narrates why a surface is empty beyond the one
 * sentence, and it never offers a different way to do the thing.
 */
import { EmptyStateMark } from "@marula-core/ui";

export function InsightState({
  title,
  hint,
  tone = "empty",
  primary,
  secondary,
}: {
  title: string;
  hint: string;
  /** `error` adds the status-pair cross to the mark. */
  tone?: "empty" | "error";
  primary?: React.ReactNode;
  secondary?: React.ReactNode;
}): React.JSX.Element {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: 10,
        padding: "64px 24px",
        textAlign: "center",
      }}
    >
      <div style={{ position: "relative", display: "inline-flex" }}>
        <EmptyStateMark />
        {tone === "error" ? (
          <span
            aria-hidden="true"
            style={{
              position: "absolute",
              right: 14,
              bottom: 20,
              color: "var(--m-status-danger-fg)",
              fontSize: 18,
              fontWeight: 700,
              lineHeight: 1,
            }}
          >
            ×
          </span>
        ) : null}
      </div>
      <p
        style={{
          margin: 0,
          color: "var(--m-text)",
          fontSize: 18,
          fontWeight: 600,
          lineHeight: 1.3,
        }}
      >
        {title}
      </p>
      <p
        style={{
          margin: 0,
          maxWidth: "46ch",
          color: "var(--m-text-secondary)",
          fontSize: 13,
          lineHeight: 1.5,
        }}
      >
        {hint}
      </p>
      {primary !== undefined || secondary !== undefined ? (
        <div style={{ display: "flex", gap: 8, marginTop: 4 }}>
          {primary}
          {secondary}
        </div>
      ) : null}
    </div>
  );
}
