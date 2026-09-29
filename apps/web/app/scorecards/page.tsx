// /scorecards — per-tech / service-desk coaching scorecards (Wave 14
// lane 3, R5).
//
// Purpose: staff-facing scorecard pages over the aggregation read model —
// SLA attainment, response/resolution latency, workload mix (APPROVED
// time entries only), and the opt-in gamification toggle. Private by
// default: every staff member sees their own card; the team view is
// owner/admin only (the API enforces it; the UI merely mirrors it).
//
// Instant-first reads, typed reason-code copy (never raw JSON),
// Workbench tokens, light+dark, ≤120ms transitions, responsive.
"use client";

import { Button, Card, Checkbox, EmptyState, FieldHint, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Spinner, Table, Tag, Td, Th, Tr } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import { isManagerPermissions } from "../../lib/dashboards";

type Scorecard = components["schemas"]["Scorecard"];
type TeamScorecardsResponse = components["schemas"]["TeamScorecardsResponse"];
type ScorecardSettingsResponse = components["schemas"]["ScorecardSettingsResponse"];

const WINDOWS = [
  { label: "Last 7 days", days: 7 },
  { label: "Last 30 days", days: 30 },
  { label: "Last 90 days", days: 90 },
] as const;

function utcDayOffset(days: number): string {
  const d = new Date();
  d.setUTCDate(d.getUTCDate() - days + 1);
  return d.toISOString().slice(0, 10);
}

/** Seconds → compact human latency ("1h 24m", "3m 05s"). */
function fmtSecs(secs: number | null | undefined): string {
  if (secs === null || secs === undefined) return "—";
  const s = Math.max(0, Math.round(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h > 0) return `${h}h ${String(m).padStart(2, "0")}m`;
  return `${m}m ${String(s % 60).padStart(2, "0")}s`;
}

function fmtPct(ratio: number | null | undefined): string {
  if (ratio === null || ratio === undefined) return "—";
  return `${Math.round(ratio * 100)}%`;
}

function fmtHours(minutes: number): string {
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/** Attainment → semantic tone (≥90% success, ≥70% warning, else danger). */
function attainmentTone(ratio: number | null | undefined): "success" | "warning" | "danger" | "neutral" {
  if (ratio === null || ratio === undefined) return "neutral";
  if (ratio >= 0.9) return "success";
  if (ratio >= 0.7) return "warning";
  return "danger";
}

function Metric({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div
      className="p-3 min-w-0"
      style={{ border: "1px solid var(--m-border)" }}
    >
      <div className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
        {label}
      </div>
      <div className="mt-1 text-[15px] font-semibold tabular-nums" style={{ color: "var(--m-text)" }}>
        {value}
      </div>
      {hint ? (
        <div className="mt-0.5 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          {hint}
        </div>
      ) : null}
    </div>
  );
}

function ScorecardBody({ card }: { card: Scorecard }) {
  const w = card.workload;
  const totalMix = w.by_work_type.reduce((sum, slice) => sum + slice.minutes, 0);
  return (
    <div className="grid gap-3">
      <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(auto-fit, minmax(9rem, 1fr))" }}>
        <Metric
          label="Response SLA"
          value={fmtPct(card.sla.response_attainment)}
          hint={`${card.sla.response_met}/${card.sla.response_total} met`}
        />
        <Metric
          label="Resolution SLA"
          value={fmtPct(card.sla.resolution_attainment)}
          hint={`${card.sla.resolution_met}/${card.sla.resolution_total} met`}
        />
        <Metric
          label="Breached"
          value={String(card.sla.breached)}
          hint={card.sla.breached > 0 ? "escalated tickets" : "none"}
        />
        <Metric label="First response" value={fmtSecs(card.response.median_seconds)} hint={`p95 ${fmtSecs(card.response.p95_seconds)}`} />
        <Metric label="Resolution" value={fmtSecs(card.resolution.median_seconds)} hint={`p95 ${fmtSecs(card.resolution.p95_seconds)}`} />
        <Metric
          label="Approved work"
          value={fmtHours(w.approved_minutes)}
          hint={`${w.approved_entries} entries · ${fmtPct(w.billable_ratio)} billable`}
        />
      </div>

      <div>
        <div className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Workload mix (approved minutes by work type)
        </div>
        {totalMix === 0 ? (
          <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            No approved time entries in this window.
          </p>
        ) : (
          <ul className="mt-1 grid gap-1">
            {w.by_work_type.map((slice) => (
              <li key={slice.work_type ?? "unclassified"} className="flex items-center gap-2 text-[12px]">
                <span className="w-32 shrink-0 truncate" style={{ color: "var(--m-text-secondary)" }}>
                  {slice.work_type ?? "Unclassified"}
                </span>
                <span
                  className="h-2 transition-opacity duration-100"
                  style={{
                    width: `${Math.max(2, Math.round((slice.minutes / totalMix) * 100))}%`,
                    minWidth: "2px",
                    background: "var(--m-accent, var(--m-text))",
                    opacity: 0.7,
                  }}
                  aria-hidden="true"
                />
                <span className="tabular-nums" style={{ color: "var(--m-text)" }}>
                  {fmtHours(slice.minutes)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div
        className="grid gap-1 border-t pt-3"
        style={{ borderColor: "var(--m-border)" }}
        data-testid="csat-section"
      >
        <h3 className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          CSAT
        </h3>
        {card.csat.available ? (
          <p className="text-[13px] tabular-nums" style={{ color: "var(--m-text)" }}>
            <strong>{card.csat.responses}</strong> response{card.csat.responses === 1 ? "" : "s"}
            {card.csat.avg_score !== null && card.csat.avg_score !== undefined
              ? ` · avg ${card.csat.avg_score.toFixed(2)} / 5`
              : ""}
            {card.csat.low_scores > 0 ? ` · ${card.csat.low_scores} low` : ""}
          </p>
        ) : (
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            Not yet available — no surveys have been answered yet (enable them under Settings → Surveys).
          </p>
        )}
      </div>
    </div>
  );
}

export default function ScorecardsPage() {
  const auth = useRequireAuth();
  const isManager = auth.status === "authed" && isManagerPermissions(auth.me?.permissions ?? []);

  const [days, setDays] = useState<number>(30);
  const from = utcDayOffset(days);
  const to = utcDayOffset(1);

  const [own, setOwn] = useState<Scorecard | null>(null);
  const [ownError, setOwnError] = useState<string | null>(null);

  const [team, setTeam] = useState<TeamScorecardsResponse | null>(null);
  const [teamError, setTeamError] = useState<string | null>(null);

  const [settings, setSettings] = useState<ScorecardSettingsResponse | null>(null);
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);

  const query = { query: { from, to } } as const;

  const refresh = useCallback(async () => {
    setOwnError(null);
    setTeamError(null);
    const ownRes = await api.api.GET("/api/v1/scorecards/me", query);
    if (ownRes.error !== undefined || !ownRes.data) {
      setOwn(null);
      setOwnError(friendlyMutationText(ownRes.error, "Your scorecard could not be loaded."));
    } else {
      setOwn(ownRes.data);
    }
    if (isManager) {
      const teamRes = await api.api.GET("/api/v1/scorecards", query);
      if (teamRes.error !== undefined || !teamRes.data) {
        setTeam(null);
        setTeamError(friendlyMutationText(teamRes.error, "Team scorecards could not be loaded."));
      } else {
        setTeam(teamRes.data);
      }
    } else {
      setTeam(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [from, to, isManager]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
    void api.api
      .GET("/api/v1/scorecards/me/settings")
      .then(({ data, error }) => {
        if (error !== undefined || !data) {
          setSettingsError(friendlyMutationText(error, "Your scorecard settings could not be loaded."));
          return;
        }
        setSettings(data);
      })
      .catch(() => setSettingsError("Your scorecard settings could not be loaded."));
  }, [auth.status, refresh]);

  const toggleGamification = async (optIn: boolean) => {
    setSettingsBusy(true);
    setSettingsError(null);
    const { data, error } = await api.api.PATCH("/api/v1/scorecards/me/settings", {
      body: { gamification_opt_in: optIn },
    });
    setSettingsBusy(false);
    if (error !== undefined || !data) {
      setSettingsError(friendlyMutationText(error, "Your scorecard settings could not be saved."));
      return;
    }
    setSettings(data);
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 max-w-6xl mx-auto p-4 md:p-6">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Scorecards
          </h1>
          <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Coaching metrics over approved work and SLA outcomes. Private by default — you see your
            own card; owners and admins see the team.
          </p>
        </div>
        <Select
          value={String(days)}
          onValueChange={(v) => setDays(Number(v))}
        >
          <SelectTrigger className="w-40" aria-label="Scorecard window">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {WINDOWS.map((w) => (
              <SelectItem key={w.days} value={String(w.days)}>
                {w.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </header>

      {ownError && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {ownError}
        </p>
      )}

      <section aria-label="Your scorecard">
        <Card className="p-4 md:p-5">
          <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
            <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              {own ? own.display_name : "Your scorecard"}
            </h2>
            {own ? (
              <Tag tone="neutral">
                {new Date(own.window.from).toISOString().slice(0, 10)} –{" "}
                {new Date(new Date(own.window.to).getTime() - 1).toISOString().slice(0, 10)}
              </Tag>
            ) : null}
          </div>
          {own === null && !ownError ? (
            <div className="p-6 flex justify-center">
              <Spinner />
            </div>
          ) : own ? (
            <ScorecardBody card={own} />
          ) : null}

          <div className="mt-4 pt-3" style={{ borderTop: "1px solid var(--m-border)" }}>
            <label className="flex items-start gap-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              <Checkbox
                checked={settings?.gamification_opt_in ?? false}
                disabled={settingsBusy || settings === null}
                onCheckedChange={(checked) => void toggleGamification(checked === true)}
              />
              <span>
                Opt in to gamification
                <FieldHint>
                  Off by default: your scorecard stays a private coaching surface. Opting in lets
                  streaks and friendly leaderboards include you.
                </FieldHint>
              </span>
            </label>
            {settingsError && (
              <p role="alert" className="mt-1 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {settingsError}
              </p>
            )}
          </div>
        </Card>
      </section>

      {isManager && (
        <section aria-label="Team scorecards">
          <h2 className="text-sm font-semibold mb-2" style={{ color: "var(--m-text)" }}>
            Team
          </h2>
          {teamError && (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {teamError}
            </p>
          )}
          {team === null && !teamError ? (
            <div className="p-6 flex justify-center">
              <Spinner />
            </div>
          ) : team && team.scorecards.length === 0 ? (
            <Card className="p-6">
              <EmptyState message="No active staff members to score yet." />
            </Card>
          ) : team ? (
            <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(20rem, 1fr))" }}>
              {team.scorecards.map((card) => (
                <Card key={card.user_id} className="p-4">
                  <div className="flex items-center justify-between gap-2 mb-2">
                    <span className="text-[13px] font-medium truncate" style={{ color: "var(--m-text)" }}>
                      {card.display_name}
                    </span>
                    <Tag tone={attainmentTone(card.sla.response_attainment)}>
                      resp {fmtPct(card.sla.response_attainment)}
                    </Tag>
                  </div>
                  <div className="grid grid-cols-2 gap-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                    <span>
                      Resolution SLA{" "}
                      <strong className="tabular-nums" style={{ color: "var(--m-text)" }}>
                        {fmtPct(card.sla.resolution_attainment)}
                      </strong>
                    </span>
                    <span className="whitespace-nowrap">
                      Breached{" "}
                      <strong className="tabular-nums" style={{ color: "var(--m-text)" }}>
                        {card.sla.breached}
                      </strong>
                    </span>
                    <span className="whitespace-nowrap">
                      Median response{" "}
                      <strong className="tabular-nums" style={{ color: "var(--m-text)" }}>
                        {fmtSecs(card.response.median_seconds)}
                      </strong>
                    </span>
                    <span className="whitespace-nowrap">
                      Approved{" "}
                      <strong className="tabular-nums" style={{ color: "var(--m-text)" }}>
                        {fmtHours(card.workload.approved_minutes)}
                      </strong>
                    </span>
                  </div>
                </Card>
              ))}
            </div>
          ) : null}
        </section>
      )}

      {isManager && <ClientsCsat from={from} to={to} />}
    </div>
  );
}

/** Per-client CSAT rollups (Wave 34 lane 2), manager-only. Types mirror the
 * pinned OpenAPI schemas; the page does not wait on SDK regeneration. */
interface ClientRollup {
  client_id: string;
  client_name: string;
  responses: number;
  avg_score: number | null;
  low_scores: number;
}

function ClientsCsat({ from, to }: { from: string; to: string }) {
  const [rollups, setRollups] = useState<ClientRollup[] | null>(null);
  const [available, setAvailable] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    api.api
      .GET("/api/v1/scorecards/clients", { params: { query: { from, to } } })
      .then(({ data, error: err }) => {
        if (cancelled) return;
        if (err !== undefined || !data) {
          setError("Client CSAT rollups could not be loaded.");
          setRollups(null);
          return;
        }
        setAvailable(data.available);
        setRollups((data as unknown as { rollups: ClientRollup[] }).rollups);
      })
      .catch(() => {
        if (!cancelled) setError("Client CSAT rollups could not be loaded.");
      });
    return () => {
      cancelled = true;
    };
  }, [from, to]);

  return (
    <section aria-label="Client CSAT">
      <h2 className="text-sm font-semibold mb-2" style={{ color: "var(--m-text)" }}>
        Client satisfaction
      </h2>
      {error && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      )}
      {!error && rollups === null ? (
        <div className="p-6 flex justify-center">
          <Spinner />
        </div>
      ) : rollups !== null && !available ? (
        <Card className="p-6">
          <EmptyState message="No survey responses yet — enable surveys under Settings → Surveys." />
        </Card>
      ) : rollups !== null && rollups.length === 0 ? (
        <Card className="p-6">
          <EmptyState message="No client responses in this window." />
        </Card>
      ) : rollups ? (
        <Card className="p-0 overflow-hidden">
          <Table>
            <thead>
              <tr>
                <Th>Client</Th>
                <Th style={{ textAlign: "right" }}>Responses</Th>
                <Th style={{ textAlign: "right" }}>Avg score</Th>
                <Th style={{ textAlign: "right" }}>Low (≤2)</Th>
              </tr>
            </thead>
            <tbody>
              {rollups.map((r) => (
                <Tr key={r.client_id}>
                  <Td>{r.client_name}</Td>
                  <Td style={{ textAlign: "right" }}>{r.responses}</Td>
                  <Td style={{ textAlign: "right" }}>{r.avg_score !== null ? r.avg_score.toFixed(2) : "—"}</Td>
                  <Td style={{ textAlign: "right" }}>{r.low_scores}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        </Card>
      ) : null}
    </section>
  );
}
