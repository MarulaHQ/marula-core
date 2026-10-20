"use client";
/**
 * Lazy boundary for the dashboard chart renderers.
 *
 * Purpose: keep Recharts out of the workspace shell. `widget-charts.tsx` is the
 * only module in `components/dashboards` that touches Recharts, and the chunk it
 * lands in is 100.4 kB gzipped — paid by every workspace route before this
 * boundary existed. `next/dynamic` gives it its own chunk, so the cost lands with the
 * first widget that actually draws a chart.
 *
 * The fallback reserves the chart's own box so the card does not reflow when
 * the chunk arrives: each export carries the intrinsic size its chart draws at
 * (720x200, or 1360x200 for the full-width lead widget), and the skeleton turns
 * that into the same aspect ratio the CSS-scaled SVG will have. `ssr: false`
 * because Recharts measures the DOM and draws nothing useful on the server.
 *
 * Non-purpose: no presentation changes — every chart keeps the props, aria
 * labels and token colours it had when it was imported directly.
 */
import dynamic from "next/dynamic";
import { ChartSkeleton } from "../charts/chart-skeleton";

const CHART_HEIGHT = 200;

function RollupBarFallback() {
  return <ChartSkeleton label="chart" height={CHART_HEIGHT} width={720} />;
}
function WideRollupBarFallback() {
  return <ChartSkeleton label="chart" height={CHART_HEIGHT} width={1360} />;
}
function AttainmentFallback() {
  return <ChartSkeleton label="chart" height={CHART_HEIGHT} width={720} />;
}
function AgingFallback() {
  return <ChartSkeleton label="chart" height={CHART_HEIGHT} width={720} />;
}
function ThroughputFallback() {
  return <ChartSkeleton label="chart" height={CHART_HEIGHT} width={720} />;
}

/** Horizontal bar rollup (time by person / by client), 720x200 box. */
export const RollupBarChart = dynamic(
  () => import("./widget-charts").then((m) => m.RollupBarChart),
  { ssr: false, loading: RollupBarFallback },
);

/** The full-width lead rollup draws at 1360x200, so its fallback does too. */
export const WideRollupBarChart = dynamic(
  () => import("./widget-charts").then((m) => m.RollupBarChart),
  { ssr: false, loading: WideRollupBarFallback },
);

export const AttainmentBarChart = dynamic(
  () => import("./widget-charts").then((m) => m.AttainmentBarChart),
  { ssr: false, loading: AttainmentFallback },
);

export const AgingBarChart = dynamic(
  () => import("./widget-charts").then((m) => m.AgingBarChart),
  { ssr: false, loading: AgingFallback },
);

export const ThroughputLineChart = dynamic(
  () => import("./widget-charts").then((m) => m.ThroughputLineChart),
  { ssr: false, loading: ThroughputFallback },
);
