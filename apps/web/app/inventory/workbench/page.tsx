// /inventory/workbench — the guided merge workbench (Wave 22 lane 2, ADR
// 0003 extension).
//
// Purpose: the review surface for ALL dedupe queues in one place, split
// layout (queue on the left, side-by-side detail on the right — the
// GitLab-MR-dashboard pattern):
// - Devices: CI merge candidates with side-by-side source cards across ALL
//   ConfigSourceLinks, the field-by-field picker (observed vs curated
//   tiers, curated sticky, one-click "accept the new value"), batch
//   accept/reject/defer, the decision evidence inline (SILENT when the
//   org's decision model is unconfigured — never a dead card), the
//   threshold context, and the full undo contract (split).
// - Contacts: the email/phone identity-key queue with the same mechanics.
// - Clients: org_map reconciliation cards (vendor org → Marula client;
//   an accept writes the durable operator map, never auto-applied).
// - Jev: the "run intelligent dedupe" trigger (admin) with the last run
//   report inline; below-threshold suggestions stay queued for humans.
//
// Non-purpose: no CI editing (curate lives on the asset documents) and no
// merge mechanics here — this page renders the queue and posts decisions;
// the backend owns every law.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Spinner,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";

type CandidateView = components["schemas"]["CandidateView"];
type MergeWorkbenchView = components["schemas"]["MergeWorkbenchView"];
type EntityCandidateView = components["schemas"]["EntityCandidateView"];
type EntityWorkbenchView = components["schemas"]["EntityWorkbenchView"];
type EntityMerge = components["schemas"]["EntityMerge"];
type ClientOrgMap = components["schemas"]["ClientOrgMap"];
type CiMerge = components["schemas"]["CiMerge"];
type DedupeRunReport = components["schemas"]["DedupeRunReport"];

type Queue = "devices" | "contacts" | "clients";
type DecisionReport = components["schemas"]["DedupeRunResponse"] | null;

const QUEUE_TABS: { key: Queue; label: string; hint: string }[] = [
  { key: "devices", hint: "Config items across identity feeds", label: "Devices" },
  { key: "contacts", hint: "Email/phone identity keys", label: "Contacts" },
  { key: "clients", hint: "Vendor org → client mapping", label: "Clients" },
];

function jsonValue(v: unknown): string {
  if (v === null || v === undefined) return "—";
  if (typeof v === "string") return v === "" ? "—" : v;
  if (typeof v === "boolean" || typeof v === "number") return String(v);
  return JSON.stringify(v);
}

function signalList(evidence: unknown): string[] {
  if (evidence === null || typeof evidence !== "object") return [];
  const signals = (evidence as { signals?: unknown }).signals;
  if (!Array.isArray(signals)) return [];
  return signals
    .filter((s): s is Record<string, unknown> => s !== null && typeof s === "object")
    .map((s) =>
      Object.entries(s)
        .map(([k, v]) => `${k.replace(/_/g, " ")}: ${Array.isArray(v) ? v.map(String).join(", ") : String(v)}`)
        .join(" · "),
    );
}

function fieldPreview(v: unknown): string {
  const s = jsonValue(v);
  return s.length > 120 ? `${s.slice(0, 120)}…` : s;
}

function sourceEntry(v: unknown): string | null {
  if (v === null || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  const src = typeof o.source_system === "string" ? o.source_system : null;
  const at = typeof o.observed_at === "string" ? o.observed_at : null;
  const contradiction = o.contradicts_curated === true;
  if (src === null && !contradiction) return null;
  return `${src ?? "?"}${at ? ` · seen ${new Date(at).toLocaleDateString()}` : ""}${contradiction ? " · contradicts curated" : ""}`;
}

// ---- the queue (left rail) ----------------------------------------------------

interface QueueRow {
  id: string;
  status: string;
  confidence: number;
  reason: string;
  primaryLabel: string;
  candidateLabel: string;
  evidence: string[];
}

function RowCard({
  row,
  selected,
  active,
  busy,
  onSelect,
  onOpen,
}: {
  row: QueueRow;
  selected: boolean;
  active: boolean;
  busy: boolean;
  onSelect: () => void;
  onOpen: () => void;
}) {
  return (
    <div
      data-testid={`workbench-row-${row.id}`}
      className="rounded-lg border p-3"
      style={{
        borderColor: active ? "var(--m-accent)" : "var(--m-border)",
        background: active ? "var(--m-surface)" : "transparent",
        opacity: busy ? 0.6 : 1,
      }}
    >
      <div className="flex items-start gap-2">
        <input
          type="checkbox"
          aria-label={`Select ${row.primaryLabel} candidate`}
          checked={selected}
          disabled={row.status !== "pending"}
          onChange={onSelect}
          className="mt-1"
        />
        <button type="button" onClick={onOpen} className="min-w-0 flex-1 text-left">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="truncate text-sm font-medium" style={{ color: "var(--m-text)" }}>
              {row.primaryLabel}
            </span>
            <span style={{ color: "var(--m-text-muted)" }}>←</span>
            <span className="truncate text-sm" style={{ color: "var(--m-text)" }}>
              {row.candidateLabel}
            </span>
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-1.5">
            <Badge tone={row.confidence >= 0.9 ? "success" : "warning"}>
              {`${Math.round(row.confidence * 100)}%`}
            </Badge>
            {row.status !== "pending" && (
              <Badge
                tone={
                  row.status === "deferred" ? "warning" : row.status === "rejected" ? "neutral" : "success"
                }
              >
                {row.status}
              </Badge>
            )}
            <span className="truncate text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              {row.reason}
            </span>
          </div>
          {row.evidence.length > 0 && (
            <ul className="mt-1 grid gap-0.5">
              {row.evidence.slice(0, 2).map((s, i) => (
                <li key={i} className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                  {s}
                </li>
              ))}
            </ul>
          )}
        </button>
      </div>
    </div>
  );
}

// ---- the page -------------------------------------------------------------------

export default function WorkbenchPage(): ReactNode {
  const auth = useRequireAuth();
  const [queue, setQueue] = useState<Queue>("devices");
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  // Queue rows (per tab) + selection + detail.
  const [ciRows, setCiRows] = useState<CandidateView[] | null>(null);
  const [entityRows, setEntityRows] = useState<EntityCandidateView[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [activeId, setActiveId] = useState<string | null>(null);

  // Detail payloads.
  const [ciDetail, setCiDetail] = useState<MergeWorkbenchView | null>(null);
  const [entityDetail, setEntityDetail] = useState<EntityWorkbenchView | null>(null);
  const [entityMerges, setEntityMerges] = useState<EntityMerge[]>([]);
  const [orgMap, setOrgMap] = useState<ClientOrgMap[] | null>(null);

  // The Jev trigger.
  const [dedupeReport, setDedupeReport] = useState<DecisionReport>(null);
  const [dedupeBusy, setDedupeBusy] = useState(false);

  const rowsFor = useCallback((): QueueRow[] => {
    if (queue === "devices") {
      return (ciRows ?? []).map((c) => ({
        id: c.id,
        status: c.status,
        confidence: c.confidence,
        reason: c.reason,
        primaryLabel: c.primary_tag,
        candidateLabel: c.candidate_tag,
        evidence: signalList(c.evidence),
      }));
    }
    return (entityRows ?? [])
      .filter((c) => (queue === "contacts" ? c.entity_kind === "contact" : c.entity_kind === "client"))
      .map((c) => ({
        id: c.id,
        status: c.status,
        confidence: c.confidence,
        reason: c.reason,
        primaryLabel: c.primary_label,
        candidateLabel:
          c.candidate_label ?? c.vendor_org_id ?? "vendor org",
        evidence: signalList(c.evidence),
      }));
  }, [queue, ciRows, entityRows]);

  const refresh = useCallback(async (): Promise<void> => {
    const [ciRes, entityRes] = await Promise.all([
      queue === "devices"
        ? api.api.GET("/api/v1/config-items/merge-candidates")
        : Promise.resolve({ data: undefined }),
      queue === "devices"
        ? Promise.resolve({ data: undefined })
        : api.api.GET("/api/v1/entity-dedupe/candidates"),
    ]);
    if (ciRes.data !== undefined) setCiRows(ciRes.data ?? []);
    if (entityRes.data !== undefined) setEntityRows(entityRes.data ?? []);
  }, [queue]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    setLoading(true);
    setLoadError(null);
    setSelected(new Set());
    setActiveId(null);
    setCiDetail(null);
    setEntityDetail(null);
    void (async () => {
      const [ciRes, entityRes] = await Promise.all([
        api.api.GET("/api/v1/config-items/merge-candidates"),
        api.api.GET("/api/v1/entity-dedupe/candidates"),
      ]);
      if (ciRes.error !== undefined || entityRes.data === undefined) {
        setLoadError("Could not load the workbench — try reloading.");
        setLoading(false);
        return;
      }
      setCiRows(ciRes.data ?? []);
      setEntityRows(entityRes.data ?? []);
      if (queue === "clients") {
        const mapRes = await api.api.GET("/api/v1/clients/org-map");
        setOrgMap(mapRes.data ?? []);
      }
      setLoading(false);
    })();
  }, [auth.status, queue]);

  const openDetail = useCallback(
    async (id: string): Promise<void> => {
      setActiveId(id);
      setCiDetail(null);
      setEntityDetail(null);
      if (queue === "devices") {
        const res = await api.api.GET("/api/v1/config-items/merge-candidates/{id}/workbench", {
          params: { path: { id } },
        });
        if (res.data) setCiDetail(res.data);
      } else {
        const res = await api.api.GET("/api/v1/entity-dedupe/candidates/{id}/workbench", {
          params: { path: { id } },
        });
        if (res.data) setEntityDetail(res.data);
      }
    },
    [queue],
  );

  async function actCi(
    c: CandidateView,
    action: "accept" | "reject" | "defer" | "restore",
  ): Promise<void> {
    setActionError(null);
    setBusyId(c.id);
    const result = await runEnvelopeMutation({
      describe: action,
      entityId: c.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {
        setCiRows((cur) =>
          (cur ?? []).map((row) =>
            row.id === c.id
              ? {
                  ...row,
                  status:
                    action === "restore"
                      ? "pending"
                      : action === "accept"
                        ? "accepted"
                        : action === "reject"
                          ? "rejected"
                          : "deferred",
                }
              : row,
          ),
        );
        // Keep the open detail in step with the decision so the deferred
        // Restore affordance (and decided banners) render immediately.
        setCiDetail((cur) =>
          cur && cur.candidate.id === c.id
            ? {
                ...cur,
                candidate: {
                  ...cur.candidate,
                  status:
                    action === "restore"
                      ? "pending"
                      : action === "accept"
                        ? "accepted"
                        : action === "reject"
                          ? "rejected"
                          : "deferred",
                },
              }
            : cur,
        );
      },
      revert: () => {
        setCiRows((cur) =>
          (cur ?? []).map((row) => (row.id === c.id ? { ...row, status: c.status } : row)),
        );
      },
      request: (client) =>
        client.POST(
          `/api/v1/config-items/merge-candidates/{id}/${action === "restore" ? "restore" : action}`,
          { params: { path: { id: c.id } } },
        ),
    });
    setBusyId(null);
    if (result.ok === false) {
      setActionError(`Could not ${action} candidate: ${rejectionText(result, "rejected.")}`);
      return;
    }
    if (action === "accept") {
      // The merge produced an undoable ledger row — reload the candidate
      // list and surface the split affordance on the ledger card.
      await refresh();
    }
  }

  async function batchCi(action: "accept" | "reject" | "defer"): Promise<void> {
    const ids = [...selected];
    if (ids.length === 0) return;
    setActionError(null);
    const result = await runEnvelopeMutation({
      describe: `Batch ${action} (${ids.length})`,
      entityId: ids[0],
      buildBody: (base) => ({ ...base }),
      optimistic: () => {
        setCiRows((cur) =>
          (cur ?? []).map((row) =>
            ids.includes(row.id) && row.status === "pending"
              ? { ...row, status: action === "accept" ? "accepted" : action === "reject" ? "rejected" : "deferred" }
              : row,
          ),
        );
      },
      revert: () => {
        void refresh();
      },
      request: (client) =>
        client.POST("/api/v1/config-items/merge-candidates/batch", {
          body: { ids, action },
        }),
    });
    setSelected(new Set());
    if (result.ok === false) {
      setActionError(`Batch ${action} failed: ${rejectionText(result, "rejected.")}`);
      return;
    }
    await refresh();
  }

  async function actEntity(
    c: EntityCandidateView,
    action: "accept" | "reject" | "defer" | "restore",
  ): Promise<void> {
    setActionError(null);
    setBusyId(c.id);
    const nextStatus =
      action === "restore"
        ? "pending"
        : action === "accept"
          ? "accepted"
          : action === "reject"
            ? "rejected"
            : "deferred";
    const optimistic = () => {
      setEntityRows((cur) =>
        (cur ?? []).map((row) => (row.id === c.id ? { ...row, status: nextStatus } : row)),
      );
    };
    const revert = () => {
      setEntityRows((cur) =>
        (cur ?? []).map((row) => (row.id === c.id ? { ...row, status: c.status } : row)),
      );
    };
    optimistic();
    const path =
      action === "restore"
        ? "/api/v1/entity-dedupe/candidates/{id}/restore"
        : `/api/v1/entity-dedupe/candidates/{id}/${action}`;
    const result = await api.api.POST(
      path as "/api/v1/entity-dedupe/candidates/{id}/accept",
      { params: { path: { id: c.id } } },
    );
    setBusyId(null);
    if (result.error !== undefined) {
      revert();
      setActionError(`Could not ${action}: ${rejectionText(result.error, "rejected.")}`);
      return;
    }
    if (action === "accept" && queue === "contacts" && result.data && "id" in result.data) {
      // The fold returns the merge audit row — the undo contract rides
      // the ledger card below.
      setEntityMerges((cur) => [result.data as EntityMerge, ...cur]);
    }
    if (queue === "clients") {
      const mapRes = await api.api.GET("/api/v1/clients/org-map");
      setOrgMap(mapRes.data ?? []);
    }
    await refresh();
  }

  async function splitEntity(merge: EntityMerge): Promise<void> {
    setActionError(null);
    setBusyId(merge.id);
    const result = await runEnvelopeMutation({
      describe: "Undo contact merge (split)",
      entityId: merge.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {
        setEntityMerges((cur) =>
          cur.map((m) => (m.id === merge.id ? { ...m, status: "undone", undone_at: new Date().toISOString() } : m)),
        );
      },
      revert: () => {
        setEntityMerges((cur) =>
          cur.map((m) => (m.id === merge.id ? { ...m, status: merge.status } : m)),
        );
      },
      request: (client) =>
        client.POST("/api/v1/entity-dedupe/merges/{id}/split", { params: { path: { id: merge.id } } }),
    });
    setBusyId(null);
    if (result.ok === false) {
      setActionError(`Could not undo: ${rejectionText(result, "rejected.")}`);
      return;
    }
    await refresh();
  }

  async function runJev(): Promise<void> {
    setActionError(null);
    setDedupeBusy(true);
    setDedupeReport(null);
    const result = await api.api.POST("/api/v1/dedupe/workflow/run", {
      body: { dry_run: false },
    });
    setDedupeBusy(false);
    if (result.error !== undefined) {
      setActionError(`The dedupe run was refused: ${rejectionText(result.error ?? {}, "refused.")}`);
      return;
    }
    setDedupeReport(result.data ?? null);
  }

  if (loading) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  const rows = rowsFor();
  const pendingRows = rows.filter((r) => r.status === "pending");
  const deferredRows = rows.filter((r) => r.status === "deferred");
  const decidedRows = rows.filter((r) => r.status !== "pending" && r.status !== "deferred");
  const selectedPending = [...selected].filter((id) => rows.find((r) => r.id === id)?.status === "pending");

  return (
    <div className="mx-auto grid w-full max-w-[1500px] gap-4 p-4 md:p-6">
      <header className="m-ledger-header">
        <div className="m-ledger-title-row">
          <h1 className="m-ledger-title">Merge workbench</h1>
          <div className="m-ledger-header-action">
            {dedupeReport !== null && dedupeReport.report !== undefined && (
              <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }} data-testid="jev-last-report">
                {`last run: ${dedupeReport.report.scanned} scanned · ${dedupeReport.report.auto_merged} auto-merged`}
              </span>
            )}
            <Button size="sm" variant="secondary" disabled={dedupeBusy} onClick={() => void runJev()} data-testid="jev-run">
              Run intelligent dedupe
            </Button>
          <Link
            href="/settings/inventory"
            className="rounded border px-2 py-1 text-[12px]"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
            data-testid="workbench-settings-link"
          >
            Queue settings
          </Link>
          </div>
        </div>
        <p className="m-ledger-subtitle">
          One review surface for every dedupe queue — side-by-side evidence, field-level
          picks, batch decisions, and a full undo trail. The merge is a person&apos;s call:
          this queue carries no AI mark, no suggested answer and no sparkle, because a
          duplicate is a decision, not a notification.
        </p>
      </header>

      {actionError !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="workbench-action-error">
          {actionError}
        </p>
      )}

      {dedupeReport !== null && (
        <Card className="grid gap-1 p-4" data-testid="jev-report">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              Intelligent dedupe run
            </span>
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              {dedupeReport.dry_run ? "dry run" : "applied"}
            </span>
          </div>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            {`${dedupeReport.report.scanned} scanned · ${dedupeReport.report.asked} asked · ${dedupeReport.report.above_threshold} at/above threshold · ${dedupeReport.report.auto_merged} auto-merged`}
            {dedupeReport.report.truncated ? " · capped (more candidates remain)" : ""}
          </p>
          {dedupeReport.report.outcomes.length > 0 && (
            <ul className="grid gap-0.5">
              {dedupeReport.report.outcomes.slice(0, 8).map((o, i) => (
                <li key={i} className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  {`${o.label} — ${o.disposition}`}
                </li>
              ))}
            </ul>
          )}
        </Card>
      )}

      <div role="tablist" aria-label="Dedupe queues" className="flex flex-wrap gap-1.5">
        {QUEUE_TABS.map((t) => (
          <button
            key={t.key}
            role="tab"
            aria-selected={queue === t.key}
            data-testid={`workbench-tab-${t.key}`}
            onClick={() => setQueue(t.key)}
            className="rounded-full border px-3 py-1 text-[12px]"
            style={{
              borderColor: queue === t.key ? "var(--m-accent)" : "var(--m-border)",
              background: queue === t.key ? "var(--m-accent)" : "transparent",
              color: queue === t.key ? "var(--m-accent-fg, #fff)" : "var(--m-text-secondary)",
              transition: "background-color 100ms ease",
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div className="grid gap-4 lg:grid-cols-[minmax(320px,420px)_1fr] lg:items-start">
        {/* ---- queue column ---- */}
        <div className="grid gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }} data-testid="workbench-queue-count">
              {pendingRows.length} pending · {deferredCount(rows)} deferred
            </span>
            {selectedPending.length > 0 && (
              <div className="flex gap-1.5" data-testid="workbench-batch">
                <Button size="sm" disabled={busyId !== null} onClick={() => void batchCi("accept")} data-testid="workbench-batch-accept">
                  Accept {selectedPending.length}
                </Button>
                <Button size="sm" variant="secondary" disabled={busyId !== null} onClick={() => void batchCi("reject")}>
                  Keep {selectedPending.length}
                </Button>
                <Button size="sm" variant="secondary" disabled={busyId !== null} onClick={() => void batchCi("defer")}>
                  Defer {selectedPending.length}
                </Button>
              </div>
            )}
          </div>
          <div className="grid max-h-[70vh] gap-2 overflow-y-auto pr-1">
            {rows.map((r) => (
              <RowCard
                key={r.id}
                row={r}
                selected={selected.has(r.id)}
                active={activeId === r.id}
                busy={busyId === r.id}
                onSelect={() =>
                  setSelected((cur) => {
                    const next = new Set(cur);
                    if (next.has(r.id)) next.delete(r.id);
                    else next.add(r.id);
                    return next;
                  })
                }
                onOpen={() => void openDetail(r.id)}
              />
            ))}
            {rows.length === 0 && (
              <EmptyState message="Nothing queued — the deterministic matcher found nothing ambiguous." />
            )}
          </div>
          {queue === "clients" && orgMap !== null && orgMap.length > 0 && (
            <Card className="grid gap-1 p-4" data-testid="org-map-summary">
              <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
                Durable org map
              </span>
              {orgMap.map((m) => (
                <p key={m.id} className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                  {`${m.source_system} · ${m.vendor_org_id} → ${m.vendor_label ?? "client"}`}
                </p>
              ))}
            </Card>
          )}
          {queue === "contacts" && entityMerges.length > 0 && (
            <Card className="grid gap-1 p-4" data-testid="entity-merge-ledger">
              <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
                Contact merge ledger
              </span>
              {entityMerges.map((m) => (
                <div key={m.id} className="flex items-center justify-between gap-2">
                  <span className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                    {`${m.primary_entity_id.slice(0, 8)} ← ${m.merged_entity_id.slice(0, 8)} · ${m.status}`}
                  </span>
                  {m.status === "applied" && (
                    <Button size="sm" variant="secondary" onClick={() => void splitEntity(m)}>
                      Undo
                    </Button>
                  )}
                </div>
              ))}
            </Card>
          )}
        </div>

        {/* ---- detail column ---- */}
        <div className="grid gap-4" data-testid="workbench-detail">
          {activeId === null && (
            <EmptyState message="Select a candidate to compare its sources side by side." />
          )}
          {queue === "devices" && ciDetail !== null && ciDetail.candidate.id === activeId && (
            <CiDetail
              detail={ciDetail}
              busy={busyId !== null}
              busyId={busyId}
              onAction={(c, a) => void actCi(c, a)}
            />
          )}
          {queue !== "devices" && entityDetail !== null && entityDetail.candidate.id === activeId && (
            <EntityDetail
              detail={entityDetail}
              queue={queue}
              busy={busyId !== null}
              busyId={busyId}
              onEntityAction={(c, a) => void actEntity(c, a)}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function deferredCount(rows: QueueRow[]): number {
  return rows.filter((r) => r.status === "deferred").length;
}

async function openDetail(id: string): Promise<void> {
  // placeholder replaced by the component's closure in the JSX below
  void id;
}

// ---- CI detail ----------------------------------------------------------------

function TierBadge({ label, value }: { label: string; value: string }): ReactNode {
  return (
    <span className="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10px]" style={{ borderColor: "var(--m-border)" }}>
      <span style={{ color: "var(--m-text-muted)" }}>{label}</span>
      <span style={{ color: "var(--m-text)" }}>{value}</span>
    </span>
  );
}

function SourceCard({
  title,
  links,
  observed,
  curated,
  fieldSources,
  identityKeys,
  testId,
}: {
  title: string;
  links: { source_system: string; external_id: string; last_seen_at: string }[];
  observed: Record<string, unknown>;
  curated: Record<string, unknown>;
  fieldSources?: Record<string, unknown>;
  identityKeys?: unknown;
  testId: string;
}): ReactNode {
  return (
    <Card className="grid gap-2 p-4" data-testid={testId}>
      <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
        {title}
      </span>
      <div className="grid gap-1">
        <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--m-text-muted)" }}>
          Source links
        </span>
        {links.map((l) => (
          <p key={`${l.source_system}-${l.external_id}`} className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
            {`${l.source_system} · ${l.external_id} · last seen ${new Date(l.last_seen_at).toLocaleDateString()}`}
          </p>
        ))}
        {links.length === 0 && <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>No source links yet.</p>}
      </div>
      <div className="grid gap-1">
        <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--m-text-muted)" }}>
          Observed tier
        </span>
        {Object.keys(observed).length === 0 ? (
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>No observed fields.</p>
        ) : (
          Object.entries(observed).map(([k, v]) => (
            <p key={k} className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
              {`${k}: ${fieldPreview(v)}`}
            </p>
          ))
        )}
      </div>
      <div className="grid gap-1">
        <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--m-text-muted)" }}>
          Curated (sticky)
        </span>
        {Object.keys(curated).length === 0 ? (
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>No curated fields — human edits land here.</p>
        ) : (
          Object.entries(curated).map(([k, v]) => (
            <p key={k} className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
              {`${k}: ${fieldPreview(v)}`}
            </p>
          ))
        )}
      </div>
    </Card>
  );
}

function CiDetail({
  detail,
  busy,
  busyId,
  onAction,
}: {
  detail: MergeWorkbenchView;
  busy: boolean;
  busyId: string | null;
  onAction: (c: CandidateView, action: "accept" | "reject" | "defer" | "restore") => void;
}): ReactNode {
  const c = detail.candidate;
  const isPending = c.status === "pending";
  return (
    <div className="grid gap-4" data-testid="ci-workbench-detail">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            {c.primary_tag} ← {c.candidate_tag}
          </span>
          <Badge tone={c.confidence >= 0.9 ? "success" : "warning"}>
            {`${Math.round(c.confidence * 100)}% confidence`}
          </Badge>
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            {c.reason}
          </span>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {isPending && (
            <>
              <Button size="sm" disabled={busy} onClick={() => onAction(c, "accept")} data-testid="detail-accept">
                Accept merge
              </Button>
              <Button size="sm" variant="secondary" disabled={busy} onClick={() => onAction(c, "reject")}>
                Keep separate
              </Button>
              <Button size="sm" variant="secondary" disabled={busy} onClick={() => onAction(c, "defer")}>
                Defer
              </Button>
            </>
          )}
          {!isPending && c.status === "deferred" && (
            <Button size="sm" variant="secondary" disabled={busy} onClick={() => onAction(c, "restore")} data-testid="detail-restore">
              Restore to queue
            </Button>
          )}
          {!isPending && c.status !== "deferred" && (
            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              {`decided: ${c.status}`}
            </span>
          )}
        </div>
      </div>

      <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        {`queued below the org's ${Math.round(detail.auto_merge_threshold * 100)}% auto-merge threshold — that is why this row needs you.`}
      </p>

      <ul className="grid gap-0.5">
        {signalList(c.evidence).map((s, i) => (
          <li key={i} className="text-[11px]" style={{ color: "var(--m-text-secondary)" }} data-testid={`detail-evidence-${i}`}>
            {s}
          </li>
        ))}
      </ul>

      {detail.decision != null && (
        <Card className="grid gap-1 p-4" data-testid="detail-decision">
          <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--m-text-muted)" }}>
            Jev suggestion (decision_record_dedupe · {detail.decision.mode})
          </span>
          <pre
            className="max-h-40 overflow-auto rounded p-2 text-[11px]"
            style={{ background: "var(--m-surface)", color: "var(--m-text-secondary)" }}
          >
            {JSON.stringify(detail.decision.answers, null, 2)}
          </pre>
          <p className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
            Suggestion-grade only — the merge still needs a human decision.
          </p>
        </Card>
      )}

      <div className="grid gap-3 md:grid-cols-2">
        <SourceCard
          title={`Survivor: ${c.primary_tag}`}
          links={(detail.primary.source_links ?? []) as { source_system: string; external_id: string; last_seen_at: string }[]}
          observed={(detail.primary.observed_fields ?? {}) as Record<string, unknown>}
          curated={(detail.primary.curated_fields ?? {}) as Record<string, unknown>}
          testId="detail-primary-card"
        />
        <SourceCard
          title={`Candidate: ${c.candidate_tag}`}
          links={(detail.candidate_side.source_links ?? []) as { source_system: string; external_id: string; last_seen_at: string }[]}
          observed={(detail.candidate_side.observed_fields ?? {}) as Record<string, unknown>}
          curated={(detail.candidate_side.curated_fields ?? {}) as Record<string, unknown>}
          testId="detail-candidate-card"
        />
      </div>

      {detail.fields.length > 0 && (
        <Card className="grid gap-2 p-4" data-testid="detail-field-picker">
          <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Field-by-field comparison
          </span>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Curated values are sticky — a feed can never overwrite them; a strong
            observed contradiction shows the one-click &quot;accept the new value&quot;.
          </p>
          {/*
            The side-by-side duplicate comparison the pack asks for. This queue
            is a HUMAN decision surface, so it carries no AI mark of any kind:
            no sparkle, no hint, no suggested answer. Every value here is either
            observed by a feed or curated by a person, and the rows below say
            which — the decision is made by reading them, not by being told.
          */}
          <div className="m-ledger-merge-compare" data-testid="detail-compare">
            <div className="m-ledger-merge-head m-ledger-merge-cell is-label">Field</div>
            <div className="m-ledger-merge-head m-ledger-merge-cell">Survivor · {c.primary_tag}</div>
            <div className="m-ledger-merge-head m-ledger-merge-cell">Candidate · {c.candidate_tag}</div>
            {detail.fields.map((f) => {
              const primaryVal = f.primary_curated ?? f.primary_observed;
              const candVal = f.candidate_curated ?? f.candidate_observed;
              const conflicts =
                candVal !== undefined &&
                primaryVal !== undefined &&
                JSON.stringify(candVal) !== JSON.stringify(primaryVal);
              const primarySrc = sourceEntry(f.primary_source);
              const candSrc = sourceEntry(f.candidate_source);
              return (
                <div
                  key={`cmp-${f.key}`}
                  className="contents"
                  data-testid={`detail-compare-${f.key}`}
                  data-conflicts={conflicts ? "true" : "false"}
                >
                  <div className="m-ledger-merge-cell is-label">{f.key}</div>
                  <div
                    className="m-ledger-merge-cell"
                    data-tier={f.primary_curated !== undefined ? "curated" : "observed"}
                  >
                    <span style={{ color: "var(--m-text-secondary)" }}>
                      {fieldPreview(f.primary_curated ?? f.primary_observed)}
                      {f.primary_curated !== undefined ? (
                        <span className="ml-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                          (curated)
                        </span>
                      ) : null}
                      {primarySrc !== null ? (
                        <span className="ml-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                          · {primarySrc}
                        </span>
                      ) : null}
                    </span>
                  </div>
                  <div
                    className="m-ledger-merge-cell"
                    data-tier={f.candidate_curated !== undefined ? "curated" : "observed"}
                    data-conflicts={conflicts ? "true" : "false"}
                  >
                    <span style={{ color: "var(--m-text-secondary)" }}>
                      {fieldPreview(f.candidate_curated ?? f.candidate_observed)}
                      {candSrc !== null ? (
                        <span className="ml-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                          · {candSrc}
                        </span>
                      ) : null}
                    </span>
                    {conflicts && isPending ? (
                      <span className="text-[10px]" style={{ color: "var(--m-status-warning-fg)" }}>
                        differs — merge, then curate the value you want (sticky)
                      </span>
                    ) : null}
                  </div>
                </div>
              );
            })}
          </div>
          {/* The keyboard-reachable picker stays: the grid above is for
              reading, this is for choosing, and both are the human's. */}
          <div className="grid gap-1.5">
            {detail.fields.map((f) => {
              const primaryVal = f.primary_curated ?? f.primary_observed;
              const candVal = f.candidate_curated ?? f.candidate_observed;
              const conflicts =
                candVal !== undefined &&
                primaryVal !== undefined &&
                JSON.stringify(candVal) !== JSON.stringify(primaryVal);
              const primarySrc = sourceEntry(f.primary_source);
              const candSrc = sourceEntry(f.candidate_source);
              return (
                <div
                  key={f.key}
                  className="grid gap-1 rounded border p-2 md:grid-cols-[140px_1fr_1fr]"
                  style={{ borderColor: "var(--m-border)" }}
                  data-testid={`detail-field-${f.key}`}
                >
                  <span className="text-[12px] font-medium" style={{ color: "var(--m-text)" }}>
                    {f.key}
                  </span>
                  <div className="grid gap-0.5">
                    <span className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                      {fieldPreview(f.primary_curated ?? f.primary_observed)}
                      {f.primary_curated !== undefined && (
                        <span className="ml-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                          (curated)
                        </span>
                      )}
                      {primarySrc !== null && (
                        <span className="ml-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                          · {primarySrc}
                        </span>
                      )}
                    </span>
                  </div>
                  <div className="grid gap-0.5">
                    <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      {fieldPreview(f.candidate_curated ?? f.candidate_observed)}
                      {candSrc !== null && (
                        <span className="ml-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                          · {candSrc}
                        </span>
                      )}
                    </span>
                    {conflicts && isPending && (
                      <span className="text-[10px]" style={{ color: "var(--m-status-warning-fg, var(--m-text-muted))" }}>
                        conflicts — accept the merge, then curate the value you want (sticky)
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </Card>
      )}

      {detail.related_pending.length > 0 && (
        <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          {`${detail.related_pending.length} related pending candidate(s) share these records — batch context for your decision.`}
        </p>
      )}
      <span className="sr-only" data-testid="detail-busy">
        {busyId ?? "idle"}
      </span>
    </div>
  );
}

// ---- entity (contacts/clients) detail -------------------------------------------

function EntityDetail({
  detail,
  queue,
  busy,
  onEntityAction,
}: {
  detail: EntityWorkbenchView;
  queue: Queue;
  busy: boolean;
  busyId: string | null;
  onEntityAction: (c: EntityCandidateView, action: "accept" | "reject" | "defer" | "restore") => void;
}): ReactNode {
  const c = detail.candidate;
  const isPending = c.status === "pending";
  return (
    <div className="grid gap-4" data-testid="entity-detail">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            {c.primary_label} ← {c.candidate_label ?? "vendor org"}
          </span>
          <Badge tone={c.confidence >= 0.9 ? "success" : "warning"}>
            {`${Math.round(c.confidence * 100)}% confidence`}
          </Badge>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {isPending && (
            <>
              <Button size="sm" disabled={busy} onClick={() => onEntityAction(c, "accept")} data-testid="entity-detail-accept">
                {queue === "clients" ? "Map to this client" : "Accept merge"}
              </Button>
              <Button size="sm" variant="secondary" disabled={busy} onClick={() => onEntityAction(c, "reject")}>
                Keep {queue === "clients" ? "unmapped" : "separate"}
              </Button>
              <Button size="sm" variant="secondary" disabled={busy} onClick={() => onEntityAction(c, "defer")}>
                Defer
              </Button>
            </>
          )}
          {!isPending && c.status === "deferred" && (
            <Button size="sm" variant="secondary" disabled={busy} onClick={() => onEntityAction(c, "restore")}>
              Restore
            </Button>
          )}
        </div>
      </div>
      <p className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
        {c.reason}
      </p>

      <ul className="grid gap-0.5">
        {signalList(c.evidence).map((s, i) => (
          <li key={i} className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
            {s}
          </li>
        ))}
      </ul>

      {detail.decision != null && (
        <Card className="grid gap-1 p-4" data-testid="entity-detail-decision">
          <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--m-text-muted)" }}>
            Jev suggestion (decision_record_dedupe · {detail.decision.mode})
          </span>
          <pre
            className="max-h-40 overflow-auto rounded p-2 text-[11px]"
            style={{ background: "var(--m-surface)", color: "var(--m-text-secondary)" }}
          >
            {JSON.stringify(detail.decision.answers, null, 2)}
          </pre>
        </Card>
      )}

      {queue === "contacts" && (
        <div className="grid gap-3 md:grid-cols-2">
          <SourceCard
            title={`Survivor: ${detail.candidate.primary_label}`}
            links={detail.primary.source_links}
            observed={(detail.primary.observed_fields ?? {}) as Record<string, unknown>}
            curated={(detail.primary.curated_fields ?? {}) as Record<string, unknown>}
            testId="entity-detail-primary"
          />
          <SourceCard
            title={`Candidate: ${detail.candidate.candidate_label ?? ""}`}
            links={detail.candidate_side.source_links}
            observed={(detail.candidate_side.observed_fields ?? {}) as Record<string, unknown>}
            curated={(detail.candidate_side.curated_fields ?? {}) as Record<string, unknown>}
            testId="entity-detail-candidate"
          />
          <Card className="p-4 md:col-span-2" data-testid="entity-detail-fields">
            <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
              Fields
            </span>
            {detail.fields.map((f) => (
              <p key={f.key} className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                {`${f.key}: ${fieldPreview(f.primary_curated ?? f.primary_observed)} ← ${fieldPreview(f.candidate_curated ?? f.candidate_observed)}`}
              </p>
            ))}
          </Card>
        </div>
      )}

      {queue === "clients" && (
        <Card className="grid gap-1 p-4" data-testid="entity-detail-reconcile">
          <span className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Vendor organization
          </span>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            {`${c.source_system ?? ""} · org ${c.vendor_org_id ?? ""} · "${c.vendor_label ?? ""}"`}
          </p>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Accepting records the operator mapping in the durable org map
            (audited); the connectors resolve against it from the next pull.
            Reconciliation is never auto-applied.
          </p>
        </Card>
      )}
    </div>
  );
}
