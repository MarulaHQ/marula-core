// /contracts — the contract ledger, rebuilt against the Marula surface pack
// (Wave 41 lane 7 `p41-ui-clients`).
//
// DERIVATION-GUIDE: `/contracts` and `/contracts/[id]` **Derive** from TL and
// the KIT components, changing two things: the ledger carries a Status
// column, and renewals are BLUSH PILLS.
//
// The colour rule is the load-bearing one here, and it is absolute
// (`DESIGN-NOTES.md` §A2, and the lane's own handoff): **a contract nearing
// renewal is blush, never red.** Red in this product means one of three
// things — a genuine SLA breach, a P1, or a destructive action — and a term
// approaching its end is none of them. A term that has ALREADY elapsed is
// different: the contract has run past its end, which is a real commercial
// event, so that one is allowed red, and only once it is actually past.
// `renewalSignal` in lib/renewal-signals.ts is the single implementation of
// that rule, shared with the client record so the two can never disagree.
//
// Purpose: know what was promised, for how long, and when it renews
// (marula-surface-brief §5.8).
//
// Non-purpose: no contract authoring on this page beyond the create dialog
// (which the record also carries); no invoice runs.
"use client";

import {
  Badge,
  Button,
  ColumnChooser,
  DataToolbar,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  ExportControl,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
  ViewSwitch,
  SurfaceFooter,
  DensitySwitch,
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { rejectionText } from "../../lib/error-copy";
import { api, useRequireAuth } from "../../lib/api";
import { money } from "../../lib/format";
import { ErrorNote } from "../../components/knowledge/ledger";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { useDataSurfacePrefs } from "../../lib/data-surface-prefs";
import { formatPackDate, renewalSignal } from "../../lib/renewal-signals";

type Contract = components["schemas"]["Contract"];
type ContractType = components["schemas"]["ContractType"];
type ContractMutationResponse = components["schemas"]["ContractMutationResponse"];
type Client = components["schemas"]["Client"];

const TYPES: ContractType[] = [
  "recurring",
  "time_and_materials",
  "retainer",
  "fixed_fee",
  "per_unit",
  "per_incident",
];

const TYPE_LABEL: Record<string, string> = {
  recurring: "Recurring",
  time_and_materials: "Time and materials",
  retainer: "Retainer",
  fixed_fee: "Fixed fee",
  per_unit: "Per unit",
  per_incident: "Per incident",
};

const STATUS_LABEL: Record<string, string> = {
  draft: "Draft",
  active: "Active",
  ended: "Ended",
  cancelled: "Cancelled",
};

/** The pack's views: the ledger itself plus the two things an account manager
 * hunts for in it. */
const VIEWS = [
  { id: "all", label: "All" },
  { id: "active", label: "Active" },
  { id: "renewing", label: "Renewing soon" },
];

const SURFACE = "contracts.list";

type ColumnId = "number" | "name" | "client" | "type" | "status" | "term" | "amount";

const COLUMNS: { id: ColumnId; label: string; defaultVisible: boolean }[] = [
  { id: "number", label: "Number", defaultVisible: true },
  { id: "name", label: "Name", defaultVisible: true },
  { id: "client", label: "Client", defaultVisible: true },
  { id: "type", label: "Type", defaultVisible: true },
  { id: "status", label: "Status", defaultVisible: true },
  { id: "term", label: "Term", defaultVisible: true },
  { id: "amount", label: "Per period", defaultVisible: true },
];

function today(): string {
  return new Date().toISOString().slice(0, 10);
}

export default function ContractsPage() {
  const auth = useRequireAuth();
  const prefs = useDataSurfacePrefs();

  const [contracts, setContracts] = useState<Contract[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [view, setView] = useState("all");
  const [typeFilter, setTypeFilter] = useState<ContractType | "all">("all");
  const [clientFilter, setClientFilter] = useState<string>("all");
  const [query, setQuery] = useState("");

  const [createOpen, setCreateOpen] = useState(false);
  const [number, setNumber] = useState("");
  const [name, setName] = useState("");
  const [clientId, setClientId] = useState<string>("");
  const [type, setType] = useState<ContractType>("recurring");
  const [currency, setCurrency] = useState("USD");
  const [effectiveFrom, setEffectiveFrom] = useState(today());
  const [effectiveUntil, setEffectiveUntil] = useState("");
  const [amount, setAmount] = useState("");
  const [frequency, setFrequency] = useState<string>("monthly");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const now = Date.now();

  const refresh = useCallback(async () => {
    const [c, cl] = await Promise.all([
      api.api.GET("/api/v1/contracts"),
      api.api.GET("/api/v1/clients"),
    ]);
    if (c.error !== undefined || c.data === undefined) {
      setLoadError("Failed to load contracts.");
      setContracts([]);
    } else {
      setContracts(c.data);
      setLoadError(null);
    }
    if (cl.error === undefined && cl.data !== undefined) setClients(cl.data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const clientName = useMemo(() => {
    const map = new Map(clients.map((c) => [c.id, c.name]));
    return (id: string) => map.get(id) ?? "—";
  }, [clients]);

  const posture = useMemo(() => {
    const list = contracts ?? [];
    return {
      active: list.filter((c) => c.status === "active").length,
      draft: list.filter((c) => c.status === "draft").length,
      // Every contract whose term is inside the renewal window, and every
      // contract whose term has already elapsed. One definition, used by the
      // pill and the view that filters on it.
      renewing: list.filter((c) => {
        const s = renewalSignal(c.effective_until, now);
        return s !== null && s.days >= 0;
      }).length,
      elapsed: list.filter((c) => {
        const s = renewalSignal(c.effective_until, now);
        return s !== null && s.days < 0;
      }).length,
      book: moneyOf(list),
    };
  }, [contracts, now]);

  const filtered = useMemo(() => {
    if (contracts === null) return null;
    const q = query.trim().toLowerCase();
    return contracts.filter((c) => {
      if (view === "active" && c.status !== "active") return false;
      if (view === "renewing") {
        const s = renewalSignal(c.effective_until, now);
        if (s === null || s.days < 0) return false;
      }
      if (typeFilter !== "all" && c.contract_type !== typeFilter) return false;
      if (clientFilter !== "all" && c.client_id !== clientFilter) return false;
      if (q === "") return true;
      return (
        c.number.toLowerCase().includes(q) ||
        c.name.toLowerCase().includes(q) ||
        clientName(c.client_id).toLowerCase().includes(q)
      );
    });
  }, [contracts, view, typeFilter, clientFilter, query, clientName, now]);

  const saved = prefs.view(SURFACE);
  const [visibleColumns, setVisibleColumns] = useState<ColumnId[]>(
    COLUMNS.map((c) => c.id),
  );
  useEffect(() => {
    if (!prefs.ready) return;
    const savedIds = saved.columns.map((c) => c.key as ColumnId);
    if (savedIds.length === 0) return;
    const known = COLUMNS.filter((c) => savedIds.includes(c.id));
    if (known.length > 0) setVisibleColumns(known.map((c) => c.id));
  }, [prefs.ready, saved.columns]);

  const visible = useMemo(
    () => COLUMNS.filter((c) => visibleColumns.includes(c.id)),
    [visibleColumns],
  );

  const onColumnsChange = (next: { key: string; visible: boolean }[]) => {
    const ordered = next
      .filter((c) => c.visible)
      .map((c) => c.key as ColumnId)
      .filter((id) => COLUMNS.some((c) => c.id === id));
    setVisibleColumns(ordered);
    void prefs.save(SURFACE, {
      ...saved,
      columns: next.map((c) => ({ key: c.key, visible: c.visible })),
    });
  };

  const createDisabled =
    creating || number.trim() === "" || name.trim() === "" || clientId === "";

  const createContract = async () => {
    setCreating(true);
    setCreateError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateContractRequest"],
      ContractMutationResponse
    >({
      describe: "Create contract",
      entityId: `pending-contract:${number}`,
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        number: number.trim(),
        name: name.trim(),
        client_id: clientId,
        contract_type: type,
        currency: currency.trim() === "" ? "USD" : currency.trim(),
        effective_from: effectiveFrom,
        effective_until: effectiveUntil === "" ? null : effectiveUntil,
        amount_minor: amount.trim() === "" ? null : Math.round(Number(amount) * 100),
        billing_frequency:
          type === "recurring" || type === "retainer"
            ? (frequency as components["schemas"]["BillingFrequency"])
            : null,
      }),
      request: (client, body) => client.POST("/api/v1/contracts", { body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setCreating(false);
    if (!result.ok) {
      setCreateError(rejectionText(result, "The contract was rejected. Nothing was saved."));
      return;
    }
    setCreateOpen(false);
    setNumber("");
    setName("");
    setClientId("");
    setAmount("");
    setEffectiveUntil("");
    void refresh();
  };

  if (auth.status !== "authed") return null;

  const cell = (id: ColumnId) => visibleColumns.includes(id);

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-4 p-4 md:p-6">
      <SurfaceHeader
        title="Contracts"
        meta={
          <div className="flex flex-wrap items-center gap-2">
          <StatPill label="active" count={posture.active} active={view === "active"} dot="var(--m-canopy)" onToggle={() => setView(view === "active" ? "all" : "active")} />
          <StatPill
          label="renewing soon"
          count={posture.renewing}
          active={view === "renewing"}
          dot="var(--m-blush)"
          onToggle={() => setView(view === "renewing" ? "all" : "renewing")}
          />
          <StatPill label="draft" count={posture.draft} active={false} dot="var(--m-input)" onToggle={() => undefined} />
          {posture.elapsed > 0 ? (
          <StatPill label="term elapsed" count={posture.elapsed} active={false} dot="var(--m-danger)" onToggle={() => undefined} />
          ) : null}
          </div>
        }
        actions={
          <Dialog
          open={createOpen}
          onOpenChange={(open) => {
          setCreateOpen(open);
          if (!open) setCreateError(null);
          }}
          >
          <DialogTrigger asChild>
          <Button variant="primary" data-testid="new-contract">
          <PlusIcon />
          New contract
          </Button>
          </DialogTrigger>
          <DialogContent
          title="New contract"
          description="A contract is the commercial agreement an invoice run bills against. Draft contracts preview and are refused by invoice runs."
          >
          <div className="grid gap-3 p-4">
          <div className="grid grid-cols-2 gap-3">
          <div>
          <Label htmlFor="ct-number">Number</Label>
          <Input id="ct-number" value={number} onChange={(e) => setNumber(e.target.value)} placeholder="CTR-2026-04" />
          </div>
          <div>
          <Label htmlFor="ct-name">Name</Label>
          <Input id="ct-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Managed Gold" />
          </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
          <div>
          <Label htmlFor="ct-client">Client</Label>
          <Select value={clientId} onValueChange={setClientId}>
          <SelectTrigger id="ct-client" className="w-full">
          <SelectValue placeholder="Choose a client" />
          </SelectTrigger>
          <SelectContent>
          {clients.map((c) => (
          <SelectItem key={c.id} value={c.id}>
          {c.name}
          </SelectItem>
          ))}
          </SelectContent>
          </Select>
          </div>
          <div>
          <Label htmlFor="ct-type">Type</Label>
          <Select value={type} onValueChange={(v) => setType(v as ContractType)}>
          <SelectTrigger id="ct-type" className="w-full">
          <SelectValue />
          </SelectTrigger>
          <SelectContent>
          {TYPES.map((t) => (
          <SelectItem key={t} value={t}>
          {TYPE_LABEL[t]}
          </SelectItem>
          ))}
          </SelectContent>
          </Select>
          </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
          <div>
          <Label htmlFor="ct-from">Effective from</Label>
          <Input id="ct-from" type="date" value={effectiveFrom} onChange={(e) => setEffectiveFrom(e.target.value)} />
          </div>
          <div>
          <Label htmlFor="ct-until">Effective until</Label>
          <Input id="ct-until" type="date" value={effectiveUntil} onChange={(e) => setEffectiveUntil(e.target.value)} />
          <FieldHint>Empty means open-ended: there is nothing to renew.</FieldHint>
          </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
          <div>
          <Label htmlFor="ct-amount">Per-period price</Label>
          <Input id="ct-amount" type="number" min={0} step="0.01" value={amount} onChange={(e) => setAmount(e.target.value)} placeholder="Optional" />
          </div>
          <div>
          <Label htmlFor="ct-freq">Billing frequency</Label>
          <Select value={frequency} onValueChange={setFrequency}>
          <SelectTrigger id="ct-freq" className="w-full">
          <SelectValue />
          </SelectTrigger>
          <SelectContent>
          {["weekly", "monthly", "quarterly", "semiannual", "annual"].map((f) => (
          <SelectItem key={f} value={f}>
          {f}
          </SelectItem>
          ))}
          </SelectContent>
          </Select>
          <FieldHint>Applies to recurring and retainer contracts.</FieldHint>
          </div>
          </div>
          {createError !== null ? (
          <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {createError}
          </p>
          ) : null}
          <DialogActions>
          <Button variant="ghost" onClick={() => setCreateOpen(false)}>
          Cancel
          </Button>
          <Button onClick={() => void createContract()} disabled={createDisabled}>
          {creating ? <Spinner /> : null} Create contract
          </Button>
          </DialogActions>
          </div>
          </DialogContent>
          </Dialog>
        }
      />

      {/* The toolbar's vertical padding is `var(--m-card-padding)`, and the
          pack's toolbar sits flush under the page head. The primitive takes no
          `style`, so the override rides a wrapper that owns the custom
          property — that keeps the fix on this surface instead of
          restyling a component every data surface in the app shares. */}
      <div style={{ "--m-card-padding": "0px" } as React.CSSProperties}>
      <DataToolbar
        views={<ViewSwitch views={VIEWS} activeId={view} onChange={setView} label="Contract views" />}
        filters={
          <>
            <div className="w-52">
              {/* Widths ride wrapper divs, not the controls: `.m-input` and
                  `.m-select-trigger` are unlayered `width: 100%` rules that
                  outrank a Tailwind utility on the element itself. */}
              <Input
                aria-label="Search contracts"
                placeholder="Search number, name or client…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <div className="w-40">
              <Select value={typeFilter} onValueChange={(v) => setTypeFilter(v as ContractType | "all")}>
                <SelectTrigger aria-label="Filter by type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All types</SelectItem>
                  {TYPES.map((t) => (
                    <SelectItem key={t} value={t}>
                      {TYPE_LABEL[t]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="w-44">
              <Select value={clientFilter} onValueChange={setClientFilter}>
                <SelectTrigger aria-label="Filter by client">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All clients</SelectItem>
                  {clients.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </>
        }
        columns={
          <ColumnChooser
            columns={COLUMNS.map((c) => ({
              key: c.id,
              label: c.label,
              visible: visibleColumns.includes(c.id),
            }))}
            onChange={onColumnsChange}
          />
        }
        export={
          <ExportControl
            columns={visible.map((c) => c.label)}
            rowCount={filtered === null ? 0 : filtered.length}
            onExport={() => {}}
          />
        }
      />
      </div>

      <div
        className="overflow-hidden rounded-2xl border"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
        {loadError !== null ? (
          <div className="p-4">
            <ErrorNote>{loadError}</ErrorNote>
          </div>
        ) : filtered === null ? (
          <div className="p-4">
            <Spinner />
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No contracts match this view. A contract is what an invoice run bills against." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                {cell("number") ? <Th>Number</Th> : null}
                {cell("name") ? <Th>Name</Th> : null}
                {cell("client") ? <Th>Client</Th> : null}
                {cell("type") ? <Th>Type</Th> : null}
                {cell("status") ? <Th>Status</Th> : null}
                {cell("term") ? <Th>Term</Th> : null}
                {cell("amount") ? <Th className="text-right">Per period</Th> : null}
              </tr>
            </thead>
            <tbody>
              {filtered.map((contract, i) => {
                const signal = renewalSignal(contract.effective_until, now);
                return (
                  <Tr
                    key={contract.id}
                    className="m-entity-row"
                    style={{ animationDelay: `${Math.min(i * 20, 160)}ms` }}
                    data-testid={`contract-row-${contract.id}`}
                  >
                    {cell("number") ? (
                      <Td>
                        <Link
                          href={`/contracts/${contract.id}`}
                          className="text-[13px] underline decoration-transparent underline-offset-2 hover:decoration-[var(--m-border-strong)]"
                        >
                          {contract.number}
                        </Link>
                      </Td>
                    ) : null}
                    {cell("name") ? <Td className="font-medium">{contract.name}</Td> : null}
                    {cell("client") ? (
                      <Td>
                        <Link href={`/clients/${contract.client_id}`} className="text-[13px] underline underline-offset-2">
                          {clientName(contract.client_id)}
                        </Link>
                      </Td>
                    ) : null}
                    {cell("type") ? (
                      <Td>
                        <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                          {TYPE_LABEL[contract.contract_type]}
                        </span>
                      </Td>
                    ) : null}
                    {cell("status") ? (
                      <Td>
                        <Badge tone={statusTone(contract.status)}>
                          {STATUS_LABEL[contract.status] ?? contract.status}
                        </Badge>
                      </Td>
                    ) : null}
                    {cell("term") ? (
                      <Td>
                        <span className="flex flex-wrap items-center gap-2">
                          <span className="text-[13px] tabular-nums">
                            {formatPackDate(contract.effective_from) ?? "—"}
                            {" → "}
                            {contract.effective_until != null && contract.effective_until !== ""
                              ? formatPackDate(contract.effective_until)
                              : "open"}
                          </span>
                          {/* THE rule: a renewal inside its window is BLUSH.
                              Red is only ever a breach that already happened,
                              which for a term means it is already past. */}
                          {signal !== null ? (
                            <Badge tone={signal.tone === "danger" ? "danger" : "due"}>
                              <span className="inline-flex items-center gap-1">
                                <ClockIcon />
                                {signal.label}
                              </span>
                            </Badge>
                          ) : null}
                        </span>
                      </Td>
                    ) : null}
                    {cell("amount") ? (
                      <Td className="text-right text-[13px] font-semibold tabular-nums">
                        {contract.amount_minor != null ? (
                          <>
                            {money(contract.amount_minor, contract.currency)}
                            <span className="ml-1 font-normal" style={{ color: "var(--m-text-muted)" }}>
                              / {contract.billing_frequency ?? "period"}
                            </span>
                          </>
                        ) : (
                          <span style={{ color: "var(--m-text-muted)" }}>—</span>
                        )}
                      </Td>
                    ) : null}
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </div>

      <SurfaceFooter
        shown={filtered?.length ?? 0}
        total={
          filtered !== null && contracts !== null && filtered.length !== contracts.length
            ? contracts.length
            : undefined
        }
        noun="contract"
        density={<DensitySwitch value={prefs.density} onChange={(step) => void prefs.setDensity(step)} variant="icons" label="Row density" />}
        trailing={<span className="tabular-nums">Booked per period: {posture.book}</span>}
      />
    </div>
  );
}

/** Header stat pill, as on the clients and projects ledgers. */
function StatPill({
  label,
  count,
  active,
  dot,
  onToggle,
}: {
  label: string;
  count: number;
  active: boolean;
  dot: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onToggle}
      data-testid={`contracts-stat-${label.replaceAll(" ", "-")}`}
      className="inline-flex cursor-pointer items-center gap-2 rounded-full border px-3 py-1.5 text-[13px] font-semibold"
      style={{
        background: active ? "var(--m-accent)" : "transparent",
        color: active ? "var(--m-on-accent)" : "var(--m-text)",
        borderColor: active ? "transparent" : "var(--m-border)",
      }}
    >
      <span className="h-2 w-2 rounded-full" style={{ background: dot }} aria-hidden="true" />
      {label}
      <span className="tabular-nums">{count}</span>
    </button>
  );
}

/** The booked value per period across live contracts, or an honest "mixed
 * currencies" when the book spans more than one. */
function moneyOf(list: Contract[]): string {
  const priced = list
    .filter((c) => c.status === "active" && c.amount_minor != null)
    .map((c) => ({ currency: c.currency, amount: c.amount_minor as number }));
  if (priced.length === 0) return "—";
  const currencies = [...new Set(priced.map((p) => p.currency))];
  if (currencies.length > 1) return "mixed currencies";
  const total = priced.reduce((acc, p) => acc + p.amount, 0);
  return money(total, currencies[0] as string);
}

function statusTone(status: string): "success" | "warning" | "danger" | "neutral" {
  switch (status) {
    case "active":
      return "success";
    case "draft":
      return "warning";
    case "cancelled":
      return "danger";
    default:
      return "neutral";
  }
}

function ClockIcon() {
  return <Icon name="clock" size={13} />;
}

function PlusIcon() {
  return <Icon name="plus" size={16} />;
}