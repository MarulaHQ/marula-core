// Report preview renderer (R6 builder; ADR 0001: Recharts on Workbench
// tokens). Token colors via chart-tokens.ts exclusively; entrance
// animations off (motion law); bar/line/pie per the spec's viz hint, table
// always available. Responsive via ResponsiveContainer across display
// sizes; prefers-reduced-motion is honored by disabling animation.
"use client";

import { useTheme } from "@marula-core/ui";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Legend,
  Line,
  LineChart,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { chartTokensFor } from "../lib/chart-tokens";
import { labelize, previewSeries, type MeasureDraft } from "../lib/report-builder";

const CHART_HEIGHT = 240;

const PIE_SHADES = ["primary", "secondary", "grid", "tick"] as const;

function tooltipStyle(t: ReturnType<typeof chartTokensFor>) {
  return {
    contentStyle: {
      background: t.tooltipBg,
      color: t.tooltipText,
      border: `1px solid ${t.tooltipBorder}`,
      borderRadius: 3,
      fontSize: 12,
    },
    labelStyle: { color: t.tooltipText },
    itemStyle: { color: t.tooltipText },
    cursor: { fill: "transparent" },
  };
}

export function ReportChart({
  page,
  measures,
  viz,
  ariaLabel,
}: {
  page: { columns: string[]; rows: Record<string, unknown>[] };
  measures: MeasureDraft[];
  viz: "bar" | "line" | "pie";
  ariaLabel: string;
}) {
  const { theme } = useTheme();
  const t = chartTokensFor(theme);
  const { labels, series } = previewSeries(page, measures);
  const data = labels.map((label, i) => {
    const row: Record<string, string | number> = { name: label };
    for (const s of series) row[s.name] = s.values[i] ?? 0;
    return row;
  });
  const xKey = "name";
  const firstSeries = series[0]?.name ?? "value";

  if (viz === "pie") {
    const pieData = labels.map((label, i) => ({
      name: label,
      value: series[0]?.values[i] ?? 0,
    }));
    return (
      <figure role="img" aria-label={ariaLabel} className="min-w-0">
        <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
          <PieChart accessibilityLayer={false}>
            <Pie data={pieData} dataKey="value" nameKey="name" isAnimationActive={false} stroke={t.tooltipBorder}>
              {pieData.map((_, i) => {
                const key = PIE_SHADES[i % PIE_SHADES.length]!;
                return <Cell key={i} fill={t[key]} />;
              })}
            </Pie>
            <Tooltip {...tooltipStyle(t)} />
            <Legend wrapperStyle={{ color: t.tick, fontSize: 12 }} />
          </PieChart>
        </ResponsiveContainer>
      </figure>
    );
  }

  if (viz === "line") {
    return (
      <figure role="img" aria-label={ariaLabel} className="min-w-0">
        <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
          <LineChart
            accessibilityLayer={false}
            data={data}
            margin={{ top: 4, right: 16, bottom: 4, left: 0 }}
          >
            <CartesianGrid stroke={t.grid} vertical={false} />
            <XAxis dataKey={xKey} tick={{ fill: t.tick, fontSize: 12 }} stroke={t.grid} />
            <YAxis tick={{ fill: t.tick, fontSize: 12 }} stroke={t.grid} width={56} />
            <Tooltip {...tooltipStyle(t)} />
            {series.length > 1 ? <Legend wrapperStyle={{ color: t.tick, fontSize: 12 }} /> : null}
            {series.map((s, i) => (
              <Line
                key={s.name}
                type="monotone"
                dataKey={s.name}
                stroke={i === 0 ? t.primary : t.secondary}
                strokeWidth={i === 0 ? 2 : 1}
                dot={false}
                isAnimationActive={false}
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </figure>
    );
  }

  return (
    <figure role="img" aria-label={ariaLabel} className="min-w-0">
      <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
        <BarChart
          accessibilityLayer={false}
          data={data}
          margin={{ top: 4, right: 16, bottom: 4, left: 0 }}
        >
          <CartesianGrid stroke={t.grid} vertical={false} />
          <XAxis dataKey={xKey} tick={{ fill: t.tick, fontSize: 12 }} stroke={t.grid} />
          <YAxis tick={{ fill: t.tick, fontSize: 12 }} stroke={t.grid} width={56} />
          <Tooltip {...tooltipStyle(t)} />
          {series.length > 1 ? <Legend wrapperStyle={{ color: t.tick, fontSize: 12 }} /> : null}
          {series.map((s, i) => (
            <Bar
              key={s.name}
              dataKey={s.name}
              fill={i === 0 ? t.primary : t.secondary}
              isAnimationActive={false}
              maxBarSize={48}
            />
          ))}
        </BarChart>
      </ResponsiveContainer>
      <span className="sr-only">{`${ariaLabel} — first series ${labelize(firstSeries)}`}</span>
    </figure>
  );
}
