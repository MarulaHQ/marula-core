// /assets — the device ledger, the Marula data-surface template applied to
// the assets area (Wave 41 lane 9, design/qa-p41-ui-assets-stock).
//
// Purpose: the device ledger the pack's derivation guide routes here —
// "Config ledger with a merge queue side by side", with the templates fixed
// by DESIGN-NOTES-tickets §2 and §11 (Assets row): views Devices, Config
// items and Merge queue; default columns Asset, Client, Type, Warranty, Last
// seen; the attention shortcuts Warranty ending and Offline. The queue and
// the config-item ledger are their own routes, so those two views navigate
// rather than duplicating a ledger here — one surface, one set of rows.
//
// Non-purpose: no asset editing (the record page and settings own that), and
// no AI affordance of any kind on this surface (the pack keeps AI off the
// list; the merge queue is a human decision surface and carries no mark
// inside it).
"use client";

import { Button } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import {
  LedgerSurface,
  type LedgerBulkAction,
  type LedgerColumn,
} from "../../components/assets-stock/ledger-surface";
import { LedgerRow } from "../../components/assets-stock/ledger-row";
import { csvFileName, downloadCsv } from "../../components/assets-stock/csv";
import {
  ClockIcon,
  CopyIcon,
  DueSoonMark,
  NeutralMark,
  OfflineDotIcon,
  OpenIcon,
  ReturnedMark,
} from "../../components/assets-stock/ledger-marks";
import { api, useRequireAuth } from "../../lib/api";
import type { FilterClause } from "@marula-core/ui";

type Asset = components["schemas"]["DeviceAsset"];
type Client = components["schemas"]["Client"];
type Warranty = NonNullable<components["schemas"]["DeviceAsset"]["warranty"]>;

const SURFACE = "assets-devices";

/** Warranty is a BLUSH pill when it is about to end, never red (client record §3).
 * The horizon itself arrives on the payload (`DeviceAsset.warranty`, derived
 * once server-side through the shared domain rule) — this page composes the
 * label and owns the words, nothing more. It used to re-derive the horizon
 * from `purchase_date` + `useful_life_months`, which is how a ledger column
 * and the mobile found sheet end up disagreeing about which side of the
 * window an asset sits on. */
function warrantyMark(warranty: Warranty | null | undefined): { state: string; label: string } {
  if (warranty == null) return { state: "none", label: "Not set" };
  const days = Math.round(warranty.days_remaining);
  if (warranty.state === "ended") {
    return { state: "ended", label: `Warranty ended ${Math.abs(days)}d ago` };
  }
  if (warranty.state === "ending") {
    return { state: "ending", label: `Warranty ends in ${days}d` };
  }
  return { state: "active", label: `Until ${warranty.ends_on}` };
}

const COLUMNS: LedgerColumn[] = [
  { key: "asset", label: "Asset", required: true },
  { key: "client", label: "Client" },
  { key: "type", label: "Type" },
  { key: "status", label: "Status" },
  { key: "warranty", label: "Warranty" },
  { key: "last_seen", label: "Last seen" },
  { key: "site", label: "Site", hiddenByDefault: true },
  { key: "tag", label: "Tag", hiddenByDefault: true },
];

const FILTER_FIELDS = [
  { key: "name", label: "Asset" },
  { key: "tag", label: "Tag" },
  { key: "kind", label: "Type" },
  { key: "status", label: "Status" },
  { key: "client", label: "Client" },
];

/**
 * A device's status, in the pack's own vocabulary rather than one grey for all.
 *
 * A status column is read by tone before it is read by word, so "rented" and
 * "retired" cannot both be the same pill: in use is a live state (green), on
 * loan is attention (blush, it is out with someone), retired is quiet (grey),
 * and anything unrecognised stays neutral rather than borrowing a meaning.
 */
function AssetStatusMark({ status }: { status: string }): ReactNode {
  if (status === "active" || status === "in_use") return <ReturnedMark label={status} />;
  if (status === "rented" || status === "loaned") return <DueSoonMark label={status} />;
  if (status === "retired" || status === "archived") return <NeutralMark label={status} />;
  return <NeutralMark label={status} />;
}

const VIEWS = [
  { id: "devices", label: "Devices" },
  { id: "config-items", label: "Config items" },
  { id: "merge-queue", label: "Merge queue" },
];

function lastSeenPhrase(iso: string | null | undefined, now: Date): { text: string; offline: boolean } {
  if (iso == null || iso === "") return { text: "Never seen", offline: true };
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return { text: "Never seen", offline: true };
  const hours = Math.round((now.getTime() - at.getTime()) / 3_600_000);
  if (hours < 0) return { text: "Just now", offline: false };
  if (hours === 0) return { text: "Just now", offline: false };
  if (hours < 48) return { text: `${hours}h ago`, offline: false };
  return { text: `${Math.round(hours / 24)}d ago`, offline: hours >= 24 * 7 };
}

export default function AssetsPage() {
  const auth = useRequireAuth();
  const router = useRouter();

  const [q, setQ] = useState("");
  const [debouncedQ, setDebouncedQ] = useState("");
  const [assets, setAssets] = useState<Asset[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [now] = useState(() => new Date());

  useEffect(() => {
    const t = window.setTimeout(() => setDebouncedQ(q.trim()), 250);
    return () => window.clearTimeout(t);
  }, [q]);

  const refresh = useCallback(async (query: string) => {
    setAssets(null);
    setError(null);
    const result = await api.api.GET("/api/v1/assets", {
      params: { query: query.length > 0 ? { q: query } : {} },
    });
    if (result.error !== undefined || result.data === undefined) {
      setError("Couldn't load devices. Your filters and columns are saved, so nothing is lost.");
      setAssets([]);
      return;
    }
    setAssets([...result.data].sort((a, b) => a.name.localeCompare(b.name)));
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh(debouncedQ);
    void api.api.GET("/api/v1/clients").then(({ data }) => setClients(data ?? []));
  }, [auth.status, debouncedQ, refresh]);

  const clientName = useMemo(() => {
    const map = new Map(clients.map((client) => [client.id, client.name]));
    return (id: string | null | undefined): string | null =>
      id == null ? null : (map.get(id) ?? null);
  }, [clients]);

  const filtered = useMemo(() => {
    const text = debouncedQ.toLowerCase();
    return (assets ?? []).filter((asset) => {
      if (text.length > 0) {
        const haystack = [asset.name, asset.tag, asset.kind, asset.status]
          .join(" ")
          .toLowerCase();
        if (!haystack.includes(text)) return false;
      }
      for (const clause of clauses) {
        const name = clientName(asset.client_id);
        const value = (
          {
            name: asset.name,
            tag: asset.tag,
            kind: asset.kind,
            status: asset.status,
            client: name ?? "",
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
  }, [assets, debouncedQ, clauses, clientName]);

  const warrantyEnding = useMemo(
    () =>
      filtered.filter((asset) => {
        const mark = warrantyMark(asset.warranty);
        return mark.state === "ending";
      }).length,
    [filtered, now],
  );
  const offline = useMemo(
    () => filtered.filter((asset) => lastSeenPhrase(asset.updated_at, now).offline).length,
    [filtered, now],
  );

  const warrantyEndingOnly = clauses.some(
    (clause) => clause.field === "warranty" && clause.value === "ending",
  );
  const offlineOnly = clauses.some((clause) => clause.field === "last_seen" && clause.value === "offline");

  const rows = useMemo(
    () =>
      filtered.filter((asset) => {
        if (warrantyEndingOnly && warrantyMark(asset.warranty).state !== "ending") {
          return false;
        }
        if (offlineOnly && !lastSeenPhrase(asset.updated_at, now).offline) return false;
        return true;
      }),
    [filtered, warrantyEndingOnly, offlineOnly, now],
  );

  function toggleShortcut(key: "warranty" | "last_seen"): void {
    const field = key === "warranty" ? "warranty" : "last_seen";
    setClauses((cur) =>
      cur.some((clause) => clause.field === field)
        ? cur.filter((clause) => clause.field !== field)
        : [...cur, { id: field, field, op: "eq", value: key === "warranty" ? "ending" : "offline" }],
    );
  }

  const bulkActions: LedgerBulkAction[] = [
    {
      key: "open",
      label: "Open",
      icon: <OpenIcon />,
      onClick: () => {
        const first = selected[0];
        if (first != null) router.push(`/assets/${first}`);
      },
      disabled: selected.length !== 1,
      title: "Open the selected device",
    },
    {
      key: "copy-tag",
      label: "Copy tag",
      icon: <CopyIcon />,
      onClick: () => {
        const tags = selected
          .map((id) => (assets ?? []).find((asset) => asset.id === id)?.tag)
          .filter((tag): tag is string => tag != null);
        if (tags.length > 0) void navigator.clipboard.writeText(tags.join("\n"));
      },
      disabled: selected.length === 0,
    },
  ];

  if (auth.status !== "authed") return null;

  return (
    <LedgerSurface
      surface={SURFACE}
      testId="assets-ledger"
      title="Devices"
      subtitle="Every managed device in the org. Open one for its fields, links, vault and history."
      shortcuts={[
        {
          key: "warranty-ending",
          label: "Warranty ending",
          count: warrantyEnding,
          tone: "blush",
          active: warrantyEndingOnly,
          onClick: () => toggleShortcut("warranty"),
        },
        {
          key: "offline",
          label: "Offline",
          count: offline,
          tone: "gold",
          active: offlineOnly,
          onClick: () => toggleShortcut("last_seen"),
        },
      ]}
      headerAction={
        <Link
          href="/inventory"
          className="m-ledger-bulk-action"
          data-testid="assets-config-items-link"
        >
          Config items
        </Link>
      }
      views={VIEWS}
      activeView="devices"
      onViewChange={(id) => {
        if (id === "config-items") router.push("/inventory");
        if (id === "merge-queue") router.push("/inventory/workbench");
      }}
      columns={COLUMNS}
      filterFields={[...FILTER_FIELDS, { key: "warranty", label: "Warranty" }, { key: "last_seen", label: "Last seen" }]}
      filterClauses={clauses}
      onFilterChange={setClauses}
      rowCount={rows.length}
      rowsShown={rows.length}
      onExport={(request) => {
        const stamp = new Date().toISOString().slice(0, 10);
        downloadCsv(
          csvFileName("devices", stamp),
          request.columns,
          rows.map((asset) => ({
            asset: asset.name,
            client: clientName(asset.client_id) ?? "Internal org asset",
            type: asset.kind,
            status: asset.status,
            warranty: warrantyMark(asset.warranty).label,
            last_seen: lastSeenPhrase(asset.updated_at, now).text,
            site: asset.site_id ?? "",
            tag: asset.tag,
          })),
        );
      }}
      selectedIds={selected}
      onToggleRow={(id, isSelected) =>
        setSelected((cur) => (isSelected ? [...cur, id] : cur.filter((row) => row !== id)))
      }
      onToggleAll={(isSelected) =>
        setSelected(isSelected ? rows.map((asset) => asset.id) : [])
      }
      allSelected={rows.length > 0 && selected.length === rows.length}
      someSelected={selected.length > 0 && selected.length < rows.length}
      bulkActions={bulkActions}
      onClearSelection={() => setSelected([])}
      emptyMessage={
        debouncedQ.length > 0 || clauses.length > 0
          ? "No devices match these filters."
          : "No devices yet. Assets appear here once a feed or a person connects one."
      }
      emptyAction={
        debouncedQ.length > 0 || clauses.length > 0 ? (
          <Button
            variant="secondary"
            onClick={() => {
              setClauses([]);
              setQ("");
            }}
            data-testid="assets-clear-filters"
          >
            Clear filters
          </Button>
        ) : null
      }
      errorMessage={error}
      onRetry={() => void refresh(debouncedQ)}
    >
      {(columns) =>
        rows.map((asset) => {
          const warranty = warrantyMark(asset.warranty);
          const seen = lastSeenPhrase(asset.updated_at, now);
          return (
            <LedgerRow
              key={asset.id}
              id={asset.id}
              selected={selected.includes(asset.id)}
              selectable
              onToggle={(rowId, isSelected) =>
                setSelected((cur) =>
                  isSelected ? [...cur, rowId] : cur.filter((id) => id !== rowId),
                )
              }
              columns={columns}
              selectLabel={`Select ${asset.name}`}
              testId={`assets-row-${asset.id}`}
              cells={{
                asset: (
                  <Link
                    href={`/assets/${asset.id}`}
                    className="font-medium"
                    style={{ color: "var(--m-text)" }}
                    data-testid={`assets-open-${asset.id}`}
                  >
                    {asset.name}
                  </Link>
                ),
                client: (
                  <span style={{ color: "var(--m-text-secondary)" }}>
                    {clientName(asset.client_id) ?? "Internal org asset"}
                  </span>
                ),
                type: asset.kind,
                status: <AssetStatusMark status={asset.status} />,
                warranty:
                  warranty.state === "ending" ? (
                    <DueSoonMark label={warranty.label} testId={`assets-warranty-${asset.id}`} />
                  ) : warranty.state === "ended" ? (
                    <NeutralMark label={warranty.label} icon={<ClockIcon />} />
                  ) : (
                    <span style={{ color: "var(--m-text-secondary)" }}>{warranty.label}</span>
                  ),
                last_seen: (
                  <NeutralMark
                    label={seen.text}
                    icon={seen.offline ? <OfflineDotIcon /> : undefined}
                    testId={`assets-last-seen-${asset.id}`}
                  />
                ),
                site: (
                  <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                    {asset.site_id ?? "—"}
                  </span>
                ),
                tag: (
                  <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                    {asset.tag}
                  </span>
                ),
              }}
            />
          );
        })
      }
    </LedgerSurface>
  );
}
