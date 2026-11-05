"use client";
/**
 * WidgetSkeleton — the Planning-loading skeleton for one widget card.
 *
 * Purpose: the flat muted skeleton the pack draws for planning while it
 * loads — no shimmer, no pulse (docs/theme.md motion law forbids it just as
 * the pack does), with a sentence that says what is loading. The bars are the
 * kit's `Skeleton` blocks and the announcement is `SkeletonGroup`, so the
 * loading shape lives in one place.
 *
 * Non-purpose: no data semantics. The caller renders its own body once the
 * read resolves.
 */
import { Skeleton, SkeletonGroup } from "@marula-core/ui";

export function WidgetSkeleton({ label }: { label: string }): React.JSX.Element {
  return (
    <SkeletonGroup label={label}>
      <div className="items-end gap-2" style={{ display: "grid", gridTemplateColumns: "repeat(8, 1fr)", height: 120 }}>
        {[70, 85, 55, 90, 62, 40, 78, 50].map((h, i) => (
          <div key={i} className="grid content-end gap-1">
            <Skeleton shape="block" height={`${h}%`} />
            <Skeleton shape="line" height={10} width="60%" />
          </div>
        ))}
      </div>
    </SkeletonGroup>
  );
}
