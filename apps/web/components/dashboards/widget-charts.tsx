// Recharts renderers for dashboard widgets (ADR 0001: Recharts on
// Workbench tokens).
//
// Purpose: token-driven bar/line charts. Colors come exclusively from
// chartTokensFor(theme); entrance animations are disabled outright (motion
// law: transform/opacity only, nothing animates in). Fixed intrinsic sizes
// keep static rows predictable and render deterministically headless.
//
// Non-purpose: no data fetching; the widget card owns loading/empty/error.
"use client";

import { useTheme } from "@meridian/ui";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Line,
  LineChart,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { chartTokensFor } from "../../lib/chart-tokens";
import type {
  AgingRow,
  RollupRow,
  SlaAttainmentRow,
  ThroughputRow,
} from "../../lib/reports";
import { AGING_BUCKET_LABELS } from "../../lib/dashboards";

const CHART_HEIGHT = 200;
const CATEGORY_AXIS_WIDTH = 132;

function useChartTokens() {
  const { theme } = useTheme();
  return chartTokensFor(theme);
}

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

/** Horizontal bar rollup (time by person / by client): hours per label. */
export function RollupBarChart({
  rows,
  ariaLabel,
}: {
  rows: RollupRow[];
  ariaLabel: string;
}) {
  const t = useChartTokens();
  const data = rows.map((r) => ({
    name: r.label !== "" ? r.label : r.group_id.slice(0, 8),
    hours: Math.round((r.metric / 60) * 10) / 10,
  }));
  return (
    <figure role="img" aria-label={ariaLabel}>
      <BarChart
        width={720}
        height={CHART_HEIGHT}
        data={data}
        layout="vertical"
        margin={{ top: 4, right: 24, bottom: 4, left: 8 }}
      >
        <CartesianGrid stroke={t.grid} horizontal={false} />
        <XAxis
          type="number"
          tick={{ fill: t.tick, fontSize: 12 }}
          stroke={t.grid}
          tickLine={false}
        />
        <YAxis
          type="category"
          dataKey="name"
          width={CATEGORY_AXIS_WIDTH}
          tick={{ fill: t.tick, fontSize: 12 }}
          stroke={t.grid}
          tickLine={false}
        />
        <Tooltip {...tooltipStyle(t)} />
        <Bar dataKey="hours" fill={t.primary} isAnimationActive={false} />
      </BarChart>
    </figure>
  );
}

/** Attainment bars: percent per policy, pinned to a 0-100 domain. */
export function AttainmentBarChart({
  rows,
  ariaLabel,
}: {
  rows: SlaAttainmentRow[];
  ariaLabel: string;
}) {
  const t = useChartTokens();
  const data = rows.map((r) => ({
    name: r.label !== "" ? r.label : r.group_id.slice(0, 8),
    pct: r.attainment_pct,
  }));
  return (
    <figure role="img" aria-label={ariaLabel}>
      <BarChart
        width={720}
        height={CHART_HEIGHT}
        data={data}
        layout="vertical"
        margin={{ top: 4, right: 24, bottom: 4, left: 8 }}
      >
        <CartesianGrid stroke={t.grid} horizontal={false} />
        <XAxis
          type="number"
          domain={[0, 100]}
          tick={{ fill: t.tick, fontSize: 12 }}
          stroke={t.grid}
          tickLine={false}
        />
        <YAxis
          type="category"
          dataKey="name"
          width={CATEGORY_AXIS_WIDTH}
          tick={{ fill: t.tick, fontSize: 12 }}
          stroke={t.grid}
          tickLine={false}
        />
        <Tooltip {...tooltipStyle(t)} />
        <Bar dataKey="pct" fill={t.primary} isAnimationActive={false} />
      </BarChart>
    </figure>
  );
}

/** Aging distribution: fixed bucket order on the x axis. */
export function AgingBarChart({
  rows,
  ariaLabel,
}: {
  rows: AgingRow[];
  ariaLabel: string;
}) {
  const t = useChartTokens();
  const data = [...rows]
    .sort((a, b) => a.bucket_ordinal - b.bucket_ordinal)
    .map((r) => ({
      name: AGING_BUCKET_LABELS[r.bucket] ?? r.bucket,
      open: r.open_count,
    }));
  return (
    <figure role="img" aria-label={ariaLabel}>
      <BarChart
        width={720}
        height={CHART_HEIGHT}
        data={data}
        margin={{ top: 4, right: 24, bottom: 4, left: 8 }}
      >
        <CartesianGrid stroke={t.grid} vertical={false} />
        <XAxis
          dataKey="name"
          tick={{ fill: t.tick, fontSize: 12 }}
          stroke={t.grid}
          tickLine={false}
        />
        <YAxis
          allowDecimals={false}
          tick={{ fill: t.tick, fontSize: 12 }}
          stroke={t.grid}
          tickLine={false}
          width={40}
        />
        <Tooltip {...tooltipStyle(t)} />
        <Bar dataKey="open" fill={t.primary} isAnimationActive={false} />
      </BarChart>
    </figure>
  );
}

/** Throughput: created vs resolved per day (two contrast-token series). */
export function ThroughputLineChart({
  rows,
  ariaLabel,
}: {
  rows: ThroughputRow[];
  ariaLabel: string;
}) {
  const t = useChartTokens();
  const data = rows.map((r) => ({
    day: r.day,
    Created: r.created_count,
    Resolved: r.resolved_count,
  }));
  return (
    <figure role="img" aria-label={ariaLabel}>
      <LineChart
        width={720}
        height={CHART_HEIGHT}
        data={data}
        margin={{ top: 4, right: 24, bottom: 4, left: 8 }}
      >
        <CartesianGrid stroke={t.grid} vertical={false} />
        <XAxis
          dataKey="day"
          tick={{ fill: t.tick, fontSize: 12 }}
          stroke={t.grid}
          tickLine={false}
        />
        <YAxis
          allowDecimals={false}
          tick={{ fill: t.tick, fontSize: 12 }}
          stroke={t.grid}
          tickLine={false}
          width={40}
        />
        <Tooltip {...tooltipStyle(t)} />
        <Line
          type="monotone"
          dataKey="Created"
          stroke={t.primary}
          dot={false}
          isAnimationActive={false}
        />
        <Line
          type="monotone"
          dataKey="Resolved"
          stroke={t.secondary}
          dot={false}
          isAnimationActive={false}
        />
      </LineChart>
    </figure>
  );
}
