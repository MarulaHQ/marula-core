"use client";

// Capacity forecast — committed PROJECT load per technician, beside
// declared availability (Wave 34 claims close-out, claim 1).
//
// Purpose: `/dispatch/capacity` allocates against appointments only, so it
// cannot show the half of a technician's load that is committed project
// work. This panel is the other half, and the two together are the whole
// answer to "who is free".
//
// The unit is PERCENT-DAYS, not percent, and the header says so. 20% for a
// month and 80% for a week are the same commitment; a percentage-only
// column would tell a planner they are 20% and 80% loads respectively.
//
// Non-purposes: no editing (allocations are the panel below), and no
// forecast of its own — this reports stated commitments, not predicted
// demand. A predictive forecast would need history this product does not
// keep.

import {
  Card,
  CardHeader,
  EmptyState,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useState } from "react";
import { api } from "../../../lib/api";

type ForecastView = components["schemas"]["CapacityForecastView"];
type ForecastEntry = components["schemas"]["CapacityForecastEntryView"];

export function CapacityForecastPanel({
  days = 14,
  onError,
}: {
  days?: number;
  onError?: (message: string) => void;
}) {
  const [view, setView] = useState<ForecastView | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let live = true;
    (async () => {
      try {
        const data = await api.api.GET("/api/v1/dispatch/planning/forecast", {
          params: { query: { days } },
        });
        if (live && data.data) setView(data.data as ForecastView);
      } catch (e) {
        if (live) {
          onError?.(e instanceof Error ? e.message : "could not load the forecast");
        }
      } finally {
        if (live) setLoading(false);
      }
    })();
    return () => {
      live = false;
    };
  }, [days, onError]);

  if (loading) return <Spinner label="Loading the forecast" />;
  if (!view) return null;

  const overCount = view.entries.filter((e) => e.over_allocated).length;
  const committed = view.entries.filter((e) => e.committed_percent_days > 0);

  return (
    <Card>
      <CardHeader>
        <h2 className="p-4 pb-0 text-sm font-semibold">
          Committed project load
          <span className="ml-3 font-normal" style={{ color: "var(--m-text-secondary)" }}>
            {view.from} to {view.to}
          </span>
        </h2>
      </CardHeader>
      <p style={{ color: "var(--m-text-secondary)", fontSize: "12px" }}>
        Share of a normal full-time load, measured in percent-days. A
        technician is 200 percent-days committed by working 100% of their time
        for two days, or two phases at 100% for one day. Appointments are not
        counted here — this is the project side of the load.
      </p>

      {overCount > 0 ? (
        <p
          role="status"
          style={{
            color: "var(--m-status-danger-fg)",
            background: "var(--m-status-danger-bg)",
            fontSize: "12px",
            padding: "8px 10px",
            borderRadius: "var(--m-radius-sm)",
            marginTop: "8px",
          }}
        >
          {overCount === 1
            ? "1 technician is committed beyond their declared availability"
            : `${overCount} technicians are committed beyond their declared availability`}
        </p>
      ) : null}

      {committed.length === 0 ? (
        <EmptyState message="No project commitments in this window. Add an allocation below to see committed project load beside availability." />
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>Technician</Th>
              <Th>Committed</Th>
              <Th>Available</Th>
              <Th>Of load</Th>
              <Th>Phases</Th>
            </tr>
          </thead>
          <tbody>
            {committed.map((e) => (
              <ForecastRow key={e.user_id} entry={e} />
            ))}
          </tbody>
        </Table>
      )}

      {/* Technicians with declared availability but no commitment are worth
          showing too — a dispatcher triaging an over-booked week needs to
          know who has room, not only who does not. */}
      {view.entries.length > committed.length ? (
        <details style={{ marginTop: "8px" }}>
          <summary
            style={{
              cursor: "pointer",
              fontSize: "12px",
              color: "var(--m-text-secondary)",
            }}
          >
            {view.entries.length - committed.length} more with no project
            commitment
          </summary>
          <ul
            style={{
              listStyle: "none",
              padding: "8px 0 0",
              margin: 0,
              fontSize: "12px",
              color: "var(--m-text-secondary)",
            }}
          >
            {view.entries
              .filter((e) => e.committed_percent_days <= 0)
              .map((e) => (
                <li key={e.user_id} style={{ padding: "2px 0" }}>
                  {e.display_name}
                  {e.available_percent_days <= 0
                    ? " — no declared availability in this window"
                    : ` — ${formatPercentDays(e.available_percent_days)} available`}
                </li>
              ))}
          </ul>
        </details>
      ) : null}
    </Card>
  );
}

function ForecastRow({ entry }: { entry: ForecastEntry }) {
  // A technician with NO declared availability has no denominator, so there
  // is no ratio. Showing "infinite overload" or "0%" would both be lies:
  // the first invents a comparison, the second hides a real commitment.
  const noAvailability = entry.available_percent_days <= 0;
  const util = entry.utilization;

  return (
    <Tr>
      <Td>
        {entry.display_name}
        {entry.over_allocated ? (
          <Tag tone="danger" style={{ marginLeft: "6px" }}>
            over-allocated
          </Tag>
        ) : null}
      </Td>
      <Td>{formatPercentDays(entry.committed_percent_days)}</Td>
      <Td>
        {noAvailability
          ? "not declared"
          : formatPercentDays(entry.available_percent_days)}
      </Td>
      <Td>
        {noAvailability || util === null || util === undefined ? (
          <span style={{ color: "var(--m-text-secondary)" }}>—</span>
        ) : (
          <LoadBar ratio={util} />
        )}
      </Td>
      <Td>
        {entry.phase_count} {entry.phase_count === 1 ? "phase" : "phases"}
      </Td>
    </Tr>
  );
}

/** A utilisation bar with a 100% rule, so "over" is visible as overflow. */
function LoadBar({ ratio }: { ratio: number }) {
  const over = ratio > 1;
  const warn = !over && ratio >= 0.8;
  const pct = Math.min(ratio, 1) * 100;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "6px" }}>
      <span
        aria-hidden
        style={{
          position: "relative",
          display: "inline-block",
          width: "64px",
          height: "6px",
          background: "var(--m-surface-sunken)",
          borderRadius: "3px",
          overflow: "hidden",
        }}
      >
        <span
          style={{
            display: "block",
            width: `${pct}%`,
            height: "100%",
            background: over
              ? "var(--m-status-danger-fg)"
              : warn
                ? "var(--m-status-warning-fg)"
                : "var(--m-status-success-fg)",
          }}
        />
      </span>
      <span style={{ fontVariantNumeric: "tabular-nums" }}>
        {Math.round(ratio * 100)}%
      </span>
    </span>
  );
}

function formatPercentDays(v: number): string {
  return `${Math.round(v)} pd`;
}
