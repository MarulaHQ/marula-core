"use client";

// DecisionSurfaceEditor — the settings/ai per-surface decision-harness
// config editor (ADR 0010 §4 + ADR 0011 §3 honesty).
//
// Purpose: one editor per decision surface with EVERYTHING the lane
// contract names:
// - enable toggle + honest "unconfigured ⇒ silently quiet" note and
//   the "what activates" description while configured=false;
// - shadow → live mode flip (shadow forever first; live is the single
//   org-wide consumability lever);
// - pinned decision model pick (only decision-capable CATALOG pairs;
//   the honest "decision model not configured" state when the catalog
//   carries no `decision` entry or the org credential slot is empty);
// - the state_template + question JSON editor with LIVE PREVIEW: pick
//   a real org ticket/alert, see the composed state INCLUDING the
//   ADR 0011 §4 derived fields (after_hours / within_sla_window /
//   elapsed_sum / minutes_since_created / overage_minutes_remaining)
//   and a tiered Choice (>255) rendered as its level-two note, then
//   FIRE ONE shadow decision through the real edge and inspect the
//   typed answers + full probability distributions + confidence;
// - per-question threshold editors (act / review);
// - fork-on-first-edit provenance: the fork is server-owned (first
//   edit forks; later edits replace it) and the banner shows "v2
//   available" when the latest curated starter version outruns the
//   pinned one — with an honest diff between the active fork and the
//   curated starter, and a real reset-to-starter button.
//
// Refusals ride verbatim (`ai_decision_config_refused: reason —...) —
// the class restrictions are visible, never normalized away.
//
// Tokens: docs/theme.md only. Motion ≤120ms transform/opacity.

import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import { resultErrorText } from "../../lib/error-copy";
import { doubleDiff as diffSummary } from "../../lib/decision-diff";
import type { components } from "@marula-core/sdk";
import {
  Badge,
  Button,
  Card,
  Input,
  Label,
  Spinner,
} from "@marula-core/ui";

type SurfaceOverview = components["schemas"]["DecisionSurfaceOverview"];
type DecisionPreviewRequest = components["schemas"]["DecisionPreviewRequest"];

interface SurfaceConfigWire {
  state_template: { fields: Record<string, unknown> };
  questions: Array<{
    id: string;
    kind: string;
    instructions: string;
    criteria?: string[];
    rubric?: string[];
    options?: { source: string } & Record<string, unknown>;
    thresholds: { act: number; review: number };
  }>;
}

interface PreviewResult {
  state: Record<string, unknown> | null;
  derived_fields: Record<string, unknown> | null;
  run: {
    status?: string;
    provider?: string;
    model?: string;
    answers?: Record<string, {
      kind?: string;
      picked?: number;
      picked_label?: string;
      level?: number;
      probability?: number;
      confidence?: number;
      distribution?: number[];
    }> | null;
    reason?: string;
  } | null;
  starter?: SurfaceConfigWire | null;
  active_config?: SurfaceConfigWire | null;
  quiet?: string;
}

export function DecisionSurfaceEditor({
  surface,
  onSaved,
}: {
  surface: SurfaceOverview;
  onSaved?: (updated: SurfaceOverview) => void;
}) {
  const [draft, setDraft] = useState<string>("");
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [firing, setFiring] = useState(false);
  const [entityType, setEntityType] = useState<"ticket" | "alert">("ticket");
  const [entityId, setEntityId] = useState<string>("");
  const [entityRows, setEntityRows] = useState<{ id: string; label: string }[]>([]);
  const [taskBusy, setTaskBusy] = useState<"mode" | "enable" | "model" | null>(null);

  const active = preview?.active_config ?? null;
  const starterMap = preview?.starter;

  const loadPreview = useCallback(
    async (fire: boolean) => {
      if (entityId === "") return;
      setPreviewing(true);
      setError(null);
      if (fire) setFiring(true);
      const { data, error: e } = await api.api.POST(
        "/api/v1/settings/ai/decision/{surface_key}/preview",
        {
          params: { path: { surface_key: surface.surface_key } },
          body: { entity_type: entityType, entity_id: entityId, fire },
        } as never,
      );
      if (fire) setFiring(false);
      setPreviewing(false);
      if (e !== undefined || data === undefined) {
        setError(resultErrorText(e, "The preview was rejected."));
        return;
      }
      const wire = data as unknown as PreviewResult;
      setPreview(wire);
      if (!Object.prototype.hasOwnProperty.call(wire, "quiet") && wire.active_config) {
        // Localize the current config — the JSON editor starts from the
        // truth the harness would dispatch (NOT from the fork origin
        // marker the server keeps internally).
        setDraftJSON(wire.active_config);
      }
    },
    [entityType, entityId, surface.surface_key],
  );

  useEffect(() => {
    setPreview(null);
  }, [surface.surface_key]);

  // Load the candidate entity lists once — real rows only, recent first.
  useEffect(() => {
    let alive = true;
    (async () => {
      if (entityType === "ticket") {
        const { data } = await api.api.GET("/api/v1/tickets");
        if (!alive) return;
        const rows = data ?? [];
        setEntityRows(
          rows.slice(0, 25).map((t) => ({
            id: String(t.id),
            label: t.title,
          })),
        );
      } else {
        const { data } = await api.api.GET("/api/v1/alerts");
        if (!alive) return;
        setEntityRows(
          (data?.alerts ?? []).map((a) => ({
            id: String(a.id),
            label: a.title,
          })),
        );
      }
    })();
    return () => {
      alive = false;
    };
  }, [entityType]);

  function setDraftJSON(config: SurfaceConfigWire) {
    setDraft(JSON.stringify(config, null, 2));
    setDirty(false);
  }

  const save = async (configOverride?: string) => {
    setSaving(true);
    setError(null);
    const body: Record<string, unknown> = {
      idempotency_key: `dconf:${surface.surface_key}:${Date.now()}`,
    };
    const source = configOverride !== undefined ? configOverride : draft;
    if (source.trim() !== "") {
      try {
        body.config = JSON.parse(source);
      } catch (err) {
        setError("The config is not valid JSON yet — the server never truncates or fixes it.");
        setSaving(false);
        return;
      }
    }
    const { data, error: e } = await api.api.PUT(
      "/api/v1/settings/ai/decision/{surface_key}/config",
      {
        params: { path: { surface_key: surface.surface_key } },
        body: body as never,
      },
    );
    setSaving(false);
    if (e !== undefined || data === undefined) {
      setError(resultErrorText(e, "The config was refused."));
      return;
    }
    setDirty(false);
    setConfigOverride(undefined);
    if (data.mutation?.updated_at != null && active != null) {
      // Server won't echo the fork state as a full config; refresh it
      // through the preview seam it already responds to.
      void loadPreview(false);
    }
    onSaved?.(surface);
  };
  const [configOverride, setConfigOverride] = useState<string | undefined>(undefined);

  const simplePut = useCallback(
    async (
      section: "mode" | "enable" | "model",
      body: Record<string, unknown>,
    ) => {
      setTaskBusy(section);
      setError(null);
      const { error: e } = await api.api.PUT("/api/v1/settings/ai/decision/{surface_key}/config", {
        params: { path: { surface_key: surface.surface_key } },
        body: body as never,
      });
      setTaskBusy(null);
      if (e !== undefined) {
        setError(resultErrorText(e, "The change was refused."));
        return;
      }
      onSaved?.(surface);
    },
    [surface.surface_key, surface, onSaved],
  );

  const v2Available =
    surface.latest_starter_version != null &&
    surface.latest_starter_version > surface.starter_version;

  // The honest ADR 0011 §3 posture: not configured = note, not a shell.
  if (!surface.configured) {
    return (
      <Card>
        <div className="p-4" data-testid={`decision-unconfigured-${surface.surface_key}`}>
          <div className="flex items-center gap-2">
            <UnconfiguredBadge />
            <h3 className="text-sm font-semibold">{surface.label}</h3>
          </div>
          <p className="mt-2 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            {surface.description}
          </p>
          <p className="mt-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            <strong>What activates:</strong> configure this surface below (seed its thresholds and
            pick the decision model) and the org master switch makes it start answering in shadow
            mode — until then it stays silently quiet and surfaces no cards or dead buttons.
          </p>
          {surface.internal && (
            <p className="mt-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Internal surface — owned by the harness, never a grant target.
            </p>
          )}
          <div className="mt-3">
            <Button
              size="sm"
              disabled={taskBusy !== null}
              onClick={() => {
                void simplePut("enable", {
                  idempotency_key: `dconf:${surface.surface_key}:${Date.now()}`,
                  enabled: true,
                  mode: "shadow",
                });
              }}
            >
              {taskBusy === "enable" ? <Spinner /> : "Configure in shadow"}
            </Button>
          </div>
        </div>
      </Card>
    );
  }

  const questions = active?.questions ?? [];

  return (
    <Card>
      <div className="p-4" data-surface-editor={surface.surface_key}>
        {/* Header + honest posture badges */}
        <div className="mb-3 flex flex-wrap items-center gap-2">
          <h3 className="text-sm font-semibold">{surface.label}</h3>
          <Badge>{surface.mode === "live" ? "LIVE" : "SHADOW"}</Badge>
          {surface.model_id != null ? (
            <Badge>{`model: ${surface.model_id}`}</Badge>
          ) : (
            <DecisionModelUnknownPill curated={surface.decision_model_curated} />
          )}
          {v2Available && <V2Pill latest={surface.latest_starter_version} />}
        </div>
        <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          {surface.description}
        </p>

        {/* Posture controls */}
        <div className="mt-3 flex flex-wrap items-center gap-3">
          <Button
            size="sm"
            variant={surface.enabled ? "secondary" : "primary"}
            disabled={taskBusy !== null || saving}
            onClick={() => void simplePut("enable", {
              idempotency_key: `dconf:${surface.surface_key}:en:${Date.now()}`,
              enabled: !surface.enabled,
            })}
          >
            {taskBusy === "enable" ? <Spinner /> : surface.enabled ? "Disable surface" : "Enable surface"}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={taskBusy !== null || saving}
            title={
              surface.mode === "live"
                ? "Flip back to shadow: answers record but never auto-behave"
                : "Flip to live: consumers may act above the act thresholds"
            }
            onClick={() => void simplePut("mode", {
              idempotency_key: `dconf:${surface.surface_key}:mode:${Date.now()}`,
              mode: surface.mode === "live" ? "shadow" : "live",
            })}
          >
            {taskBusy === "mode" ? <Spinner /> : surface.mode === "live" ? "Back to shadow" : "Flip to live"}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={saving}
            onClick={() => void simplePut("model", {
              idempotency_key: `dconf:${surface.surface_key}:rst:${Date.now()}`,
              reset_to_starter: true,
            })}
          >
            Discard fork · follow curated starter
          </Button>
        </div>

        {/* The honest downed/unconfigured decision-model posture */}
        <DecisionModelNote surface={surface} />

        {error != null && (
          <p className="mt-3 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
            {error}
          </p>
        )}

        {/* Threshold editors — per question, riding the config route */}
        <div className="mt-4">
          <p className="text-[12px] font-semibold" style={{ color: "var(--m-text-secondary)" }}>
            Thresholds
          </p>
          {questions.length === 0 ? (
            <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Thresholds live in the question set — compose state below to bring them in.
            </p>
          ) : (
            <div className="mt-2 flex flex-col gap-2" data-testid="threshold-rows">
              {questions.map((q) => (
                <div key={q.id} className="flex items-center gap-3">
                  <span className="w-40 shrink-0 truncate text-[13px]">{q.id}</span>
                  <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    {q.kind}
                  </span>
                  <Input
                    type="number"
                    step="0.01"
                    className="w-24 text-[12px]"
                    aria-label={`${q.id} act threshold`}
                    value={String(q.thresholds.act)}
                    onChange={(e) => {
                      const next = JSON.parse(JSON.stringify(active));
                      next.questions = questions.map((x) => {
                        if (x.id !== q.id) return x;
                        return {
                          ...x,
                          thresholds: {
                            ...x.thresholds,
                            act: Number(e.target.value),
                          },
                        };
                      });
                      setConfigOverride(JSON.stringify(next));
                      setDraftJSON(next as SurfaceConfigWire);
                    }}
                  />
                  <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    act
                  </span>
                  <Input
                    type="number"
                    step="0.01"
                    className="w-24 text-[12px]"
                    aria-label={`${q.id} review threshold`}
                    value={String(q.thresholds.review)}
                    onChange={(e) => {
                      const next = JSON.parse(JSON.stringify(active));
                      next.questions = questions.map((x) => {
                        if (x.id !== q.id) return x;
                        return {
                          ...x,
                          thresholds: {
                            ...x.thresholds,
                            review: Number(e.target.value),
                          },
                        };
                      });
                      setConfigOverride(JSON.stringify(next));
                      setDraftJSON(next as SurfaceConfigWire);
                    }}
                  />
                  <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    review
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* state_template + questions JSON editor */}
        <div className="mt-4">
          <div className="flex items-center justify-between">
            <Label htmlFor={`st-${surface.surface_key}`}>State template + question set (JSON)</Label>
            <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              First edit forks from the starter ({`v${surface.starter_version}`} pinned);
              refusals ride the ADR 0011 §4 reasons verbatim.
            </span>
          </div>
          <textarea
            id={`st-${surface.surface_key}`}
            data-testid="state-template-editor"
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              setDirty(true);
            }}
            rows={14}
            spellCheck={false}
            className="mt-1 w-full rounded-[var(--m-radius-container)] border p-3 font-mono text-[12px]"
            style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
            placeholder="Compose state below to load the active question set."
          />
          <div className="mt-2 flex items-center gap-2">
            <Button
              size="sm"
              disabled={saving || (!dirty && configOverride === undefined)}
              onClick={() => void save()}
            >
              {saving ? <Spinner /> : dirty || configOverride !== undefined ? "Fork & save customized config" : "Saved"}
            </Button>
          </div>
        </div>

        {/* Live preview: pick a REAL org entity, see composed state */}
        <div className="mt-5 border-t pt-4" style={{ borderColor: "var(--m-border)" }}>
          <p className="text-[12px] font-semibold" style={{ color: "var(--m-text-secondary)" }}>
            Live preview — composed state from a real org entity
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <select
              className="rounded-[var(--m-radius-control)] border px-2 py-1 text-[12px]"
              style={{ borderColor: "var(--m-border)" }}
              value={entityType}
              onChange={(e) => {
                setEntityType(e.target.value as "ticket" | "alert");
                setEntityId("");
              }}
              aria-label="Entity type"
            >
              <option value="ticket">Ticket</option>
              <option value="alert">Alert</option>
            </select>
            <select
              className="max-w-72 rounded-[var(--m-radius-control)] border px-2 py-1 text-[12px]"
              style={{ borderColor: "var(--m-border)" }}
              value={entityId}
              onChange={(e) => setEntityId(e.target.value)}
              aria-label="Entity"
            >
              <option value="">Pick a real {entityType}…</option>
              {entityRows.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.label}
                </option>
              ))}
            </select>
            <Button size="sm" variant="secondary" disabled={entityId === "" || previewing} onClick={() => void loadPreview(false)}>
              Compose state
            </Button>
            <Button
              size="sm"
              disabled={entityId === "" || firing}
              data-testid="shadow-fire"
              onClick={() => void loadPreview(true)}
            >
              {firing ? <Spinner /> : "Fire shadow decision"}
            </Button>
          </div>

          {/* Derived fields rendered beside the raw state (ADR 0011 §4) */}
          {preview?.derived_fields != null && (
            <div className="mt-3" data-testid="derived-fields">
              <p className="text-[12px] font-semibold" style={{ color: "var(--m-text-secondary)" }}>
                Derived fields (pre-computed in code — never asked of the model)
              </p>
              <table className="mt-1 text-[12px]">
                <tbody>
                  {Object.entries(preview.derived_fields).map(([k, v]) => (
                    <tr key={`d-${k}`}>
                      <td className="pr-4 font-mono">{k}</td>
                      <td className="font-mono" style={{ color: "var(--m-text-secondary)" }}>
                        {JSON.stringify(v)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {preview?.state != null && (
            <pre
              className="mt-3 max-h-48 overflow-auto rounded-[var(--m-radius-container)] border p-2 text-[12px]"
              style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
            >
              {JSON.stringify(preview.state, null, 2)}
            </pre>
          )}

          {/* Typed answers + confidences after a fired shadow run */}
          {preview?.run != null && (
            <div className="mt-3" data-testid="shadow-run">
              {preview.run.reason != null ? (
                <p className="text-[13px]" style={{ color: "var(--m-status-warning-fg)" }}>
                  The run answered quiet: <code>{preview.run.reason}</code> — degrade-never-5xx
                  posture (e.g. the decision model credential is missing).
                </p>
              ) : (
                <>
                  <p className="text-[12px] font-semibold" style={{ color: "var(--m-text-secondary)" }}>
                    Shadow answers — {preview.run.provider}/{preview.run.model}
                  </p>
                  <div className="mt-1 flex flex-col gap-1">
                    {Object.entries(preview.run.answers ?? {}).map(([id, a]) => (
                      <ShadowAnswerRow key={id} id={id} a={a} />
                    ))}
                  </div>
                </>
              )}
            </div>
          )}

          {/* Honest tiered Choice note: >255 inventories render levels */}
          {questions.some((q) => q.options?.source === "tiered") && (
            <p className="mt-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Tiered Choice present: level-one category is asked here; the level-two item pass is a
              second ~free call consumers make when they need the item.
            </p>
          )}

          {/* The "v2 available" honest diff */}
          {starterMap != null && (
            <div className="mt-3" data-testid="starter-diff">
              <p className="text-[12px] font-semibold" style={{ color: "var(--m-text-secondary)" }}>
                Diff vs curated starter v{surface.starter_version}
              </p>
              <HardDiff
                  current={preview?.active_config}
                  starter={starterMap}
                />
            </div>
          )}
        </div>
      </div>
    </Card>
  );
}

function ShadowAnswerRow({
  id,
  a,
}: {
  id: string;
  a: {
    kind?: string;
    picked?: number;
    picked_label?: string;
    level?: number;
    probability?: number;
    confidence?: number;
    distribution?: number[];
  };
}) {
  return (
    <div className="flex items-center gap-2 text-[12px]" data-testid={`shadow-answer-${id}`}>
      <span className="w-40 shrink-0 truncate font-semibold">{id}</span>
      <span className="font-mono">
        {a.kind === "noul"
          ? `P=${((a.probability ?? 0) * 100).toFixed(0)}%`
          : a.kind === "score"
            ? `level ${a.level}`
            : a.kind === "choice"
              ? (a.picked_label ?? `#${a.picked}`)
              : "—"}
      </span>
      {a.confidence != null && (
        <span style={{ color: "var(--m-text-muted)" }}>conf {Math.round(a.confidence * 100)}%</span>
      )}
      {a.distribution != null && a.distribution.length > 0 && (
        <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
          dist [{a.distribution.map((d) => d.toFixed(2)).join(", ")}]
        </span>
      )}
    </div>
  );
}

/** Line-level honest diff: which template fields + questions changed. */
function HardDiff({ current, starter }: { current: unknown; starter: unknown }) {
  const stats = diffSummary(starter, current);
  if (stats.length === 0) return null;
  return (
    <ul className="mt-1 list-inside list-disc text-[12px]">
      {stats.map((line) => (
        <li key={line} style={{ color: "var(--m-text-secondary)" }}>
          {line}
        </li>
      ))}
    </ul>
  );
}

function UnconfiguredBadge() {
  return (
    <span
      className="inline-flex h-[18px] items-center rounded-full px-2 text-[9px] font-bold tracking-wide"
      style={{ background: "var(--m-status-neutral-bg)", color: "var(--m-status-neutral-fg)" }}
    >
      UNCONFIGURED
    </span>
  );
}

function V2Pill({ latest }: { latest?: number | null }) {
  return (
    <span
      className="inline-flex h-[18px] items-center rounded-full px-2 text-[9px] font-bold tracking-wide"
      style={{ background: "var(--m-status-warning-bg)", color: "var(--m-status-warning-fg)" }}
      title="A newer curated starter exists — diff it below before keeping the fork."
    >
      V{latest} AVAILABLE
    </span>
  );
}

/** The honest "decision model not configured / downed" posture (ADR 0011 §3). */
function DecisionModelUnknownPill({ curated }: { curated: boolean }) {
  return (
    <span
      className="inline-flex h-[18px] items-center rounded-full px-2 text-[9px] font-bold tracking-wide"
      style={
        curated
          ? { background: "var(--m-status-warning-bg)", color: "var(--m-status-warning-fg)" }
          : { background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }
      }
    >
      {curated ? "CREDENTIAL NOT SET" : "MODEL NOT CATALOGED"}
    </span>
  );
}

/** The what-activates note under the configured surface's posture row. */
function DecisionModelNote({ surface }: { surface: SurfaceOverview }) {
  if (surface.decision_model_curated == null) return null;
  if (surface.model_id == null) {
    return (
      <p className="mt-2 text-[12px]" data-testid="model-unknown-note" style={{ color: "var(--m-status-warning-fg)" }}>
        <strong>Decision model not configured.</strong> The surface answers quiet ("no card, no
        match — never a dead button") until{" "}
        {surface.decision_model_curated
          ? "an org or personal credentials row exists for the provider"
          : "an admin catalogs a decision-capable model (capabilities carry the `decision` edge) and the provider credential is sealed"}.
        Enable + flip to live without the model and nothing beyond the honest note changes.
      </p>
    );
  }
  return null;
}
