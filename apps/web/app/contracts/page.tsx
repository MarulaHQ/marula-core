// /contracts — staff commercial agreements (Workspace-core UI, T3).
//
// Purpose: the contract list with client filter and the gated create
// dialog covering all four agreement shapes (recurring / T&M / retainer /
// fixed-fee), including the T&M rate card and fixed-fee schedule specs
// the create endpoint accepts in one envelope. Links into
// /contracts/[id] for full editing.
// Non-purpose: no invoice issuing here (that is /billing's job; detail
// pages link to the per-contract billing preview).
"use client";

import { rejectionText } from "../../lib/error-copy";
import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { money } from "../../lib/format";
import { runEnvelopeMutation } from "../../lib/envelope-write";
import { contractStatusTone, termSignal } from "../../lib/money-summary";

type Contract = components["schemas"]["Contract"];
type ContractMutationResponse = components["schemas"]["ContractMutationResponse"];
type ContractType = components["schemas"]["ContractType"];
type Client = components["schemas"]["Client"];
type RateSpec = components["schemas"]["RateSpec"];
type FeeSpec = components["schemas"]["FeeSpec"];

const TYPES: ContractType[] = ["recurring", "time_and_materials", "retainer", "fixed_fee"];

const TYPE_LABEL: Record<ContractType, string> = {
  recurring: "recurring",
  time_and_materials: "t&m",
  retainer: "retainer",
  fixed_fee: "fixed fee",
};

function today(): string {
  return new Date().toISOString().slice(0, 10);
}

export default function ContractsPage() {
  const auth = useRequireAuth();
  const [contracts, setContracts] = useState<Contract[] | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [clientFilter, setClientFilter] = useState<string>("all");
  const [typeFilter, setTypeFilter] = useState<ContractType | "all">("all");
  const [query, setQuery] = useState("");

  const [createOpen, setCreateOpen] = useState(false);
  const [number, setNumber] = useState("");
  const [name, setName] = useState("");
  const [clientId, setClientId] = useState<string>("");
  const [type, setType] = useState<ContractType>("recurring");
  const [currency, setCurrency] = useState("USD");
  const [effectiveFrom, setEffectiveFrom] = useState(today());
  const [amount, setAmount] = useState("");
  const [frequency, setFrequency] = useState<components["schemas"]["BillingFrequency"]>("monthly");
  const [anchor, setAnchor] = useState(today());
  const [hourlyRate, setHourlyRate] = useState("");
  const [includedMinutes, setIncludedMinutes] = useState("");
  const [overageRate, setOverageRate] = useState("");
  const [rates, setRates] = useState<RateSpec[]>([]);
  const [fees, setFees] = useState<FeeSpec[]>([]);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

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

  const filtered = useMemo(() => {
    if (contracts === null) return null;
    const q = query.trim().toLowerCase();
    return contracts.filter(
      (c) =>
        (clientFilter === "all" || c.client_id === clientFilter) &&
        (typeFilter === "all" || c.contract_type === typeFilter) &&
        (q === "" || c.number.toLowerCase().includes(q) || c.name.toLowerCase().includes(q)),
    );
  }, [contracts, clientFilter, typeFilter, query]);

  // Book posture — counts over the real list + a single-currency roll-up.
  const posture = useMemo(() => {
    const list = contracts ?? [];
    const active = list.filter((c) => c.status === "active");
    const drafts = list.filter((c) => c.status === "draft");
    const renewing = active.filter((c) => {
      const sig = termSignal(c.effective_until);
      return sig !== null;
    });
    const recurring = active
      .filter((c) => c.amount_minor != null)
      .map((c) => ({ currency: c.currency as string, amount: c.amount_minor as number }));
    const single = [...new Set(recurring.map((r) => r.currency))];
    const book: ReactNode =
      recurring.length === 0
        ? "—"
        : single.length === 1
          ? money(
              recurring.reduce((acc, r) => acc + r.amount, 0),
              single[0] as string,
            )
          : "mixed currencies";
    return { activeCount: active.length, draftCount: drafts.length, renewCount: renewing.length, book };
  }, [contracts]);

  const resetForm = () => {
    setNumber("");
    setName("");
    setClientId("");
    setType("recurring");
    setCurrency("USD");
    setEffectiveFrom(today());
    setAmount("");
    setFrequency("monthly");
    setAnchor(today());
    setHourlyRate("");
    setIncludedMinutes("");
    setOverageRate("");
    setRates([]);
    setFees([]);
  };

  const needsAmount = type === "recurring" || type === "retainer";
  const createDisabled =
    creating ||
    number.trim() === "" ||
    name.trim() === "" ||
    clientId === "" ||
    effectiveFrom === "" ||
    (needsAmount && amount.trim() === "");

  const createContract = async () => {
    if (createDisabled) return;
    setCreating(true);
    setCreateError(null);
    const tempId = `pending-contract:${number}`;
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateContractRequest"],
      ContractMutationResponse
    >({
      describe: "Create contract",
      entityId: tempId,
      optimistic: () =>
        setContracts((prev) => [
          ...(prev ?? []),
          {
            id: tempId,
            org_id: "",
            client_id: clientId,
            number: number.trim(),
            name: name.trim(),
            contract_type: type,
            status: "draft",
            currency,
            effective_from: effectiveFrom,
            effective_until: null,
            amount_minor: needsAmount && amount.trim() !== "" ? Math.round(Number(amount) * 100) : null,
            billing_frequency: needsAmount ? frequency : null,
            billing_anchor: needsAmount ? anchor : null,
            hourly_rate_minor: type === "time_and_materials" && hourlyRate.trim() !== "" ? Math.round(Number(hourlyRate) * 100) : null,
            included_minutes: type === "retainer" && includedMinutes.trim() !== "" ? Number(includedMinutes) : null,
            overage_rate_minor: type === "retainer" && overageRate.trim() !== "" ? Math.round(Number(overageRate) * 100) : null,
            notes: null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
        ]),
      revert: () => setContracts((prev) => (prev ?? []).filter((c) => c.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        client_id: clientId,
        number: number.trim(),
        name: name.trim(),
        contract_type: type,
        currency,
        effective_from: effectiveFrom,
        amount_minor: needsAmount && amount.trim() !== "" ? Math.round(Number(amount) * 100) : null,
        billing_frequency: needsAmount ? frequency : null,
        billing_anchor: needsAmount ? anchor : null,
        hourly_rate_minor:
          type === "time_and_materials" && hourlyRate.trim() !== ""
            ? Math.round(Number(hourlyRate) * 100)
            : null,
        included_minutes:
          type === "retainer" && includedMinutes.trim() !== "" ? Number(includedMinutes) : null,
        overage_rate_minor:
          type === "retainer" && overageRate.trim() !== ""
            ? Math.round(Number(overageRate) * 100)
            : null,
        rates: type === "time_and_materials" && rates.length > 0 ? rates : undefined,
        fees: type === "fixed_fee" && fees.length > 0 ? fees : undefined,
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
    resetForm();
    void refresh();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <header className="m-rise flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Contracts</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Recurring, T&M, retainer, and fixed-fee agreements that drive billing.
          </p>
        </div>
        <Dialog
          open={createOpen}
          onOpenChange={(open) => {
            setCreateOpen(open);
            if (!open) setCreateError(null);
          }}
        >
          <DialogTrigger asChild>
            <Button>New contract</Button>
          </DialogTrigger>
          <DialogContent title="New contract">
            <div className="grid max-h-[70vh] gap-3 overflow-auto p-4">
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label htmlFor="c-number">Contract number</Label>
                  <Input id="c-number" value={number} onChange={(e) => setNumber(e.target.value)} placeholder="C-2026-001" />
                </div>
                <div>
                  <Label htmlFor="c-client">Client</Label>
                  <Select value={clientId} onValueChange={setClientId}>
                    <SelectTrigger id="c-client" className="w-full">
                      <SelectValue placeholder="Pick a client" />
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
              </div>
              <div>
                <Label htmlFor="c-name">Name</Label>
                <Input id="c-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Managed services 2026" />
              </div>
              <div className="grid grid-cols-3 gap-3">
                <div>
                  <Label htmlFor="c-type">Type</Label>
                  <Select value={type} onValueChange={(v) => setType(v as ContractType)}>
                    <SelectTrigger id="c-type" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {TYPES.map((t) => (
                        <SelectItem key={t} value={t}>
                          {t.replaceAll("_", " ")}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label htmlFor="c-currency">Currency</Label>
                  <Input id="c-currency" value={currency} onChange={(e) => setCurrency(e.target.value)} />
                </div>
                <div>
                  <Label htmlFor="c-from">Effective from</Label>
                  <Input id="c-from" type="date" value={effectiveFrom} onChange={(e) => setEffectiveFrom(e.target.value)} />
                </div>
              </div>

              {needsAmount ? (
                <div className="grid grid-cols-3 gap-3">
                  <div>
                    <Label htmlFor="c-amount">{type === "retainer" ? "Per-period price" : "Per-period price"} (USD)</Label>
                    <Input id="c-amount" type="number" min={0} step="0.01" value={amount} onChange={(e) => setAmount(e.target.value)} />
                  </div>
                  <div>
                    <Label htmlFor="c-freq">Frequency</Label>
                    <Select
                      value={frequency}
                      onValueChange={(v) => setFrequency(v as components["schemas"]["BillingFrequency"])}
                    >
                      <SelectTrigger id="c-freq" className="w-full">
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
                  </div>
                  <div>
                    <Label htmlFor="c-anchor">Billing anchor</Label>
                    <Input id="c-anchor" type="date" value={anchor} onChange={(e) => setAnchor(e.target.value)} />
                    <FieldHint>First billing period's start.</FieldHint>
                  </div>
                </div>
              ) : null}

              {type === "retainer" ? (
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <Label htmlFor="c-minutes">Included minutes / period</Label>
                    <Input id="c-minutes" type="number" min={0} value={includedMinutes} onChange={(e) => setIncludedMinutes(e.target.value)} />
                  </div>
                  <div>
                    <Label htmlFor="c-overage">Overage hourly rate (USD)</Label>
                    <Input id="c-overage" type="number" min={0} step="0.01" value={overageRate} onChange={(e) => setOverageRate(e.target.value)} />
                  </div>
                </div>
              ) : null}

              {type === "time_and_materials" ? (
                <>
                  <div>
                    <Label htmlFor="c-hourly">Default hourly rate (USD)</Label>
                    <Input id="c-hourly" type="number" min={0} step="0.01" value={hourlyRate} onChange={(e) => setHourlyRate(e.target.value)} />
                    <FieldHint>Used when no rate-card row matches a time entry's work type.</FieldHint>
                  </div>
                  <div>
                    <Label>Rate card</Label>
                    {rates.map((r, i) => (
                      <div key={i} className="mb-1 flex items-center gap-2 text-[13px]">
                        <span className="flex-1">{r.work_type}</span>
                        <span className="tabular-nums">{money(r.rate_minor, currency)}</span>
                        <Button variant="ghost" size="sm" onClick={() => setRates(rates.filter((_, j) => j !== i))}>
                          Remove
                        </Button>
                      </div>
                    ))}
                    <RateRow onAdd={(workType, rateMinor) => setRates([...rates, { work_type: workType, rate_minor: rateMinor }])} />
                  </div>
                </>
              ) : null}

              {type === "fixed_fee" ? (
                <div>
                  <Label>Fee schedule</Label>
                  {fees.map((f, i) => (
                    <div key={i} className="mb-1 flex items-center gap-2 text-[13px]">
                      <span className="flex-1">{f.label}</span>
                      <span>{f.due_on}</span>
                      <span className="tabular-nums">{money(f.amount_minor, currency)}</span>
                      <Button variant="ghost" size="sm" onClick={() => setFees(fees.filter((_, j) => j !== i))}>
                        Remove
                      </Button>
                    </div>
                  ))}
                  <FeeRow onAdd={(fee) => setFees([...fees, fee])} />
                </div>
              ) : null}

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
      </header>

      {/* Book posture — active/draft counts, renewals approaching, one-currency roll-up. */}
      <div data-testid="contract-stat-strip" className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {(
          [
            { label: "Active", value: posture.activeCount, hint: "billable now", testId: "contracts-stat-active" },
            { label: "Draft", value: posture.draftCount, hint: "previews only — no runs bill drafts", testId: "contracts-stat-draft" },
            { label: "Renewals", value: posture.renewCount, hint: "term ends within 30d", testId: "contracts-stat-renew" },
            { label: "Priced book", value: posture.book, hint: "per-period, single currency", testId: "contracts-stat-book" },
          ] as const
        ).map((t, i) => (
          <Card key={t.label} className="m-rise p-4" style={{ animationDelay: `${Math.min(i, 6) * 40}ms` }}>
            <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              {t.label}
            </p>
            <p className="mt-1 font-mono text-xl font-semibold tabular-nums" style={{ color: "var(--m-text)" }} data-testid={t.testId}>
              {t.value}
            </p>
            <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              {t.hint}
            </p>
          </Card>
        ))}
      </div>

      <Card className="m-rise overflow-x-auto">
        <div className="flex flex-wrap items-center justify-between gap-3 p-4 pb-3">
          <h2 className="text-sm font-semibold">All contracts</h2>
          <div className="flex shrink-0 flex-wrap gap-2">
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search number or name…"
              aria-label="Search contracts"
              data-testid="contracts-search"
              style={{ width: 180 }}
            />
            <Select value={typeFilter} onValueChange={(v) => setTypeFilter(v as ContractType | "all")}>
              <SelectTrigger aria-label="Filter by type" style={{ width: 150 }}>
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
            <Select value={clientFilter} onValueChange={setClientFilter}>
              <SelectTrigger aria-label="Filter by client" style={{ width: 180 }}>
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
        </div>
        {filtered === null ? (
          <div className="p-4">
            {loadError !== null ? (
              <p role="alert" className="text-[13px]">{loadError}</p>
            ) : (
              <Spinner />
            )}
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No contracts yet. A contract is the commercial agreement an invoice run bills against." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Number</Th>
                <Th>Name</Th>
                <Th>Client</Th>
                <Th>Type</Th>
                <Th>Status</Th>
                <Th>Term</Th>
                <Th className="text-right">Per period</Th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((contract) => {
                const sig = termSignal(contract.effective_until);
                return (
                  <Tr key={contract.id} className="row-hover">
                    <Td>
                      <Link
                        href={`/contracts/${contract.id}`}
                        className="font-mono text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
                      >
                        {contract.number}
                      </Link>
                    </Td>
                    <Td>
                      <span className="font-medium">{contract.name}</span>
                    </Td>
                    <Td>{clientName(contract.client_id)}</Td>
                    <Td className="text-[13px]">{TYPE_LABEL[contract.contract_type]}</Td>
                    <Td>
                      <Tag tone={contractStatusTone(contract.status)}>{contract.status}</Tag>
                    </Td>
                    <Td>
                      <span className="tabular-nums">
                        {contract.effective_from}
                        {contract.effective_until !== null && contract.effective_until !== undefined
                          ? ` → ${contract.effective_until}`
                          : " → open"}
                      </span>
                      {sig !== null ? (
                        <Tag tone={sig.tone === "warning" ? "warning" : "danger"} className="ml-1">
                          {sig.label}
                        </Tag>
                      ) : null}
                    </Td>
                    <Td className="text-right font-mono text-[12px] tabular-nums">
                      {contract.amount_minor !== null && contract.amount_minor !== undefined
                        ? `${money(contract.amount_minor, contract.currency)} / ${contract.billing_frequency ?? "period"}`
                        : "—"}
                    </Td>
                  </Tr>
                );
              })}
            </tbody>
          </Table>
        )}
      </Card>
    </div>
  );
}

function RateRow({ onAdd }: { onAdd: (workType: string, rateMinor: number) => void }) {
  const [workType, setWorkType] = useState("");
  const [rate, setRate] = useState("");
  return (
    <div className="flex items-end gap-2">
      <div className="flex-1">
        <Input aria-label="Work type" placeholder="work type (e.g. onsite)" value={workType} onChange={(e) => setWorkType(e.target.value)} />
      </div>
      <div className="w-28">
        <Input aria-label="Hourly rate (USD)" type="number" min={0} step="0.01" placeholder="rate" value={rate} onChange={(e) => setRate(e.target.value)} />
      </div>
      <Button
        variant="secondary"
        size="sm"
        disabled={workType.trim() === "" || rate.trim() === ""}
        onClick={() => {
          onAdd(workType.trim(), Math.round(Number(rate) * 100));
          setWorkType("");
          setRate("");
        }}
      >
        Add row
      </Button>
    </div>
  );
}

function FeeRow({ onAdd }: { onAdd: (fee: FeeSpec) => void }) {
  const [label, setLabel] = useState("");
  const [amount, setAmount] = useState("");
  const [dueOn, setDueOn] = useState("");
  return (
    <div className="flex items-end gap-2">
      <div className="flex-1">
        <Input aria-label="Fee label" placeholder="label (e.g. Q1 install)" value={label} onChange={(e) => setLabel(e.target.value)} />
      </div>
      <div className="w-28">
        <Input aria-label="Amount (USD)" type="number" min={0} step="0.01" placeholder="amount" value={amount} onChange={(e) => setAmount(e.target.value)} />
      </div>
      <div className="w-40">
        <Input aria-label="Due on" type="date" value={dueOn} onChange={(e) => setDueOn(e.target.value)} />
      </div>
      <Button
        variant="secondary"
        size="sm"
        disabled={label.trim() === "" || amount.trim() === "" || dueOn === ""}
        onClick={() => {
          onAdd({ label: label.trim(), amount_minor: Math.round(Number(amount) * 100), due_on: dueOn, notes: null });
          setLabel("");
          setAmount("");
          setDueOn("");
        }}
      >
        Add fee
      </Button>
    </div>
  );
}
