"use client";
/**
 * Lazy boundary for the report builder's preview chart.
 *
 * Purpose: `/reports` is the heaviest workspace route and Recharts is the reason
 * — the builder rendered its preview straight from `report-preview.tsx`, so the
 * charting library shipped with the builder form and every visitor paid for it
 * before they had a report to look at. Behind `next/dynamic` the library loads
 * with the first preview instead.
 *
 * The fallback is 240px tall, the exact height `ResponsiveContainer` occupies in
 * `report-preview.tsx`, so the preview slot does not jump when the chart lands.
 *
 * Non-purpose: no behaviour change — `ReportChart` keeps its props, aria label
 * and token colours; only the module's arrival moved.
 */
import dynamic from "next/dynamic";
import { ChartSkeleton } from "./charts/chart-skeleton";

/** `CHART_HEIGHT` in report-preview.tsx — the ResponsiveContainer's height. */
const REPORT_CHART_HEIGHT = 240;

function ReportChartFallback() {
  return <ChartSkeleton label="report preview" height={REPORT_CHART_HEIGHT} />;
}

export const ReportChart = dynamic(
  () => import("./report-preview").then((m) => m.ReportChart),
  { ssr: false, loading: ReportChartFallback },
);
