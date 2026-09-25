"use client";
// Routing strip (Wave 32 lane 7): the dispatch board's read-only view of
// the day's journey plan — typed impossible-travel warnings and
// suggestion-grade route improvements. Silent when there is nothing to
// say (ADR 0010 visibly-absent posture): no provider hint, no empty
// chrome, no nag. Suggestions never apply themselves; the dispatcher
// moves blocks on the board.
import { Tag } from "@meridian/ui";
import { useEffect, useState } from "react";
import { api } from "../lib/api";

type TravelWarningView = {
  kind: string;
  appointment_id: string | null;
  message: string;
  short_minutes?: number | null;
};

type TechnicianJourneyView = {
  user_id: string;
  display_name: string;
  total_travel_minutes: number;
  warnings: TravelWarningView[];
};

type TravelSuggestionView = {
  kind: string;
  appointment_id: string;
  minutes_saved: number;
  label: string;
};

type RoutingDayView = {
  date: string;
  status: { provider_id: string; quality: string; measured: boolean };
  technicians: TechnicianJourneyView[];
  suggestions: { available: boolean; reason?: string | null; items: TravelSuggestionView[] };
};

const BLOCKING = new Set(["zero_transfer", "insufficient_transfer"]);

export function RoutingStrip({ dayIso }: { dayIso: string }) {
  const [view, setView] = useState<RoutingDayView | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const { data, error } = await api.api.GET(
        "/api/v1/dispatch/routing" as never,
        { params: { query: { date: dayIso } } } as never,
      );
      if (cancelled || error !== undefined || data === undefined) return;
      setView(data as unknown as RoutingDayView);
    })();
    return () => {
      cancelled = true;
    };
  }, [dayIso]);

  if (view === null) return null;
  const blocking = view.technicians.flatMap((tech) =>
    tech.warnings
      .filter((warning) => BLOCKING.has(warning.kind))
      .map((warning) => ({ ...warning, tech: tech.display_name })),
  );
  const travel = view.technicians.filter((tech) => tech.total_travel_minutes > 0);
  const suggestions = view.suggestions.available ? view.suggestions.items : [];
  if (blocking.length === 0 && suggestions.length === 0 && travel.length === 0) {
    return null;
  }

  return (
    <div
      data-testid="routing-strip"
      className="flex flex-wrap items-center gap-x-4 gap-y-1 px-4 pb-2 text-[11px]"
    >
      {travel.length > 0 && (
        <span style={{ color: "var(--m-text-muted)" }}>
          Travel:{" "}
          {travel
            .map((tech) => `${tech.display_name} ${tech.total_travel_minutes} min`)
            .join(" · ")}
          {view.status.measured ? "" : " (estimated)"}
        </span>
      )}
      {blocking.length > 0 && (
        <span className="flex flex-wrap items-center gap-2">
          <Tag tone="warning">
            {blocking.length} travel {blocking.length === 1 ? "issue" : "issues"}
          </Tag>
          {blocking.slice(0, 4).map((warning, index) => (
            <span
              key={`${warning.kind}-${warning.appointment_id ?? index}`}
              title={warning.message}
              style={{ color: "var(--m-status-warning-fg)" }}
            >
              {warning.tech}:{" "}
              {warning.kind === "zero_transfer"
                ? "no transfer time"
                : `short ${warning.short_minutes ?? "?"} min`}
            </span>
          ))}
        </span>
      )}
      {suggestions.length > 0 && (
        <span className="flex flex-wrap items-center gap-2">
          <Tag tone="neutral">Route suggestions</Tag>
          {suggestions.slice(0, 3).map((suggestion, index) => (
            <span
              key={`${suggestion.kind}-${suggestion.appointment_id}-${index}`}
              title="Suggestion only — apply it by moving the block."
              style={{ color: "var(--m-text-secondary)" }}
            >
              {suggestion.label}
            </span>
          ))}
        </span>
      )}
    </div>
  );
}
