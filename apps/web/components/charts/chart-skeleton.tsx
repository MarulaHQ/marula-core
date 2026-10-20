"use client";
/**
 * ChartSkeleton — the placeholder behind a lazily loaded chart.
 *
 * Purpose: hold the chart's exact box while its chunk loads, so nothing on the
 * card moves when the chart arrives. The bars come from the same muted surface
 * token the chart grid sits on, carry no numbers and are aria-hidden; the
 * region announces "loading" to a screen reader instead.
 *
 * Sizing is copied from the renderer rather than guessed. A chart that draws at
 * an intrinsic width and scales by CSS (`width` given) reserves that aspect
 * ratio; a chart in a `ResponsiveContainer` (`height` only) reserves the fixed
 * pixel height it will occupy.
 *
 * Non-purpose: no pulse or shimmer (docs/theme.md motion law forbids it), and
 * no spinner — a spinner announces loading while collapsing the layout.
 */
export function ChartSkeleton({
  label,
  height,
  width,
}: {
  /** The chart's accessible name, echoed into the status text. */
  label: string;
  /** Rendered height in px when the chart does not scale. */
  height: number;
  /** Intrinsic width when the chart scales to the card (aspect-ratio mode). */
  width?: number;
}) {
  const box =
    width === undefined
      ? { height }
      : { aspectRatio: `${width} / ${height}`, width: "100%" };
  return (
    <div
      className="relative"
      style={box}
      role="status"
      aria-busy="true"
      aria-label={`Loading ${label}`}
      data-testid="chart-skeleton"
    >
      <div
        aria-hidden="true"
        className="absolute inset-0 grid items-end gap-2"
        style={{ gridTemplateColumns: "repeat(6, 1fr)" }}
      >
        {[58, 86, 44, 72, 96, 62].map((share, index) => (
          <div
            key={index}
            className="rounded-md"
            style={{
              height: `${share}%`,
              background: "var(--m-surface-2)",
            }}
          />
        ))}
      </div>
    </div>
  );
}
