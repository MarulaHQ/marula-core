"use client";
/**
 * WidgetSkeleton — the Planning-loading skeleton for one widget card.
 *
 * Purpose: the flat muted skeleton the pack draws for planning while it
 * loads — no shimmer, no pulse (docs/theme.md motion law forbids it just as
 * the pack does), with a sentence that says what is loading.
 *
 * Non-purpose: no data semantics. The caller renders its own body once the
 * read resolves.
 */

export function WidgetSkeleton({ label }: { label: string }): React.JSX.Element {
  return (
    <div
      className="grid gap-3"
      aria-busy="true"
      role="status"
      aria-label={label}
    >
      <div className="items-end gap-2" style={{ display: "grid", gridTemplateColumns: "repeat(8, 1fr)", height: 120 }}>
        {[70, 85, 55, 90, 62, 40, 78, 50].map((h, i) => (
          <div key={i} className="grid content-end gap-1">
            <div
              style={{
                height: `${h}%`,
                background: "var(--m-surface-2)",
                borderRadius: 8,
              }}
            />
            <div style={{ height: 10, background: "var(--m-surface-2)", borderRadius: 5, width: "60%" }} />
          </div>
        ))}
      </div>
      <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
        Loading {label}…
      </p>
    </div>
  );
}
