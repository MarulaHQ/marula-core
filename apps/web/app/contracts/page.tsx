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
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
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
type BlockSpec = components["schemas"]["BlockSpec"];
type CapSpec = components["schemas"]["CapSpec"];

const TYPES: ContractType[] = [
  "recurring",
  "time_and_materials",
  "retainer",
  "fixed_fee",
  "per_unit",
  "per_incident",
];

const TYPE_LABEL: Record<ContractType, string> = {
  recurring: "recurring",
  time_and_materials: "t&m",
  retainer: "retainer",
  fixed_fee: "fixed fee",
  per_unit: "per-unit",
  per_incident: "per-incident",
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
  const [blocks, setBlocks] = useState<BlockSpec[]>([]);
  const [unitPrice, setUnitPrice] = useState("");
  const [unitBasis, setUnitBasis] = useState<"devices" | "users">("devices");
  const [incidentPrice, setIncidentPrice] = useState("");
  const [incidentPriority, setIncidentPriority] = useState<string>("any");
  const [poNumber, setPoNumber] = useState("");
  const [minimum, setMinimum] = useState("");
  const [caps, setCaps] = useState<CapSpec[]>([]);
  const [escalatorBps, setEscalatorBps] = useState("");
  const [escalatorFrom, setEscalatorFrom] = useState("");
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
    setUnitPrice("");
    setUnitBasis("devices");
    setIncidentPrice("");
    setIncidentPriority("any");
    setPoNumber("");
    setMinimum("");
    setRates([]);
    setFees([]);
    setBlocks([]);
    setCaps([]);
    setEscalatorBps("");
    setEscalatorFrom("");
  };

  const needsAmount = type === "recurring" || type === "retainer";
  // Prepaid labor blocks ride the two labor-billing types; NTE caps are
  // T&M-only. The server refuses both otherwise (validate_specs), so the
  // form never offers them where they would be rejected.
  const blockBearing = type === "time_and_materials" || type === "retainer";
  const capBearing = type === "time_and_materials";
  // A minimum tops a period up against base + consumed labor, and the
  // engine prices consumed labor for T&M as well as retainer, so the
  // labor types carry a floor too. An escalator walks billing
  // anniversaries off the anchor instead, so it needs a billing
  // frequency: the types that have one.
  const minimumBearing = needsAmount || type === "time_and_materials";
  const escalatorBearing = needsAmount;
  const createDisabled =
    creating ||
    number.trim() === "" ||
    name.trim() === "" ||
    clientId === "" ||
    effectiveFrom === "" ||
    (needsAmount && amount.trim() === "") ||
    (type === "per_unit" && unitPrice.trim() === "") ||
    (type === "per_incident" && incidentPrice.trim() === "");

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
            unit_price_minor: type === "per_unit" && unitPrice.trim() !== "" ? Math.round(Number(unitPrice) * 100) : null,
            unit_basis: type === "per_unit" ? unitBasis : null,
            incident_price_minor: type === "per_incident" && incidentPrice.trim() !== "" ? Math.round(Number(incidentPrice) * 100) : null,
            incident_priority: type === "per_incident" ? incidentPriority : null,
            po_number: poNumber.trim() !== "" ? poNumber.trim() : null,
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
        unit_price_minor:
          type === "per_unit" && unitPrice.trim() !== "" ? Math.round(Number(unitPrice) * 100) : null,
        unit_basis: type === "per_unit" ? unitBasis : null,
        incident_price_minor:
          type === "per_incident" && incidentPrice.trim() !== ""
            ? Math.round(Number(incidentPrice) * 100)
            : null,
        incident_priority: type === "per_incident" ? incidentPriority : null,
        po_number: poNumber.trim() !== "" ? poNumber.trim() : null,
        minimum_commitment_minor:
          minimum.trim() !== "" ? Math.round(Number(minimum) * 100) : null,
        escalator:
          escalatorBps.trim() === ""
            ? undefined
            : {
                uplift_bps: Math.round(Number(escalatorBps)),
                first_uplift_on: escalatorFrom === "" ? null : escalatorFrom,
              },
        rates: type === "time_and_materials" && rates.length > 0 ? rates : undefined,
        fees: type === "fixed_fee" && fees.length > 0 ? fees : undefined,
        blocks: blockBearing && blocks.length > 0 ? blocks : undefined,
        caps: capBearing && caps.length > 0 ? caps : undefined,
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
            Recurring, T&M, retainer, fixed-fee, per-unit, and per-incident agreements that drive billing.
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
                <>
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
                <div>
                  <Label>Prepaid hour blocks</Label>
                  <FieldHint>
                    Buy-forward hours on top of the retainer&apos;s included minutes. Draws come off
                    the oldest block first.
                  </FieldHint>
                  {blocks.map((b, i) => (
                    <div key={i} className="mb-1 flex flex-wrap items-center gap-2 text-[13px]">
                      <span className="flex-1">
                        {b.label} · {b.total_minutes} min · {money(b.sale_price_minor, currency)}
                      </span>
                      <span style={{ color: "var(--m-text-muted)" }}>{b.rollover_policy}</span>
                      <Button variant="ghost" size="sm" onClick={() => setBlocks(blocks.filter((_, j) => j !== i))}>
                        Remove
                      </Button>
                    </div>
                  ))}
                  <BlockRow onAdd={(block) => setBlocks([...blocks, block])} />
                </div>
                </>
              ) : null}

              {type === "per_unit" ? (
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <Label htmlFor="c-unit-price">Price per unit / period (USD)</Label>
                    <Input id="c-unit-price" type="number" min={0} step="0.01" value={unitPrice} onChange={(e) => setUnitPrice(e.target.value)} />
                  </div>
                  <div>
                    <Label htmlFor="c-unit-basis">Count units by</Label>
                    <Select value={unitBasis} onValueChange={(v) => setUnitBasis(v as "devices" | "users")}>
                      <SelectTrigger id="c-unit-basis" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="devices">Devices (active client assets)</SelectItem>
                        <SelectItem value="users">Users (org seats)</SelectItem>
                      </SelectContent>
                    </Select>
                    <FieldHint>Counted from the inventories at each billing evaluation.</FieldHint>
                  </div>
                </div>
              ) : null}

              {type === "per_incident" ? (
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <Label htmlFor="c-incident-price">Price per closed ticket (USD)</Label>
                    <Input id="c-incident-price" type="number" min={0} step="0.01" value={incidentPrice} onChange={(e) => setIncidentPrice(e.target.value)} />
                  </div>
                  <div>
                    <Label htmlFor="c-incident-priority">Ticket band</Label>
                    <Select value={incidentPriority} onValueChange={setIncidentPriority}>
                      <SelectTrigger id="c-incident-priority" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="any">Any closed ticket</SelectItem>
                        {["p1", "p2", "p3", "p4"].map((p) => (
                          <SelectItem key={p} value={p}>
                            {p.toUpperCase()} only
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              ) : null}

              <div>
                <Label htmlFor="c-po">Client PO number (optional)</Label>
                <Input id="c-po" value={poNumber} onChange={(e) => setPoNumber(e.target.value)} placeholder="PO-2026-014" />
                <FieldHint>Printed on every invoice this contract produces. Tax is not computed here — it stays an accounting-export setting.</FieldHint>
              </div>

              {minimumBearing ? (
                <div>
                  <Label htmlFor="c-minimum">Minimum commitment per period (USD, optional)</Label>
                  <Input id="c-minimum" type="number" min={0} step="0.01" value={minimum} onChange={(e) => setMinimum(e.target.value)} />
                  <FieldHint>A true-up line tops a period up when billing falls short of it, counting the labor the period consumed.</FieldHint>
                </div>
              ) : null}

              {escalatorBearing ? (
                <div>
                  <Label htmlFor="c-escalator">Annual escalator (basis points, optional)</Label>
                  <div className="flex items-center gap-2">
                    <Input
                      id="c-escalator"
                      className="w-32"
                      type="number"
                      min={1}
                      max={5000}
                      step={50}
                      value={escalatorBps}
                      onChange={(e) => setEscalatorBps(e.target.value)}
                    />
                    <Input
                      aria-label="First uplift on"
                      type="date"
                      value={escalatorFrom}
                      onChange={(e) => setEscalatorFrom(e.target.value)}
                    />
                  </div>
                  <FieldHint>
                    300 is +3.0% at every billing anniversary. Leave the date empty to start at the
                    anchor&apos;s first full period.
                  </FieldHint>
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
                  <div>
                    <Label>Not-to-exceed caps</Label>
                    <FieldHint>
                      A ceiling on billable work. Reaching it is never silent: the cap says what
                      happens — stop billing, warn only, or bill the overage at a rate you set.
                    </FieldHint>
                    {caps.map((c, i) => (
                      <div key={i} className="mb-1 flex flex-wrap items-center gap-2 text-[13px]">
                        <span className="flex-1">
                          {c.scope === "period" ? "Per period" : "Per contract"}
                          {c.work_type ? ` · ${c.work_type}` : ""} · {money(c.cap_minor, currency)}
                        </span>
                        <span style={{ color: "var(--m-text-muted)" }}>
                          {c.reached_behavior === "stop"
                            ? "stops at the cap"
                            : c.reached_behavior === "warn"
                              ? "warns only"
                              : "bills the overage"}
                        </span>
                        <Button variant="ghost" size="sm" onClick={() => setCaps(caps.filter((_, j) => j !== i))}>
                          Remove
                        </Button>
                      </div>
                    ))}
                    <CapRow onAdd={(cap) => setCaps([...caps, cap])} />
                  </div>
                  <div>
                    <Label>Prepaid hour blocks</Label>
                    <FieldHint>
                      Hours the client paid for up front. Draws come off the oldest block first;
                      minutes past the balance spill onto the overage card.
                    </FieldHint>
                    {blocks.map((b, i) => (
                      <div key={i} className="mb-1 flex flex-wrap items-center gap-2 text-[13px]">
                        <span className="flex-1">
                          {b.label} · {b.total_minutes} min · {money(b.sale_price_minor, currency)}
                        </span>
                        <span style={{ color: "var(--m-text-muted)" }}>{b.rollover_policy}</span>
                        <Button variant="ghost" size="sm" onClick={() => setBlocks(blocks.filter((_, j) => j !== i))}>
                          Remove
                        </Button>
                      </div>
                    ))}
                    <BlockRow onAdd={(block) => setBlocks([...blocks, block])} />
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

function BlockRow({ onAdd }: { onAdd: (block: BlockSpec) => void }) {
  const [label, setLabel] = useState("");
  const [minutes, setMinutes] = useState("");
  const [sale, setSale] = useState("");
  const [rate, setRate] = useState("");
  const [rollover, setRollover] = useState<"expire" | "rollover" | "none">("expire");
  const [expires, setExpires] = useState("");
  return (
    <div className="flex flex-wrap items-end gap-2">
      <div className="min-w-40 flex-1">
        <Input aria-label="Block label" placeholder="label (e.g. 50-hour block, March)" value={label} onChange={(e) => setLabel(e.target.value)} />
      </div>
      <div className="w-28">
        <Input aria-label="Block minutes" type="number" min={1} step={15} placeholder="minutes" value={minutes} onChange={(e) => setMinutes(e.target.value)} />
      </div>
      <div className="w-28">
        <Input aria-label="Block sale price (USD)" type="number" min={0} step="0.01" placeholder="sale price" value={sale} onChange={(e) => setSale(e.target.value)} />
      </div>
      <div className="w-28">
        <Input aria-label="Block draw-down rate (USD/hour)" type="number" min={0} step="0.01" placeholder="block rate" value={rate} onChange={(e) => setRate(e.target.value)} />
      </div>
      <div className="w-32">
        <Select value={rollover} onValueChange={(v) => setRollover(v as "expire" | "rollover" | "none")}>
          <SelectTrigger aria-label="Rollover policy" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="expire">unused minutes lapse</SelectItem>
            <SelectItem value="rollover">roll into a successor</SelectItem>
            <SelectItem value="none">hold, no rollover</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="w-40">
        <Input aria-label="Block expires on" type="date" value={expires} onChange={(e) => setExpires(e.target.value)} />
      </div>
      <Button
        variant="secondary"
        size="sm"
        disabled={label.trim() === "" || minutes.trim() === "" || sale.trim() === ""}
        onClick={() => {
          onAdd({
            label: label.trim(),
            total_minutes: Math.round(Number(minutes)),
            sale_price_minor: Math.round(Number(sale) * 100),
            rollover_policy: rollover,
            block_rate_minor: rate.trim() === "" ? null : Math.round(Number(rate) * 100),
            expires_on: expires === "" ? null : expires,
          });
          setLabel("");
          setMinutes("");
          setSale("");
          setRate("");
        }}
      >
        Add block
      </Button>
    </div>
  );
}

function CapRow({ onAdd }: { onAdd: (cap: CapSpec) => void }) {
  const [scope, setScope] = useState<"period" | "contract">("period");
  const [cap, setCap] = useState("");
  const [behavior, setBehavior] = useState<"stop" | "warn" | "overage_billable">("warn");
  const [workType, setWorkType] = useState("");
  const [overage, setOverage] = useState("");
  return (
    <div className="flex flex-wrap items-end gap-2">
      <div className="w-32">
        <Select value={scope} onValueChange={(v) => setScope(v as "period" | "contract")}>
          <SelectTrigger aria-label="Cap scope" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="period">Per period</SelectItem>
            <SelectItem value="contract">Whole contract</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="w-28">
        <Input aria-label="Cap amount (USD)" type="number" min={0} step="0.01" placeholder="cap" value={cap} onChange={(e) => setCap(e.target.value)} />
      </div>
      <div className="w-44">
        <Select value={behavior} onValueChange={(v) => setBehavior(v as "stop" | "warn" | "overage_billable")}>
          <SelectTrigger aria-label="Cap reached behavior" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="warn">Warn only</SelectItem>
            <SelectItem value="stop">Stop at the cap</SelectItem>
            <SelectItem value="overage_billable">Bill the overage</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="min-w-32 flex-1">
        <Input aria-label="Cap work type (optional)" placeholder="work type (all labor)" value={workType} onChange={(e) => setWorkType(e.target.value)} />
      </div>
      <div className="w-28">
        <Input aria-label="Overage rate (USD/hour)" type="number" min={0} step="0.01" placeholder="overage rate" value={overage} onChange={(e) => setOverage(e.target.value)} />
      </div>
      <Button
        variant="secondary"
        size="sm"
        disabled={cap.trim() === ""}
        onClick={() => {
          onAdd({
            scope,
            cap_minor: Math.round(Number(cap) * 100),
            reached_behavior: behavior,
            work_type: workType.trim() === "" ? null : workType.trim(),
            overage_rate_minor: overage.trim() === "" ? null : Math.round(Number(overage) * 100),
          });
          setCap("");
          setWorkType("");
          setOverage("");
        }}
      >
        Add cap
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
