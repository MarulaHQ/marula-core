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

import { rejectionText } from "../../lib/error-copy";
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
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { LedgerSurface, type LedgerColumn } from "../../components/assets-stock/ledger-surface";
import { LedgerRow } from "../../components/assets-stock/ledger-row";
import { csvFileName, downloadCsv } from "../../components/assets-stock/csv";
import { NeutralMark, ReturnedMark } from "../../components/assets-stock/ledger-marks";
import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import type { FilterClause } from "@marula-core/ui";

type CandidateView = components["schemas"]["CandidateView"];
type CiMerge = components["schemas"]["CiMerge"];
type ConfigItemView = components["schemas"]["ConfigItemView"];

/**
 * The identity keys that make one device one record, as `key: value` pairs.
 *
 * `identity_keys` is an OBJECT keyed by identity kind, not an array — casting it
 * to `string[]` and calling `.join()` type-checked and then threw the moment a
 * row rendered. The pack's Config-items view shows these pairs, which is also
 * the only readable form: "okta:user-9f2" says which identity matched.
 */
function identityKeysText(keys: unknown): string {
  if (keys == null || typeof keys !== "object") return "";
  const entries = Object.entries(keys as Record<string, unknown>)
    .filter(([, value]) => value != null && value !== "")
    .map(([kind, value]) => `${kind}: ${String(value)}`);
  return entries.join(", ");
}
type CiMergeStatus = components["schemas"]["CiMergeStatus"];

const CONFIG_VIEWS = [
  { id: "ledger", label: "Config items" },
  { id: "merge-queue", label: "Merge queue" },
  { id: "devices", label: "Devices" },
];

const CONFIG_COLUMNS: LedgerColumn[] = [
  { key: "item", label: "Item", required: true },
  { key: "tag", label: "Tag" },
  { key: "status", label: "Status" },
  { key: "identity", label: "Identity keys" },
  { key: "updated", label: "Updated" },
];

const CONFIG_FILTERS = [
  { key: "item", label: "Item" },
  { key: "tag", label: "Tag" },
  { key: "status", label: "Status" },
  { key: "identity", label: "Identity" },
];

/**
 * A config item's ledger state, in the ledger's own marks: an orphan waits
 * for a human (blush, with the word), an active item is healthy (green), and
 * a merged record is quiet — it is still queryable, so it must not look like
 * an error.
 */
function ItemStatusMark({
  status,
  testId,
}: {
  status: string;
  testId?: string;
}): ReactNode {
  if (status === "active") return <ReturnedMark label="Active" testId={testId} />;
  if (status === "orphan") return <NeutralMark label="Orphan · needs review" testId={testId} />;
  if (status === "merged") return <NeutralMark label="Merged" testId={testId} />;
  return <NeutralMark label={status} testId={testId} />;
}

const STATUS_TONE: Record<CiMergeStatus, "neutral" | "success" | "warning" | "danger"> = {
  pending: "warning",
  auto_merged: "success",
  accepted: "success",
  rejected: "neutral",
  deferred: "warning",
};

const ITEM_TONE: Record<string, "neutral" | "success" | "warning" | "danger"> = {
  active: "success",
  merged: "neutral",
  retired: "neutral",
  orphan: "warning",
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
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const router = useRouter();

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
        identityKeysText(i.identity_keys).toLowerCase().includes(q),
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
        `Could not ${accept ? "accept" : "reject"} candidate ${candidate.candidate_tag}: ${rejectionText(result, "rejected.")}`,
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
      setActionError(`Could not undo the merge: ${rejectionText(result, "rejected.")}`);
      return;
    }
    await refreshAll();
  }

  const rows = filteredItems.filter((item) => {
    for (const clause of clauses) {
      const value = (
        {
          item: item.name,
          tag: item.tag,
          status: item.status,
          identity: identityKeysText(item.identity_keys),
        }[clause.field] ?? ""
      ).toLowerCase();
      const want = (clause.value ?? "").toLowerCase();
      if (clause.op === "is_set" && value === "") return false;
      if (clause.op === "is_not_set" && value !== "") return false;
      if (clause.op === "contains" && !value.includes(want)) return false;
      if (clause.op === "eq" && value !== want) return false;
      if (clause.op === "ne" && value === want) return false;
    }
    return true;
  });

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

      {/* ---- merge queue: the review surface moved to the workbench ---- */}
      <section className="grid gap-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Merge queue
          </h2>
          <Link
            href="/inventory/workbench"
            className="rounded-full border px-3 py-1 text-[12px]"
            style={{ borderColor: "var(--m-accent)", color: "var(--m-text)" }}
            data-testid="open-workbench"
          >
            {pending.length > 0
              ? `Open merge workbench (${pending.length} pending)`
              : "Open merge workbench"}
          </Link>
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
            {pending.slice(0, 3).map((c) => (
              <Link
                key={c.id}
                href="/inventory/workbench"
                data-testid={`merge-candidate-${c.id}`}
                className="rounded-lg border p-3"
                style={{ borderColor: "var(--m-border)" }}
              >
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium">{c.primary_tag}</span>
                  <span style={{ color: "var(--m-text-muted)" }}>←</span>
                  <span className="text-sm font-medium">{c.candidate_tag}</span>
                  <StatusTag tone={c.confidence >= 0.9 ? "success" : "warning"}>
                    {`${Math.round(c.confidence * 100)}% confidence`}
                  </StatusTag>
                  <span className="truncate text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    {c.reason}
                  </span>
                </div>
              </Link>
            ))}
            {pending.length > 3 && (
              <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {`${pending.length - 3} more in the workbench…`}
              </p>
            )}
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

      <LedgerSurface
        surface="config-items"
        testId="config-items-ledger"
        title="Config items"
        subtitle="Devices mirrored from identity feeds, with the identity keys that make one device one record."
        shortcuts={[
          {
            key: "merge-queue",
            label: "In the merge queue",
            count: pending.length,
            tone: "blush",
            active: false,
            onClick: () => {
              window.location.href = "/inventory/workbench";
            },
          },
        ]}
        headerAction={
          <Link
            href="/inventory/workbench"
            className="m-ledger-bulk-action"
            data-testid="open-workbench"
          >
            {pending.length > 0
              ? `Open merge workbench (${pending.length} pending)`
              : "Open merge workbench"}
          </Link>
        }
        views={CONFIG_VIEWS}
        activeView="ledger"
        onViewChange={(id) => {
          if (id === "merge-queue") router.push("/inventory/workbench");
          if (id === "devices") router.push("/assets");
        }}
        columns={CONFIG_COLUMNS}
        filterFields={CONFIG_FILTERS}
        filterClauses={clauses}
        onFilterChange={setClauses}
        rowCount={rows.length}
        rowsShown={rows.length}
        onExport={(request) => {
          const stamp = new Date().toISOString().slice(0, 10);
          downloadCsv(
            csvFileName("config-items", stamp),
            request.columns,
            rows.map((item) => ({
              item: item.name,
              tag: item.tag,
              status: item.status,
              identity: identityKeysText(item.identity_keys),
              updated: item.updated_at,
            })),
          );
        }}
        selectedIds={selected}
        onToggleRow={(id, isSelected) =>
          setSelected((cur) => (isSelected ? [...cur, id] : cur.filter((row) => row !== id)))
        }
        onToggleAll={(isSelected) => setSelected(isSelected ? rows.map((i) => i.id) : [])}
        allSelected={rows.length > 0 && selected.length === rows.length}
        someSelected={selected.length > 0 && selected.length < rows.length}
        onClearSelection={() => setSelected([])}
        emptyMessage="No config items match yet. A feed or a person creates one."
        errorMessage={loadError}
        keyboardHints="A merged record stays queryable, marked not deleted"
      >
        {(columns) =>
          rows.map((item) => (
            <LedgerRow
              key={item.id}
              id={item.id}
              selected={selected.includes(item.id)}
              selectable
              onToggle={(rowId, isSelected) =>
                setSelected((cur) => (isSelected ? [...cur, rowId] : cur.filter((id) => id !== rowId)))
              }
              columns={columns}
              selectLabel={`Select ${item.name}`}
              testId={`inventory-row-${item.id}`}
              cells={{
                item: (
                  <span>
                    <span style={{ color: "var(--m-text)" }}>{item.name}</span>
                    {item.merged_into_id != null ? (
                      <span className="ml-1 text-xs" style={{ color: "var(--m-text-muted)" }}>
                        merged
                      </span>
                    ) : null}
                  </span>
                ),
                tag: <span className="font-mono text-xs">{item.tag}</span>,
                status: <ItemStatusMark status={item.status} testId={`inventory-status-${item.id}`} />,
                identity: (
                  <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                    {identityKeysText(item.identity_keys) || "—"}
                  </span>
                ),
                updated: (
                  <span className="font-mono text-xs tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                    {new Date(item.updated_at).toISOString().slice(0, 10)}
                  </span>
                ),
              }}
            />
          ))
        }
      </LedgerSurface>
      <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
        Merged records stay queryable (their row is marked, not deleted).
      </p>

    </div>
  );
}
