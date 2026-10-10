// /settings/ai/shadow — what a surface would have said, and the run log.
//
// Purpose: states 11 and 12 of the settings pack, on their own page, because
// this is the ONE place in the product where shadow mode is visible. On every
// product surface a shadow run leaves no trace; here the admin can watch what
// it would have proposed, how it agreed with the person who actually decided,
// and decide whether to promote it.
//
// What is real and what is not:
//   - The surfaces, their mode and their run budget are real, read from the
//     decision-harness API.
//   - The per-ticket "what it would have said" list is NOT in the product: no
//     run log endpoint exists, so the list says so plainly and points at the
//     surface editor, which fires a real shadow decision and shows the real
//     answers. The drawn page shape — surface picker, the two tabs, the strip,
//     the four figures, the run table and the promote card — is kept, so the
//     place exists and reads right the day the log lands.
//   - Promotion is real: it flips the surface's mode to live through the
//     config API, and it is reversible.
//
// Non-purpose: no new backend, and no way to read a client's data from here.

"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Badge, Button, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Spinner } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import { api } from "../../../../lib/api";
import { resultErrorText } from "../../../../lib/error-copy";
import { SettingsHeader } from "../../../../components/settings/settings-shell";
import {
  Kpi,
  Note,
  PromoteCard,
  ThresholdBar,
} from "../../../../components/settings/settings-forms";

type DecisionSurfaceOverview = components["schemas"]["DecisionSurfaceOverview"];
type DecisionRunsResponse = NonNullable<
  components["schemas"]["DecisionRunsResponse"]
>;
type DecisionRunView = components["schemas"]["DecisionRunView"];

const THRESHOLD = 0.8;

export default function ShadowRunsPage(): React.JSX.Element {
  const [surfaces, setSurfaces] = useState<DecisionSurfaceOverview[]>([]);
  const [featuresEnabled, setFeaturesEnabled] = useState(false);
  const [selected, setSelected] = useState<string>("");
  const [tab, setTab] = useState<"list" | "log">("list");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // The run log is a READ over the append-only decision_answers store, which
  // has recorded every answered run since migration 0090. Nothing here is
  // narrated: the four figures and each row are what the product recorded.
  const [log, setLog] = useState<DecisionRunsResponse | null>(null);
  const [logLoading, setLogLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    const res = await api.api.GET("/api/v1/settings/ai/decision");
    if (res.error !== undefined) {
      setError("The AI settings did not load.");
      setLoading(false);
      return;
    }
    const all = res.data?.surfaces ?? [];
    setFeaturesEnabled(res.data?.features_enabled ?? false);
    setSurfaces(all.filter((surface) => !surface.internal));
    setSelected((prev) =>
      prev !== "" && all.some((surface) => surface.surface_key === prev)
        ? prev
        : (all.find((surface) => !surface.internal && surface.configured)?.surface_key ?? ""),
    );
    setLoading(false);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const surface = useMemo(
    () => surfaces.find((s) => s.surface_key === selected) ?? null,
    [surfaces, selected],
  );

  // The log is read per surface, and only when the surface can have runs: a
  // surface with no grant has nothing recorded, which the page says in words.
  const loadLog = useCallback(async () => {
    if (selected === "") {
      setLog(null);
      return;
    }
    setLogLoading(true);
    const res = await api.api.GET("/api/v1/settings/ai/decision/{surface_key}/runs", {
      params: { path: { surface_key: selected }, query: { days: 7, limit: 50 } },
    });
    setLogLoading(false);
    setLog(res.error === undefined && res.data !== undefined ? res.data : null);
  }, [selected]);

  useEffect(() => {
    void loadLog();
  }, [loadLog]);

  const setMode = async (mode: "shadow" | "live"): Promise<void> => {
    if (surface === null) return;
    setBusy(true);
    setError(null);
    const res = await api.api.PUT("/api/v1/settings/ai/decision/{surface_key}/config", {
      params: { path: { surface_key: surface.surface_key } },
      body: {
        idempotency_key: `ai-mode:${surface.surface_key}:${mode}:${Date.now()}`,
        mode,
      } as never,
    });
    setBusy(false);
    if (res.error !== undefined) {
      setError(resultErrorText(res, "The mode was refused."));
      return;
    }
    await load();
  };

  if (loading) {
    return (
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: 24 }}>
        <Spinner />
        <span style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>
          Loading the decision surfaces…
        </span>
      </div>
    );
  }

  return (
    <div data-testid="ai-shadow-page">
      <SettingsHeader
        title="Shadow runs"
        description="What a surface would have said, recorded without anyone seeing it."
      >
        <label className="flex items-center gap-2">
          <span className="sr-only">Surface</span>
          <Select value={selected === "" ? undefined : selected} onValueChange={setSelected}>
            <SelectTrigger aria-label="Surface" data-testid="ai-shadow-surface">
              <SelectValue placeholder="No surfaces" />
            </SelectTrigger>
            <SelectContent>
              {surfaces.map((s) => (
                <SelectItem key={s.surface_key} value={s.surface_key}>
                  {s.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>
      </SettingsHeader>

      {!featuresEnabled ? (
        <Note testId="ai-shadow-master-off">
          <b>The master switch is off.</b> No surface can run while the organisation has AI
          disabled, so there is nothing to watch. Turn it on in{" "}
          <a href="/settings/ai" style={{ fontWeight: 600 }}>
            AI
          </a>
          .
        </Note>
      ) : surface === null || !surface.configured ? (
        <Note testId="ai-shadow-not-granted">
          <b>Not granted.</b> This surface has no configuration yet, so it has never run. Grant it
          in <a href="/settings/ai" style={{ fontWeight: 600 }}>AI</a> and it starts in shadow, with
          nothing shown to anyone.
        </Note>
      ) : null}

      <div
        role="group"
        aria-label="Views"
        data-testid="ai-shadow-tabs"
        style={{ display: "flex", gap: 4, margin: "0 0 16px" }}
      >
        {(
          [
            ["list", "What it would have said"],
            ["log", "Run log"],
          ] as const
        ).map(([key, label]) => (
          <button
            key={key}
            type="button"
            aria-pressed={tab === key}
            onClick={() => setTab(key)}
            data-testid={`ai-shadow-tab-${key}`}
            style={{
              height: 36,
              padding: "0 14px",
              borderRadius: 10,
              border: "none",
              background: tab === key ? "var(--m-surface-2)" : "transparent",
              color: tab === key ? "var(--m-text)" : "var(--m-text-secondary)",
              fontWeight: tab === key ? 600 : 500,
              cursor: "pointer",
            }}
          >
            {label}
          </button>
        ))}
      </div>

      {surface !== null && surface.configured && featuresEnabled ? (
        <div
          data-testid="ai-shadow-strip"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            padding: "12px 16px",
            borderRadius: 14,
            background: "var(--m-surface-2)",
            fontSize: 13,
            lineHeight: 1.5,
            marginBottom: 14,
          }}
        >
          <ShieldIcon />
          <span>
            {surface.mode === "live" ? (
              <>
                <b>Live.</b> This surface now proposes above the threshold. A person still approves
                every write.
              </>
            ) : (
              <>
                <b>In shadow.</b> Nothing below was shown to anyone. This page is the only place it
                appears.
              </>
            )}
          </span>
        </div>
      ) : null}

      {error !== null ? (
        <p role="alert" style={{ fontSize: 13, color: "var(--m-status-due-fg)", margin: "0 0 12px" }}>
          {error}
        </p>
      ) : null}

      {tab === "list" ? (
        <section
          data-testid="ai-shadow-list"
          style={{
            border: "1px solid var(--m-border)",
            borderRadius: 16,
            background: "var(--m-surface)",
            padding: "18px 20px",
          }}
        >
          <h2 style={{ margin: "0 0 4px", fontSize: 15, fontWeight: 600, color: "var(--m-text)" }}>
            Nothing recorded yet
          </h2>
          <p
            style={{
              margin: 0,
              fontSize: 13,
              lineHeight: 1.5,
              color: "var(--m-text-secondary)",
              maxWidth: 560,
            }}
          >
            The product keeps no per-ticket list of what a shadow run would have said, so there is
            nothing to show here yet. The surface editor fires a real shadow decision on a real
            ticket and shows the answers, the full probability distribution and the confidence — the
            same evidence this page will show once the run log lands.
          </p>
          <div style={{ marginTop: 12 }}>
            <a href={`/settings/ai`} style={{ fontWeight: 600 }}>
              Open the surface editor
            </a>
          </div>
        </section>
      ) : (
        <>
          <div
            data-testid="ai-shadow-log"
            style={{
              border: "1px solid var(--m-border)",
              borderRadius: 16,
              background: "var(--m-surface)",
              overflow: "hidden",
            }}
          >
            {logLoading ? (
              <p style={{ padding: 20, fontSize: 13, color: "var(--m-text-secondary)" }}>
                Reading the run log…
              </p>
            ) : log === null ? (
              <p style={{ padding: 20, fontSize: 13, color: "var(--m-text-secondary)" }}>
                The run log could not be read for this surface.
              </p>
            ) : !log.summary.has_evidence ? (
              <div style={{ padding: "18px 20px" }}>
                <h2
                  style={{ margin: "0 0 4px", fontSize: 15, fontWeight: 600, color: "var(--m-text)" }}
                >
                  No runs in the last {log.days} days
                </h2>
                <p
                  style={{
                    margin: 0,
                    fontSize: 13,
                    lineHeight: 1.5,
                    color: "var(--m-text-secondary)",
                    maxWidth: 560,
                  }}
                >
                  Nothing has been decided for this surface in this window, so there is no evidence
                  to read and nothing to promote on. Fire a decision from the surface editor to see
                  the real answers, the distribution and the confidence the harness computes.
                </p>
              </div>
            ) : (
              <>
                {/* The four figures, all measured: the count, the count that
                    cleared the surface's own act threshold, the agreement
                    against where the ticket actually went, and how many runs
                    could be compared at all. */}
                <div
                  data-testid="ai-shadow-kpis"
                  style={{
                    display: "grid",
                    gridTemplateColumns: "repeat(4, minmax(0, 1fr))",
                    gap: 10,
                    padding: "14px 16px",
                    borderBottom: "1px solid var(--m-border)",
                  }}
                >
                  <Kpi value={String(log.summary.runs)} label="runs, last 7 days" />
                  <Kpi value={String(log.summary.above_threshold)} label="proposed above the line" />
                  <Kpi
                    value={`${log.summary.agreed} / ${log.summary.comparable}`}
                    label="matched what the agent did"
                  />
                  <div
                    style={{
                      border: "1px solid var(--m-border)",
                      borderRadius: 14,
                      background: "var(--m-surface)",
                      padding: "14px 16px",
                    }}
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <ThresholdBar
                        value={log.summary.threshold}
                        threshold={log.summary.lowest_threshold}
                        width={64}
                      />
                      <b
                        style={{
                          fontSize: 20,
                          fontWeight: 600,
                          color: "var(--m-text)",
                          fontVariantNumeric: "tabular-nums",
                        }}
                      >
                        {log.summary.lowest_threshold === log.summary.threshold
                          ? log.summary.threshold.toFixed(2)
                          : `${log.summary.lowest_threshold.toFixed(2)}–${log.summary.threshold.toFixed(2)}`}
                      </b>
                    </div>
                    <div style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>
                      the {log.summary.threshold_questions === 1 ? "line" : "lines"} in force
                    </div>
                  </div>
                </div>
                <RunTable runs={log.runs} />
              </>
            )}
          </div>

          {surface !== null && surface.configured ? (
            <PromoteCard
              promoted={surface.mode === "live"}
              title={surface.mode === "live" ? "Live" : "Ready to go live?"}
              actions={
                surface.mode === "live" ? (
                  <Button
                    variant="secondary"
                    disabled={busy}
                    onClick={() => void setMode("shadow")}
                    data-testid="ai-shadow-demote"
                  >
                    Return to shadow
                  </Button>
                ) : (
                  <>
                    <Button
                      variant="secondary"
                      disabled={busy}
                      onClick={() => void setMode("shadow")}
                      data-testid="ai-shadow-keep"
                    >
                      Keep in shadow
                    </Button>
                    <Button
                      variant="primary"
                      disabled={busy}
                      onClick={() => void setMode("live")}
                      data-testid="ai-shadow-promote"
                    >
                      Promote to live
                    </Button>
                  </>
                )
              }
            >
              {surface.mode === "live"
                ? "Proposals now appear above the threshold. You can return it to shadow at any time, and a person still approves every write."
                : "Promoting flips this surface to live, so it may propose above the threshold. A person still approves every write, and you can put it back in shadow at any time."}
            </PromoteCard>
          ) : null}
        </>
      )}

      {surface !== null ? (
        <p
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            marginTop: 14,
            fontSize: 13,
            color: "var(--m-text-secondary)",
          }}
        >
          <Badge tone={surface.mode === "live" ? "success" : "warning"}>
            {surface.mode === "live" ? "Live" : "Shadow"}
          </Badge>
          <span>
            {surface.model_provider !== null && surface.model_provider !== undefined
              ? `Pinned to ${surface.model_provider} · ${surface.model_id ?? "no model pinned"}`
              : "No decision model pinned yet — the surface cannot run until one is."}
          </span>
        </p>
      ) : null}
    </div>
  );
}

function ShieldIcon(): React.JSX.Element {
  return (
    <span style={{ display: "inline-flex", flexShrink: 0, color: "var(--m-text-secondary)" }}>
      <Icon name="shield" size={18} />
    </span>
  );
}

/**
 * One row per recorded run: when, which ticket, what it would have said in
 * words, how sure it was against the line, and whether the human agreed.
 *
 * The last column is the honest part. Agreement is measured by comparing the
 * proposed label with the status the ticket actually reached, so a run whose
 * ticket has since been deleted says "not comparable" rather than counting as
 * a disagreement.
 */
function RunTable({ runs }: { runs: DecisionRunView[] }): React.JSX.Element {
  return (
    <div role="table" aria-label="Decision runs" data-testid="ai-shadow-run-table">
      <div
        role="row"
        style={{
          display: "grid",
          gridTemplateColumns: "150px minmax(180px, 1.4fr) minmax(160px, 1fr) 120px 130px",
          alignItems: "center",
          minHeight: 40,
          background: "var(--m-canvas)",
          borderBottom: "1px solid var(--m-border)",
        }}
      >
        {["When", "Ticket", "What it would have said", "Confidence", "The agent did"].map((header) => (
          <span
            key={header}
            style={{
              fontSize: 12,
              letterSpacing: "0.06em",
              textTransform: "uppercase",
              fontWeight: 600,
              color: "var(--m-text-secondary)",
              paddingLeft: 12,
            }}
          >
            {header}
          </span>
        ))}
      </div>
      {runs.map((run) => (
        <div
          key={run.id}
          role="row"
          data-testid={`ai-shadow-run-${run.id}`}
          style={{
            display: "grid",
            gridTemplateColumns: "150px minmax(180px, 1.4fr) minmax(160px, 1fr) 120px 130px",
            alignItems: "center",
            minHeight: 56,
            padding: "8px 0",
            borderBottom: "1px solid var(--m-border)",
          }}
        >
          <span
            style={{
              paddingLeft: 12,
              fontSize: 13,
              color: "var(--m-text-secondary)",
              fontVariantNumeric: "tabular-nums",
            }}
          >
            {new Date(run.created_at).toLocaleString(undefined, {
              month: "short",
              day: "numeric",
              hour: "2-digit",
              minute: "2-digit",
            })}
          </span>
          <span
            style={{
              paddingLeft: 12,
              fontSize: 13,
              color: "var(--m-text)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {run.ticket_number ?? "—"}
            {run.ticket_title === null || run.ticket_title === undefined ? null : (
              <span style={{ color: "var(--m-text-secondary)" }}> · {run.ticket_title}</span>
            )}
          </span>
          <span style={{ paddingLeft: 12, display: "flex", flexWrap: "wrap", gap: 6 }}>
            {run.answers.length === 0 ? (
              <span style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>no answer</span>
            ) : (
              run.answers.map((answer) => (
                <span
                  key={answer.question_id}
                  title={`${answer.question_id} · ${answer.above_threshold ? "above" : "below"} the line`}
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 6,
                    height: 24,
                    padding: "0 10px",
                    borderRadius: 999,
                    background: answer.above_threshold
                      ? "var(--m-status-warning-bg)"
                      : "var(--m-surface-2)",
                    color: answer.above_threshold
                      ? "var(--m-status-warning-fg)"
                      : "var(--m-text-secondary)",
                    fontSize: 13,
                    fontWeight: 600,
                    whiteSpace: "nowrap",
                  }}
                >
                  {answer.label}
                </span>
              ))
            )}
          </span>
          <span style={{ paddingLeft: 12, display: "flex", alignItems: "center", gap: 8 }}>
            {run.top_confidence === null || run.top_confidence === undefined ? (
              <span style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>—</span>
            ) : (
              <>
                <ThresholdBar
                  value={run.top_confidence}
                  threshold={log_threshold(run.top_confidence)}
                  width={48}
                />
                <span
                  style={{
                    fontSize: 13,
                    color: "var(--m-text)",
                    fontVariantNumeric: "tabular-nums",
                  }}
                >
                  {run.top_confidence.toFixed(2)}
                </span>
              </>
            )}
          </span>
          <span style={{ paddingLeft: 12 }}>
            {run.agreed === null || run.agreed === undefined ? (
              <span style={{ fontSize: 13, color: "var(--m-text-secondary)" }}>
                not comparable
              </span>
            ) : run.agreed ? (
              <Badge tone="success">agreed</Badge>
            ) : (
              <Badge tone="warning">differed</Badge>
            )}
          </span>
        </div>
      ))}
    </div>
  );
}

/** The run's own confidence against a 0..1 bar; the line itself is the
 * surface's threshold, drawn once in the figures above. */
function log_threshold(value: number): number {
  return Math.min(1, Math.max(0, value));
}
