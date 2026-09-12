// /inventory — config-item inventory + the CI merge queue (Wave 12 lane 3
// backends, migration 0061).
//
// Purpose: two faces of the same inventory: (a) the config-item ledger
// (devices mirrored from feeds) with their identity keys and source links,
// and (b) the CI merge queue — reviewable ambiguous matches with VISIBLE
// evidence (confidence, reason, identity signals), one-click accept →
// audited fold summary, and undo via split. Auto-merged and decided
// candidates read their ledger state honestly.
//
// Non-purpose: no CI editing here (curated fields live on the asset-side
// documents; the merge queue only merges/splits records).
"use client";

import {
  Button,
  Card,
  EmptyState,
  Spinner,
  StatusTag,
  Table,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";

type CandidateView = components["schemas"]["CandidateView"];
type CiMerge = components["schemas"]["CiMerge"];
type ConfigItemView = components["schemas"]["ConfigItemView"];
type CiMergeStatus = components["schemas"]["CiMergeStatus"];

const STATUS_TONE: Record<CiMergeStatus, "neutral" | "success" | "warning" | "danger"> = {
  pending: "warning",
  auto_merged: "success",
  accepted: "success",
  rejected: "neutral",
};

type Signal = Record<string, unknown>;

/** Evidence is `{ signals: [<loose key/value objects>] }` (CI3). */
function evidenceSignals(evidence: unknown): Signal[] {
  if (evidence === null || typeof evidence !== "object") return [];
  const signals = (evidence as { signals?: unknown }).signals;
  if (!Array.isArray(signals)) return [];
  return signals.filter((s): s is Signal => s !== null && typeof s === "object");
}

function signalText(s: Signal): string {
  return Object.entries(s)
    .map(([k, v]) => {
      if (Array.isArray(v)) return `${k.replace(/_/g, " ")}: ${v.map(String).join(", ")}`;
      return `${k.replace(/_/g, " ")}: ${String(v)}`;
    })
    .join(" · ");
}

export default function InventoryPage(): ReactNode {
  const auth = useRequireAuth();
  const [items, setItems] = useState<ConfigItemView[] | null>(null);
  const [candidates, setCandidates] = useState<CandidateView[] | null>(null);
  const [merges, setMerges] = useState<CiMerge[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const [itemRes, candRes] = await Promise.all([
        api.api.GET("/api/v1/config-items"),
        api.api.GET("/api/v1/config-items/merge-candidates"),
      ]);
      if (itemRes.data === undefined || candRes.data === undefined) {
        setLoadError("Could not load inventory — try reloading.");
        return;
      }
      setItems(itemRes.data ?? []);
      const cands = candRes.data ?? [];
      setCandidates(cands);
      // The merges ledger is per-item; reconstruct the org log from the
      // decided candidates' primary items (bounded so the page stays cheap).
      const decided = cands.filter((c) => c.status === "auto_merged" || c.status === "accepted");
      const primaryIds = [...new Set(decided.map((c) => c.primary_config_item_id))].slice(0, 5);
      const logs = await Promise.all(
        primaryIds.map((id) =>
          api.api
            .GET("/api/v1/config-items/{id}/merges", { params: { path: { id } } })
            .then(({ data }) => data ?? [])
            .catch(() => [] as CiMerge[]),
        ),
      );
      const seen = new Set<string>();
      setMerges(logs.flat().filter((m) => (seen.has(m.id) ? false : (seen.add(m.id), true))));
    })();
  }, [auth.status]);

  const filteredItems = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (items ?? []).filter(
      (i) =>
        q === "" ||
        i.name.toLowerCase().includes(q) ||
        i.tag.toLowerCase().includes(q) ||
        (i.identity_keys as string[] | null ?? []).some((k) => k.toLowerCase().includes(q)),
    );
  }, [items, query]);

  const pending = useMemo(
    () => (candidates ?? []).filter((c) => c.status === "pending"),
    [candidates],
  );

  async function refreshAll(): Promise<void> {
    const [itemRes, candRes] = await Promise.all([
      api.api.GET("/api/v1/config-items"),
      api.api.GET("/api/v1/config-items/merge-candidates"),
    ]);
    if (itemRes.data !== undefined) setItems(itemRes.data ?? []);
    if (candRes.data !== undefined) setCandidates(candRes.data ?? []);
  }

  async function decide(candidate: CandidateView, accept: boolean): Promise<void> {
    setActionError(null);
    setBusyId(candidate.id);
    const optimisticFold = () => {
      setCandidates((cur) =>
        (cur ?? []).map((c) =>
          c.id === candidate.id
            ? {
                ...c,
                status: accept ? ("accepted" as CiMergeStatus) : ("rejected" as CiMergeStatus),
                decided_at: new Date().toISOString(),
              }
            : c,
        ),
      );
      if (accept) {
        setItems((cur) => (cur ?? []).filter((i) => i.id !== candidate.candidate_config_item_id));
      }
    };
    const revert = () => {
      setCandidates((cur) =>
        (cur ?? []).map((c) => (c.id === candidate.id ? { ...c, status: candidate.status, decided_at: null } : c)),
      );
      if (accept) {
        void refreshAll();
      } else {
        setItems((cur) => (cur ?? []));
      }
    };
    const result = await runEnvelopeMutation({
      describe: accept ? "Accept merge" : "Reject candidate",
      entityId: candidate.id,
      buildBody: (base) => ({ ...base }),
      optimistic: optimisticFold,
      revert,
      request: (client) =>
        client.POST(
          accept
            ? "/api/v1/config-items/merge-candidates/{id}/accept"
            : "/api/v1/config-items/merge-candidates/{id}/reject",
          {
            params: { path: { id: candidate.id } },
          },
        ),
    });
    setBusyId(null);
    if (result.ok === false) {
      setActionError(
        `Could not ${accept ? "accept" : "reject"} candidate ${candidate.candidate_tag}: ${result.message ?? "rejected."}`,
      );
      return;
    }
    await refreshAll();
  }

  async function splitMerge(merge: CiMerge): Promise<void> {
    setActionError(null);
    setBusyId(merge.id);
    const result = await runEnvelopeMutation({
      describe: "Undo merge (split)",
      entityId: merge.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => {
        setMerges((cur) =>
          (cur ?? []).map((m) =>
            m.id === merge.id ? { ...m, status: "split", undone_at: new Date().toISOString() } : m,
          ),
        );
      },
      revert: () => {
        setMerges((cur) =>
          (cur ?? []).map((m) =>
            m.id === merge.id
              ? { ...m, status: merge.status, undone_at: merge.undone_at ?? null, undone_by: merge.undone_by ?? null }
              : m,
          ),
        );
      },
      request: (client) =>
        client.POST("/api/v1/config-items/merges/{id}/split", {
          params: { path: { id: merge.id } },
        }),
    });
    setBusyId(null);
    if (result.ok === false) {
      setActionError(`Could not undo the merge: ${result.message ?? "rejected."}`);
      return;
    }
    await refreshAll();
  }

  if (items === null && loadError === null) {
    return <Spinner />;
  }
  if (loadError !== null) {
    return <EmptyState message={loadError} />;
  }

  function tagFor(itemId: string): string {
    return (items ?? []).find((i) => i.id === itemId)?.tag ?? itemId.slice(0, 8);
  }

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <header>
        <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
          Inventory
        </h1>
        <p className="mt-0.5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Config items mirrored from identity feeds, with the merge queue that
          keeps the ledger one-device-one-record.
        </p>
      </header>

      {/* ---- merge queue ---- */}
      <section className="grid gap-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Merge queue
          </h2>
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }} data-testid="merge-queue-count">
            {pending.length} pending
          </span>
        </div>
        {actionError !== null && (
          <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="merge-action-error">
            {actionError}
          </p>
        )}
        {candidates === null ? (
          <Spinner />
        ) : pending.length === 0 ? (
          <EmptyState message="No pending merge candidates — the identity matcher found nothing ambiguous." />
        ) : (
          <div className="grid gap-2">
            {pending.map((c) => (
              <Card key={c.id} className="grid gap-2 p-4" data-testid={`merge-candidate-${c.id}`}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium">{c.primary_tag}</span>
                  <span style={{ color: "var(--m-text-muted)" }}>←</span>
                  <span className="text-sm font-medium">{c.candidate_tag}</span>
                  <StatusTag tone={c.confidence >= 0.9 ? "success" : "warning"}>
                    {`${Math.round(c.confidence * 100)}% confidence`}
                  </StatusTag>
                  <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    {c.reason}
                  </span>
                </div>
                <ul className="grid gap-0.5 text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                  {evidenceSignals(c.evidence).map((sig, i) => (
                    <li key={i} data-testid={`merge-evidence-${c.id}-${i}`}>
                      {signalText(sig)}
                    </li>
                  ))}
                  {evidenceSignals(c.evidence).length === 0 && (
                    <li style={{ color: "var(--m-text-muted)" }}>
                      Evidence detail not available for this candidate.
                    </li>
                  )}
                </ul>
                <div className="flex flex-wrap gap-2">
                  <Button size="sm" disabled={busyId === c.id} onClick={() => void decide(c, true)} data-testid={`merge-accept-${c.id}`}>
                    Accept merge
                  </Button>
                  <Button size="sm" variant="secondary" disabled={busyId === c.id} onClick={() => void decide(c, false)} data-testid={`merge-reject-${c.id}`}>
                    Keep separate
                  </Button>
                </div>
              </Card>
            ))}
          </div>
        )}
      </section>

      {/* ---- merged ledger + undo ---- */}
      {merges !== null && merges.length > 0 && (
        <section className="grid gap-2">
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Merge ledger
          </h2>
          <Card className="overflow-x-auto p-0">
            <Table>
              <thead>
                <tr>
                  <Th>Survivor</Th>
                  <Th>Absorbed</Th>
                  <Th>Status</Th>
                  <Th>Merged</Th>
                  <Th>Fold summary</Th>
                  <Th>Actions</Th>
                </tr>
              </thead>
              <tbody>
                {merges.map((m) => (
                  <Tr key={m.id}>
                    <Td>{tagFor(m.primary_config_item_id)}</Td>
                    <Td>{tagFor(m.merged_config_item_id)}</Td>
                    <Td>
                      <StatusTag tone={m.status === "split" ? "warning" : "success"}>{m.status}</StatusTag>
                    </Td>
                    <Td>{new Date(m.merged_at).toLocaleString()}</Td>
                    <Td>
                      {(() => {
                        const f = (m.moved ?? null) as
                          | { comments_folded?: number; time_minutes_folded?: number; cost_lines_folded?: number; expenses_folded?: number; children_relinked?: number }
                          | null;
                        if (f === null) return "—";
                        const parts: string[] = [];
                        if ((f.comments_folded ?? 0) > 0) parts.push(`${f.comments_folded} comments`);
                        if ((f.time_minutes_folded ?? 0) > 0) parts.push(`${f.time_minutes_folded} min`);
                        if ((f.cost_lines_folded ?? 0) > 0) parts.push(`${f.cost_lines_folded} cost lines`);
                        if ((f.children_relinked ?? 0) > 0) parts.push(`${f.children_relinked} children relinked`);
                        return parts.length > 0 ? parts.join(", ") : "no records moved";
                      })()}
                    </Td>
                    <Td>
                      {m.status !== "split" && m.undone_at === null && m.undone_by === null && (
                        <Button size="sm" variant="secondary" disabled={busyId === m.id} onClick={() => void splitMerge(m)}>
                          Undo (split)
                        </Button>
                      )}
                    </Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          </Card>
        </section>
      )}

      {/* ---- config-item ledger ---- */}
      <section className="grid gap-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Config items
          </h2>
          <input
            className="rounded-[2px] border px-2 py-1 text-[13px]"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text)", maxWidth: 320 }}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search name, tag, identity…"
            aria-label="Search config items"
            data-testid="inventory-search"
          />
        </div>
        <Card className="overflow-x-auto p-0">
          <Table data-testid="inventory-table">
            <thead>
              <tr>
                <Th>Name</Th>
                <Th>Tag</Th>
                <Th>Status</Th>
                <Th>Identity keys</Th>
                <Th>Updated</Th>
              </tr>
            </thead>
            <tbody>
              {filteredItems.map((i) => (
                <Tr key={i.id} data-testid={`inventory-row-${i.id}`}>
                  <Td>
                    <span className="font-medium">{i.name}</span>
                    {i.merged_into_id !== null && i.merged_into_id !== undefined && (
                      <span className="ml-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                        merged → {i.merged_into_id.slice(0, 8)}
                      </span>
                    )}
                  </Td>
                  <Td className="font-mono text-[12px]">{i.tag}</Td>
                  <Td>{i.status}</Td>
                  <Td className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    {(() => {
                      const keys = (i.identity_keys as string[] | null) ?? [];
                      return keys.length > 0 ? keys.join(", ") : "—";
                    })()}
                  </Td>
                  <Td>{new Date(i.updated_at).toLocaleString()}</Td>
                </Tr>
              ))}
              {filteredItems.length === 0 && (
                <Tr>
                  <Td>
                    <EmptyState message="No config items match yet." />
                  </Td>
                </Tr>
              )}
            </tbody>
          </Table>
        </Card>
        <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          Merged records stay queryable (their row is marked, not deleted).
        </p>
      </section>
    </div>
  );
}
