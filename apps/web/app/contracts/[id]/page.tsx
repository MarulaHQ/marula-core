// /contracts/[id] — the contract editor (Workspace-core UI, T3).
//
// Purpose: one agreement's full editor: identity + lifecycle, the
// per-type commercial fields (recurring/T&M/retainer/fixed-fee), the T&M
// rate card and fixed-fee schedule (replaced wholesale via PATCH, as the
// API defines), and the per-contract billing preview that links into
// /billing over the same window. All edits ride the IF1–IF3 envelope
// with If-Match versioning and inline typed errors.
// Non-purpose: no invoice issuing (documents are issued artifacts on
// /billing); no contract deletion UI beyond the status lifecycle.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Button,
  Card,
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
import { useParams } from "next/navigation";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { money } from "../../../lib/format";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { contractStatusTone, termSignal } from "../../../lib/money-summary";

type ContractDetail = components["schemas"]["ContractDetail"];
type ContractMutationResponse = components["schemas"]["ContractMutationResponse"];
type BillingEvaluation = components["schemas"]["BillingEvaluation"];
type RateSpec = components["schemas"]["RateSpec"];
type FeeSpec = components["schemas"]["FeeSpec"];

export default function ContractDetailPage(): ReactNode {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const contractId = params.id;

  const [contract, setContract] = useState<ContractDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Editable mirror of the contract's commercial fields.
  const [name, setName] = useState("");
  const [status, setStatus] = useState<components["schemas"]["ContractStatus"]>("draft");
  const [effectiveFrom, setEffectiveFrom] = useState("");
  const [effectiveUntil, setEffectiveUntil] = useState("");
  const [amount, setAmount] = useState("");
  const [frequency, setFrequency] = useState<string>("monthly");
  const [anchor, setAnchor] = useState("");
  const [hourlyRate, setHourlyRate] = useState("");
  const [includedMinutes, setIncludedMinutes] = useState("");
  const [overageRate, setOverageRate] = useState("");
  const [notes, setNotes] = useState("");

  const [previewFrom, setPreviewFrom] = useState(effectiveFrom);
  const [previewTo, setPreviewTo] = useState("");
  const [preview, setPreview] = useState<BillingEvaluation | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/contracts/{id}", {
      params: { path: { id: contractId } },
    });
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load this contract. It may not exist or you may not have access.");
      return;
    }
    setContract(data);
    setLoadError(null);
    setName(data.name);
    setStatus(data.status);
    setEffectiveFrom(data.effective_from);
    setEffectiveUntil(data.effective_until ?? "");
    setAmount(data.amount_minor !== null && data.amount_minor !== undefined ? String(data.amount_minor / 100) : "");
    setFrequency(data.billing_frequency ?? "monthly");
    setAnchor(data.billing_anchor ?? "");
    setHourlyRate(data.hourly_rate_minor !== null && data.hourly_rate_minor !== undefined ? String(data.hourly_rate_minor / 100) : "");
    setIncludedMinutes(data.included_minutes !== null && data.included_minutes !== undefined ? String(data.included_minutes) : "");
    setOverageRate(data.overage_rate_minor !== null && data.overage_rate_minor !== undefined ? String(data.overage_rate_minor / 100) : "");
    setNotes(data.notes ?? "");
    if (previewFrom === "") setPreviewFrom(data.effective_from);
  }, [contractId, previewFrom]);

  useEffect(() => {
    if (auth.status === "authed") void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auth.status, load]);

  const needsAmount =
    contract?.contract_type === "recurring" || contract?.contract_type === "retainer";

  const save = async (extra: Partial<components["schemas"]["PatchContractRequest"]> = {}) => {
    if (contract === null) return;
    setSaving(true);
    setSaveError(null);
    setSaved(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["PatchContractRequest"],
      ContractMutationResponse
    >({
      describe: "Save contract",
      entityId: contract.id,
      optimistic: () => {},
      revert: () => void load(),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        version: base.version,
        name: name.trim(),
        status,
        effective_from: effectiveFrom,
        effective_until: effectiveUntil === "" ? null : effectiveUntil,
        amount_minor: needsAmount && amount.trim() !== "" ? Math.round(Number(amount) * 100) : null,
        billing_frequency: needsAmount ? (frequency as components["schemas"]["BillingFrequency"]) : null,
        billing_anchor: needsAmount && anchor !== "" ? anchor : null,
        hourly_rate_minor:
          contract.contract_type === "time_and_materials" && hourlyRate.trim() !== ""
            ? Math.round(Number(hourlyRate) * 100)
            : null,
        included_minutes:
          contract.contract_type === "retainer" && includedMinutes.trim() !== ""
            ? Number(includedMinutes)
            : null,
        overage_rate_minor:
          contract.contract_type === "retainer" && overageRate.trim() !== ""
            ? Math.round(Number(overageRate) * 100)
            : null,
        notes: notes.trim() === "" ? null : notes.trim(),
        ...extra,
      }),
      request: (client, body) =>
        client.PATCH("/api/v1/contracts/{id}", { params: { path: { id: contractId } }, body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setSaving(false);
    if (!result.ok) {
      setSaveError(rejectionText(result, "The save was rejected (stale version?). Reload and retry."));
      void load();
      return;
    }
    setSaved("Saved.");
    void load();
  };

  const runPreview = async () => {
    setPreviewing(true);
    setPreviewError(null);
    const { data, error } = await api.api.GET("/api/v1/contracts/{id}/billing-preview", {
      params: { path: { id: contractId }, query: { from: previewFrom, to: previewTo } },
    });
    setPreviewing(false);
    if (error !== undefined || data === undefined) {
      setPreview(null);
      setPreviewError("Preview failed — check the window and retry.");
      return;
    }
    setPreview(data);
  };

  if (auth.status !== "authed") return null;

  if (loadError !== null) {
    return (
      <div className="grid gap-4 p-6">
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
        <Link href="/contracts" className="text-[13px] underline underline-offset-2">
          Back to contracts
        </Link>
      </div>
    );
  }

  if (contract === null) {
    return (
      <div className="p-6">
        <Spinner />
      </div>
    );
  }

  const isTm = contract.contract_type === "time_and_materials";
  const isFixed = contract.contract_type === "fixed_fee";
  const isRetainer = contract.contract_type === "retainer";

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <div className="m-rise">
        <Link href="/contracts" className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
          ← Contracts
        </Link>
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <h1 className="font-mono text-lg font-semibold tabular-nums">{contract.number}</h1>
          <Tag tone={contractStatusTone(contract.status)}>{contract.status}</Tag>
          <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {contract.contract_type.replaceAll("_", " ")} · {contract.currency}
          </span>
          {(() => {
            const sig = termSignal(contract.effective_until);
            return sig !== null ? (
              <Tag tone={sig.tone === "warning" ? "warning" : "danger"} data-testid="contract-term-signal">
                {sig.label}
              </Tag>
            ) : null;
          })()}
          <Link
            href={`/clients/${contract.client_id}`}
            className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
          >
            View client
          </Link>
        </div>
      </div>

      {saveError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {saveError}
        </p>
      ) : null}
      {saved !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {saved}
        </p>
      ) : null}

      <Card>
        <h2 className="p-4 pb-0 text-sm font-semibold">Agreement</h2>
        <div className="grid gap-3 p-4">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <Label htmlFor="cd-name">Name</Label>
              <Input id="cd-name" value={name} onChange={(e) => setName(e.target.value)} />
            </div>
            <div>
              <Label htmlFor="cd-status">Status</Label>
              <Select value={status} onValueChange={(v) => setStatus(v as components["schemas"]["ContractStatus"])}>
                <SelectTrigger id="cd-status" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {["draft", "active", "ended", "cancelled"].map((s) => (
                    <SelectItem key={s} value={s}>
                      {s}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldHint>Draft contracts preview but are refused by invoice runs.</FieldHint>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <Label htmlFor="cd-from">Effective from</Label>
              <Input id="cd-from" type="date" value={effectiveFrom} onChange={(e) => setEffectiveFrom(e.target.value)} />
            </div>
            <div>
              <Label htmlFor="cd-until">Effective until</Label>
              <Input id="cd-until" type="date" value={effectiveUntil} onChange={(e) => setEffectiveUntil(e.target.value)} />
              <FieldHint>Empty means open-ended.</FieldHint>
            </div>
          </div>
          {needsAmount ? (
            <div className="grid grid-cols-3 gap-3">
              <div>
                <Label htmlFor="cd-amount">Per-period price (USD)</Label>
                <Input id="cd-amount" type="number" min={0} step="0.01" value={amount} onChange={(e) => setAmount(e.target.value)} />
              </div>
              <div>
                <Label htmlFor="cd-freq">Frequency</Label>
                <Select value={frequency} onValueChange={setFrequency}>
                  <SelectTrigger id="cd-freq" className="w-full">
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
                <Label htmlFor="cd-anchor">Billing anchor</Label>
                <Input id="cd-anchor" type="date" value={anchor} onChange={(e) => setAnchor(e.target.value)} />
              </div>
            </div>
          ) : null}
          {isTm ? (
            <div>
              <Label htmlFor="cd-hourly">Default hourly rate (USD)</Label>
              <Input id="cd-hourly" type="number" min={0} step="0.01" value={hourlyRate} onChange={(e) => setHourlyRate(e.target.value)} />
            </div>
          ) : null}
          {isRetainer ? (
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="cd-minutes">Included minutes / period</Label>
                <Input id="cd-minutes" type="number" min={0} value={includedMinutes} onChange={(e) => setIncludedMinutes(e.target.value)} />
              </div>
              <div>
                <Label htmlFor="cd-overage">Overage hourly rate (USD)</Label>
                <Input id="cd-overage" type="number" min={0} step="0.01" value={overageRate} onChange={(e) => setOverageRate(e.target.value)} />
              </div>
            </div>
          ) : null}
          <div>
            <Label htmlFor="cd-notes">Notes</Label>
            <Input id="cd-notes" value={notes} onChange={(e) => setNotes(e.target.value)} />
          </div>
          <div>
            <Button onClick={() => void save()} disabled={saving}>
              {saving ? <Spinner /> : null} Save changes
            </Button>
          </div>
        </div>
      </Card>

      {isTm ? (
        <Card>
          <h2 className="p-4 pb-0 text-sm font-semibold">Rate card</h2>
          {contract.rates.length === 0 ? (
            <div className="p-4">
              <EmptyState message="No rate-card rows. Every time entry bills at the contract's default hourly rate until rows are added below." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>Work type</Th>
                  <Th className="text-right">Hourly rate</Th>
                </tr>
              </thead>
              <tbody>
                {contract.rates.map((rate) => (
                  <Tr key={rate.id}>
                    <Td>{rate.work_type}</Td>
                    <Td className="text-right tabular-nums">{money(rate.rate_minor, contract.currency)}</Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
          <div className="p-4">
            <RateCardEditor
              onApply={(rows) =>
                void save({
                  rates: rows.map((r) => ({
                    work_type: r.workType,
                    rate_minor: Math.round(Number(r.rate) * 100),
                  })),
                })
              }
              saving={saving}
            />
          </div>
        </Card>
      ) : null}

      {isFixed ? (
        <Card>
          <h2 className="p-4 pb-0 text-sm font-semibold">Fee schedule</h2>
          {contract.fees.length === 0 ? (
            <div className="p-4">
              <EmptyState message="No fees scheduled. A fee becomes a billable line when its due date falls inside an invoice run's window." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>Label</Th>
                  <Th>Due on</Th>
                  <Th className="text-right">Amount</Th>
                  <Th>Notes</Th>
                </tr>
              </thead>
              <tbody>
                {contract.fees.map((fee) => (
                  <Tr key={fee.id}>
                    <Td>{fee.label}</Td>
                    <Td>{fee.due_on}</Td>
                    <Td className="text-right tabular-nums">{money(fee.amount_minor, contract.currency)}</Td>
                    <Td>{fee.notes ?? "—"}</Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
          <div className="p-4">
            <FeeScheduleEditor
              onApply={(rows) =>
                void save({
                  fees: rows.map((f) => ({
                    label: f.label,
                    amount_minor: Math.round(Number(f.amount) * 100),
                    due_on: f.dueOn,
                    notes: f.notes.trim() === "" ? null : f.notes.trim(),
                  })),
                })
              }
              saving={saving}
            />
          </div>
        </Card>
      ) : null}

      <Card>
        <div className="flex items-center justify-between p-4 pb-0">
          <h2 className="text-sm font-semibold">Billing preview</h2>
          <Link href="/billing" className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
            Billing →
          </Link>
        </div>
        <div className="flex items-end gap-2 p-4">
          <div>
            <Label htmlFor="cd-pf">From</Label>
            <Input id="cd-pf" type="date" value={previewFrom} onChange={(e) => setPreviewFrom(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="cd-pt">To</Label>
            <Input id="cd-pt" type="date" value={previewTo} onChange={(e) => setPreviewTo(e.target.value)} />
          </div>
          <Button variant="secondary" onClick={() => void runPreview()} disabled={previewing || previewFrom === "" || previewTo === ""}>
            {previewing ? <Spinner /> : null} Preview
          </Button>
        </div>
        {previewError !== null ? (
          <p role="alert" className="px-4 pb-4 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {previewError}
          </p>
        ) : null}
        {preview !== null ? (
          <div className="px-4 pb-4 text-[13px]">
            Billable candidates: {preview.lines.length} · total{" "}
            <span className="tabular-nums">{money(preview.total_minor, preview.currency)}</span>.
            Run the invoice from the Billing page over the same window.
          </div>
        ) : null}
      </Card>
    </div>
  );
}

function RateCardEditor({ onApply, saving }: { onApply: (rows: { workType: string; rate: string }[]) => void; saving: boolean }) {
  const [rows, setRows] = useState<{ workType: string; rate: string }[]>([]);
  return (
    <div className="grid gap-2">
      {rows.map((row, i) => (
        <div key={i} className="flex items-center gap-2">
          <Input
            aria-label="Work type"
            placeholder="work type"
            value={row.workType}
            onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, workType: e.target.value } : r)))}
          />
          <Input
            className="w-32"
            aria-label="Hourly rate (USD)"
            type="number"
            min={0}
            step="0.01"
            placeholder="rate"
            value={row.rate}
            onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, rate: e.target.value } : r)))}
          />
          <Button variant="ghost" size="sm" onClick={() => setRows(rows.filter((_, j) => j !== i))}>
            Remove
          </Button>
        </div>
      ))}
      <div className="flex gap-2">
        <Button variant="secondary" size="sm" onClick={() => setRows([...rows, { workType: "", rate: "" }])}>
          Add row
        </Button>
        <Button size="sm" disabled={saving || rows.length === 0} onClick={() => onApply(rows)}>
          {saving ? <Spinner /> : null} Replace rate card
        </Button>
      </div>
    </div>
  );
}

function FeeScheduleEditor({ onApply, saving }: { onApply: (rows: { label: string; amount: string; dueOn: string; notes: string }[]) => void; saving: boolean }) {
  const [rows, setRows] = useState<{ label: string; amount: string; dueOn: string; notes: string }[]>([]);
  return (
    <div className="grid gap-2">
      {rows.map((row, i) => (
        <div key={i} className="flex items-center gap-2">
          <Input
            aria-label="Fee label"
            placeholder="label"
            value={row.label}
            onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, label: e.target.value } : r)))}
          />
          <Input
            className="w-32"
            aria-label="Amount (USD)"
            type="number"
            min={0}
            step="0.01"
            placeholder="amount"
            value={row.amount}
            onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, amount: e.target.value } : r)))}
          />
          <Input
            className="w-40"
            aria-label="Due on"
            type="date"
            value={row.dueOn}
            onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, dueOn: e.target.value } : r)))}
          />
          <Input
            aria-label="Notes"
            placeholder="notes (optional)"
            value={row.notes}
            onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, notes: e.target.value } : r)))}
          />
          <Button variant="ghost" size="sm" onClick={() => setRows(rows.filter((_, j) => j !== i))}>
            Remove
          </Button>
        </div>
      ))}
      <div className="flex gap-2">
        <Button variant="secondary" size="sm" onClick={() => setRows([...rows, { label: "", amount: "", dueOn: "", notes: "" }])}>
          Add fee
        </Button>
        <Button
          size="sm"
          disabled={saving || rows.length === 0 || rows.some((r) => r.label.trim() === "" || r.amount === "" || r.dueOn === "")}
          onClick={() => onApply(rows)}
        >
          {saving ? <Spinner /> : null} Replace fee schedule
        </Button>
      </div>
    </div>
  );
}
