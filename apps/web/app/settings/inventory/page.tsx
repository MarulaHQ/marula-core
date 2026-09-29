// /settings/inventory — merge & identity org settings (Wave 22 lane 2).
//
// Purpose: the org's ONE identity-confidence dial surfaced as settings:
// the auto-merge threshold (drives the CI + contact queues), per-CI-type
// field precedence, and the JEV INTELLIGENT DEDUPE WORKFLOW editor —
// manual or scheduled passes, report-only or auto-merge, with the typed
// `dedupe_decision_not_configured` gate (an unconfigured decision model
// refuses the enable; ADR 0011 §3) and the run-now + last-report view.
//
// Non-purpose: no merge mechanics here — the queues live in
// /inventory/workbench; this page only stores the configuration.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Spinner,
  StatusTag,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useEffect, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { SettingsHeader } from "../../../components/settings/settings-shell";

type CiSettings = components["schemas"]["CiSettingsView"];
type PrecedenceRow = components["schemas"]["CiFieldPrecedence"];
type DedupeWorkflowView = components["schemas"]["DedupeWorkflowView"];
type DedupeRunResponse = components["schemas"]["DedupeRunResponse"];

export default function InventorySettingsPage(): ReactNode {
  const auth = useRequireAuth();
  const [settings, setSettings] = useState<CiSettings | null>(null);
  const [precedence, setPrecedence] = useState<PrecedenceRow[]>([]);
  const [workflow, setWorkflow] = useState<DedupeWorkflowView | null>(null);
  const [thresholdDraft, setThresholdDraft] = useState<string>("85");
  const [modeDraft, setModeDraft] = useState<string>("report_only");
  const [intervalDraft, setIntervalDraft] = useState<string>("");
  const [runBusy, setRunBusy] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [savedNote, setSavedNote] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const [settingsRes, precedenceRes] = await Promise.all([
        api.api.GET("/api/v1/config-items/settings"),
        api.api.GET("/api/v1/config-items/precedence"),
      ]);
      if (settingsRes.data === undefined) {
        setLoadError("Could not load merge settings — try reloading.");
        setLoading(false);
        return;
      }
      setSettings(settingsRes.data);
      setThresholdDraft(String(Math.round((settingsRes.data.auto_merge_threshold ?? 0.85) * 100)));
      setPrecedence(precedenceRes.data ?? []);
      // The workflow row may not exist yet (typed 404) — that is the
      // honest "not configured" state, not an error.
      const wfRes = await api.api.GET("/api/v1/dedupe/workflow");
      if (wfRes.data !== undefined) {
        setWorkflow(wfRes.data);
        setModeDraft(wfRes.data.workflow.mode);
        setIntervalDraft(
          wfRes.data.workflow.interval_hours !== null && wfRes.data.workflow.interval_hours !== undefined
            ? String(wfRes.data.workflow.interval_hours)
            : "",
        );
      }
      setLoading(false);
    })();
  }, [auth.status]);

  async function saveThreshold(): Promise<void> {
    setSaveError(null);
    setSavedNote(null);
    const pct = Number(thresholdDraft);
    if (!Number.isFinite(pct) || pct < 0 || pct > 100) {
      setSaveError("The threshold must be a percentage between 0 and 100.");
      return;
    }
    const value = Math.round(pct * 10) / 1000;
    const result = await runEnvelopeMutation({
      describe: "Save auto-merge threshold",
      buildBody: (base) => ({ ...base }),
      optimistic: () => {
        setSettings((cur) =>
          cur === null ? cur : { ...cur, auto_merge_threshold: value },
        );
      },
      revert: () => {
        void (async () => {
          const res = await api.api.GET("/api/v1/config-items/settings");
          if (res.data !== undefined) setSettings(res.data);
        })();
      },
      request: (client) =>
        client.PUT("/api/v1/config-items/settings", {
          body: { auto_merge_threshold: value },
        }),
    });
    if (result.ok === false) {
      setSaveError(`Could not save the threshold: ${rejectionText(result, "refused.")}`);
      return;
    }
    setSavedNote("Auto-merge threshold saved.");
  }

  async function saveWorkflow(enabled: boolean): Promise<void> {
    setSaveError(null);
    setSavedNote(null);
    if (workflow === null) return;
    const threshold = Number(thresholdDraft) / 100;
    const interval = intervalDraft.trim() === "" ? null : Number(intervalDraft);
    const result = await runEnvelopeMutation({
      describe: enabled ? "Enable intelligent dedupe" : "Update dedupe workflow",
      buildBody: (base) => ({ ...base }),
      optimistic: () => {
        setWorkflow((cur) =>
          cur === null
            ? cur
            : {
                ...cur,
                workflow: {
                  ...cur.workflow,
                  enabled,
                  confidence_threshold: threshold,
                  mode: modeDraft === "auto_merge" ? "auto_merge" : "report_only",
                  interval_hours: interval,
                },
              },
        );
      },
      revert: () => {
        void (async () => {
          const res = await api.api.GET("/api/v1/dedupe/workflow");
          if (res.data !== undefined) setWorkflow(res.data);
        })();
      },
      request: (client) =>
        client.PUT("/api/v1/dedupe/workflow", {
          body: {
            enabled,
            confidence_threshold: threshold,
            mode: modeDraft,
            interval_hours: interval,
            notify_user_ids: workflow.workflow.notify_user_ids ?? [],
            entity_kinds: workflow.workflow.entity_kinds ?? ["config_items", "contacts"],
          },
        }),
    });
    if (result.ok === false) {
      setSaveError(`Could not save the workflow: ${rejectionText(result, "refused.")}`);
      void (async () => {
        const res = await api.api.GET("/api/v1/dedupe/workflow");
        if (res.data !== undefined) setWorkflow(res.data);
      })();
      return;
    }
    const res = await api.api.GET("/api/v1/dedupe/workflow");
    if (res.data !== undefined) setWorkflow(res.data);
    setSavedNote(enabled ? "Intelligent dedupe workflow enabled." : "Workflow saved.");
  }

  async function runNow(): Promise<void> {
    setSaveError(null);
    setRunBusy(true);
    const result = await api.api.POST("/api/v1/dedupe/workflow/run", {
      body: { dry_run: false },
    });
    setRunBusy(false);
    if (result.error !== undefined) {
      setSaveError(`The run was refused: ${rejectionText(result.error, "refused.")}`);
      void (async () => {
        const res = await api.api.GET("/api/v1/dedupe/workflow");
        if (res.data !== undefined) setWorkflow(res.data);
      })();
      return;
    }
    const res = await api.api.GET("/api/v1/dedupe/workflow");
    if (res.data !== undefined) setWorkflow(res.data);
    setSavedNote("Dedupe run finished — the report is stamped on the workflow.");
  }

  if (loading) return <EmptyState message="Loading merge settings…" />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  const configured = workflow?.decision_configured ?? false;
  const wf = workflow?.workflow ?? null;
  const last = (wf?.last_result ?? {}) as {
    scanned?: number;
    asked?: number;
    above_threshold?: number;
    auto_merged?: number;
    truncated?: boolean;
    finished_at?: string;
  };

  return (
    <div className="grid gap-6">
      <SettingsHeader
        title="Inventory & merge"
        description="The identity-confidence dial, per-CI-type precedence, and the intelligent dedupe workflow. The review queue itself lives in the merge workbench."
      >
        <Link
          href="/inventory/workbench"
          className="rounded border px-2 py-1 text-[12px]"
          style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
        >
          Open workbench
        </Link>
      </SettingsHeader>

      {saveError !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="settings-error">
          {saveError}
        </p>
      )}
      {savedNote !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-success-fg, var(--m-text))" }} data-testid="settings-saved">
          {savedNote}
        </p>
      )}

      {/* ---- threshold + precedence ---- */}
      <section className="grid gap-2">
        <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
          Auto-merge threshold
        </h2>
        <Card className="grid gap-2 p-4">
          <div className="flex flex-wrap items-end gap-3">
            <div className="grid gap-1">
              <Label htmlFor="auto-merge-threshold">Confidence %</Label>
              <Input
                id="auto-merge-threshold"
                className="w-28"
                value={thresholdDraft}
                onChange={(e) => setThresholdDraft(e.target.value)}
                inputMode="decimal"
                data-testid="auto-merge-threshold"
              />
            </div>
            <Button size="sm" onClick={() => void saveThreshold()} data-testid="save-threshold">
              Save
            </Button>
          </div>
          <FieldHint>
            Candidates scoring at/above this fold automatically; everything
            below queues for review in the workbench. Phone-only contact
            matches (75%) can never auto-merge.
          </FieldHint>
        </Card>
      </section>

      <section className="grid gap-2">
        <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
          Per-CI-type field precedence
        </h2>
        {precedence.length === 0 ? (
          <EmptyState message="No custom precedence rows — the shipped defaults apply." />
        ) : (
          <Card className="grid gap-1 p-4">
            {precedence.map((p, i) => (
              <p key={i} className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                {`${p.field_key}: ${p.source_system} (priority ${p.priority})`}
              </p>
            ))}
          </Card>
        )}
      </section>

      {/* ---- the Jev workflow ---- */}
      <section className="grid gap-2">
        <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
          Intelligent dedupe (Jev)
        </h2>
        <Card className="grid gap-3 p-4" data-testid="jev-workflow">
          <div className="flex flex-wrap items-center gap-2">
            <StatusTag tone={wf?.enabled ? "success" : "neutral"}>
              {wf?.enabled ? "enabled" : "off"}
            </StatusTag>
            <StatusTag tone={configured ? "success" : "warning"}>
              {configured ? "decision model configured" : "decision model not configured"}
            </StatusTag>
          </div>
          <FieldHint>
            Runs over the pending dedupe queues, asking the decision model one
            question per candidate (suggestion-grade). Report-only records the
            answers into the workbench; auto-merge also folds the candidates at
            or above the workflow threshold (audited, undoable exactly like a
            human accept).
          </FieldHint>

          {!configured && (
            <p className="text-[12px]" style={{ color: "var(--m-status-warning-fg, var(--m-text-muted))" }}>
              Enabling requires a decision model — configure one under
              Settings → AI & connectors first. Until then the queues keep
              running on deterministic rules alone.
            </p>
          )}

          <div className="grid gap-3 md:grid-cols-3">
            <div className="grid gap-1">
              <Label htmlFor="dedupe-mode">Run mode</Label>
              <select
                id="dedupe-mode"
                className="rounded border bg-transparent px-2 py-1 text-[13px]"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text)" }}
                value={modeDraft}
                onChange={(e) => setModeDraft(e.target.value)}
                data-testid="dedupe-mode"
              >
                <option value="report_only">Report only (suggest, never merge)</option>
                <option value="auto_merge">Auto-merge above threshold</option>
              </select>
            </div>
            <div className="grid gap-1">
              <Label htmlFor="dedupe-interval">Schedule (hours, empty = manual)</Label>
              <Input
                id="dedupe-interval"
                className="w-32"
                value={intervalDraft}
                onChange={(e) => setIntervalDraft(e.target.value)}
                inputMode="numeric"
                placeholder="24"
                data-testid="dedupe-interval"
              />
            </div>
            <div className="grid content-end gap-2">
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  disabled={runBusy}
                  onClick={() => void saveWorkflow(true)}
                  data-testid="dedupe-enable"
                >
                  Enable
                </Button>
                <Button size="sm" variant="secondary" disabled={runBusy} onClick={() => void saveWorkflow(false)}>
                  Disable
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={runBusy}
                  onClick={() => void runNow()}
                  data-testid="dedupe-run-now"
                >
                  Run now
                </Button>
              </div>
            </div>
          </div>

          {wf !== null && wf.last_run_at !== null && (
            <div className="grid gap-1 rounded p-3" style={{ background: "var(--m-surface)" }} data-testid="jev-last-run">
              <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {`last run ${new Date(wf.last_run_at as string).toLocaleString()}`}
              </span>
              <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                {`${last.scanned ?? 0} scanned · ${last.asked ?? 0} asked · ${last.above_threshold ?? 0} at/above threshold · ${last.auto_merged ?? 0} auto-merged`}
                {last.truncated ? " · capped (more remain)" : ""}
              </p>
            </div>
          )}
        </Card>
      </section>
    </div>
  );
}
