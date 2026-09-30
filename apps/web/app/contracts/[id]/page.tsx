// /contracts/[id] — the contract record + editor (Workspace-core UI T3),
// reshaped to the Marula surface pack's record template (Wave 41 lane 7
// `p41-ui-clients`).
//
// DERIVATION-GUIDE: `/contracts` and `/contracts/[id]` **Derive** from TL and
// the KIT components. `DESIGN-NOTES-clients.md` §7 says a contract copies the
// client's "Contracts and billing" tab: the contract card becomes the page,
// and the ledger is its invoices. So this page is the SAME three-column record
// shape the client and project records use — customer on the left, the editor
// in the middle, what needs attention on the right — with the editors this
// page has always carried (agreement, rate card, fee schedule, prepaid
// blocks, not-to-exceed caps, change orders, service credits, billing
// preview) untouched inside the content column.
//
// What changed is the frame and the signals, not the editing:
//   1. **The renewal strip**, blush, stating the date a renewal quote should
//      go out — the pack's point that a renewal is missed when the lead time
//      is not written down.
//   2. **A terms card** of four figures (value, term, billing, terms).
//   3. **A right-hand summary** — status, term, renewal — so what needs
//      attention is on the page rather than a click away.
//   4. Every existing editor, its mutation, and its envelope versioning kept
//      exactly as it was.
//
// THE colour rule (`DESIGN-NOTES.md` §A2): a contract nearing renewal is
// BLUSH, never red. Red is only a genuine SLA breach, a P1, or a destructive
// action — and a term that has ALREADY elapsed is the one renewal-adjacent
// case allowed red, because the end date has genuinely passed.
//
// Non-purpose: no invoice issuing (documents are issued artifacts on
// /billing); no contract deletion UI beyond the status lifecycle.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Badge,
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
import { formatPackDate, renewalQuoteDueDate, renewalSignal } from "../../../lib/renewal-signals";
import {
  RecordFrame,
  RecordRow,
  RecordSection,
  SummaryCard,
} from "../../../components/records/record-frame";

type ContractDetail = components["schemas"]["ContractDetail"];
type ContractMutationResponse = components["schemas"]["ContractMutationResponse"];
type BillingEvaluation = components["schemas"]["BillingEvaluation"];
type RateSpec = components["schemas"]["RateSpec"];
type FeeSpec = components["schemas"]["FeeSpec"];
type BlockSpec = components["schemas"]["BlockSpec"];
type CapSpec = components["schemas"]["CapSpec"];
type ChangeOrder = components["schemas"]["ContractChangeOrder"];
type SlaCredit = components["schemas"]["ContractSlaCredit"];

export default function ContractDetailPage(): ReactNode {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const contractId = params.id;

  const [contract, setContract] = useState<ContractDetail | null>(null);
  // The client's name for the left column. Fetched once per contract and
  // read defensively: a client that is out of scope must not blank the
  // column, it names itself as the reference the contract already carries.
  const [contractClientName, setContractClientName] = useState<string>("Client");
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
  const [poNumber, setPoNumber] = useState("");
  const [unitPrice, setUnitPrice] = useState("");
  const [unitBasis, setUnitBasis] = useState<"devices" | "users">("devices");
  const [incidentPrice, setIncidentPrice] = useState("");
  const [incidentPriority, setIncidentPriority] = useState<string>("any");
  const [minimum, setMinimum] = useState("");
  const [escalatorBps, setEscalatorBps] = useState("");
  const [escalatorFrom, setEscalatorFrom] = useState("");
  const [dropEscalator, setDropEscalator] = useState(false);

  // Child-entity editors (blocks, caps, change orders, service credits)
  // each save through their own endpoint; `childError` is the single line
  // they report rejections on so a failed write never reads as a success.
  const [childError, setChildError] = useState<string | null>(null);
  const [childBusy, setChildBusy] = useState(false);
  const [orderDraft, setOrderDraft] = useState({
    number: "",
    title: "",
    amount: "",
    description: "",
  });
  const [creditDraft, setCreditDraft] = useState({
    reason: "",
    amount: "",
    appliesOn: "",
  });

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
    // The client's name, resolved separately from the contract payload.
    // A failure here is not a page failure: the column falls back to a
    // neutral label and the contract's own fields still read.
    void api.api.GET("/api/v1/clients").then(({ data: cs }) => {
      const found = (cs ?? []).find((c) => c.id === data.client_id);
      if (found !== undefined) setContractClientName(found.name);
    });
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
    setPoNumber(data.po_number ?? "");
    setUnitPrice(
      data.unit_price_minor !== null && data.unit_price_minor !== undefined
        ? String(data.unit_price_minor / 100)
        : "",
    );
    setUnitBasis(data.unit_basis ?? "devices");
    setIncidentPrice(
      data.incident_price_minor !== null && data.incident_price_minor !== undefined
        ? String(data.incident_price_minor / 100)
        : "",
    );
    setIncidentPriority(data.incident_priority ?? "any");
    setMinimum(
      data.minimum_commitment_minor !== null && data.minimum_commitment_minor !== undefined
        ? String(data.minimum_commitment_minor / 100)
        : "",
    );
    setEscalatorBps(data.escalator === null || data.escalator === undefined ? "" : String(data.escalator.uplift_bps));
    setEscalatorFrom(data.escalator?.first_uplift_on ?? "");
    setDropEscalator(false);
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
        // A blank field means "leave as it is" for every optional money
        // column the PATCH treats as `None = unchanged`. Only the two
        // tri-state fields (minimum, escalator) can actually be cleared,
        // so blank minimum sends `clear_minimum` and the escalator sends
        // `clear_escalator`. Silently ignoring a cleared field would be
        // worse than a hint that says so.
        po_number: poNumber.trim() === "" ? null : poNumber.trim(),
        unit_price_minor:
          contract.contract_type === "per_unit" && unitPrice.trim() !== ""
            ? Math.round(Number(unitPrice) * 100)
            : null,
        unit_basis: contract.contract_type === "per_unit" ? unitBasis : null,
        incident_price_minor:
          contract.contract_type === "per_incident" && incidentPrice.trim() !== ""
            ? Math.round(Number(incidentPrice) * 100)
            : null,
        incident_priority: contract.contract_type === "per_incident" ? incidentPriority : null,
        minimum_commitment_minor:
          minimum.trim() === "" ? null : Math.round(Number(minimum) * 100),
        clear_minimum: minimum.trim() === "",
        ...(dropEscalator
          ? { clear_escalator: true }
          : escalatorBps.trim() === ""
            ? {}
            : {
                escalator: {
                  uplift_bps: Math.round(Number(escalatorBps)),
                  first_uplift_on: escalatorFrom === "" ? null : escalatorFrom,
                },
              }),
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

  // Change orders and service credits are their own endpoints rather than
  // columns on the contract, so each write reports on its own line and a
  // rejection never leaves a half-saved form looking saved.
  const createChangeOrder = async () => {
    if (orderDraft.number.trim() === "" || orderDraft.title.trim() === "") return;
    if (orderDraft.amount.trim() === "") return;
    setChildBusy(true);
    setChildError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateChangeOrderRequest"],
      ChangeOrder
    >({
      describe: `Create change order ${orderDraft.number.trim()}`,
      entityId: `${contractId}:${orderDraft.number.trim()}`,
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        number: orderDraft.number.trim(),
        title: orderDraft.title.trim(),
        amount_minor: Math.round(Number(orderDraft.amount) * 100),
        description: orderDraft.description.trim() === "" ? null : orderDraft.description.trim(),
      }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, body) =>
        client.POST("/api/v1/contracts/{id}/change-orders", {
          params: { path: { id: contractId } },
          body,
        }),
    });
    setChildBusy(false);
    if (!result.ok) {
      setChildError(result.friendly ?? rejectionText(result, "The change order was rejected."));
      return;
    }
    setOrderDraft({ number: "", title: "", amount: "", description: "" });
    void load();
  };

  const transitionOrder = async (order: ChangeOrder, action: "approve" | "reject" | "cancel") => {
    setChildBusy(true);
    setChildError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["TransitionChangeOrderRequest"],
      ChangeOrder
    >({
      describe: `${action} change order ${order.number}`,
      entityId: order.id,
      buildBody: (base) => ({ ...base, action }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, body) =>
        client.POST("/api/v1/contracts/{id}/change-orders/{order_id}/transition", {
          params: { path: { id: contractId, order_id: order.id } },
          body,
        }),
    });
    setChildBusy(false);
    if (!result.ok) {
      setChildError(result.friendly ?? rejectionText(result, `The ${action} was rejected.`));
      return;
    }
    void load();
  };

  const createCredit = async () => {
    if (contract === null) return;
    if (creditDraft.reason.trim() === "") return;
    if (creditDraft.amount.trim() === "" || creditDraft.appliesOn === "") return;
    setChildBusy(true);
    setChildError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateSlaCreditRequest"],
      SlaCredit
    >({
      describe: "Issue service credit",
      entityId: `${contractId}:${creditDraft.reason.trim().slice(0, 24)}`,
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        reason: creditDraft.reason.trim(),
        amount_minor: Math.round(Number(creditDraft.amount) * 100),
        currency: contract.currency,
        applies_on: creditDraft.appliesOn,
        ticket_id: null,
      }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, body) =>
        client.POST("/api/v1/contracts/{id}/sla-credits", {
          params: { path: { id: contractId } },
          body,
        }),
    });
    setChildBusy(false);
    if (!result.ok) {
      setChildError(result.friendly ?? rejectionText(result, "The credit was rejected."));
      return;
    }
    setCreditDraft({ reason: "", amount: "", appliesOn: "" });
    void load();
  };

  const cancelCredit = async (credit: SlaCredit) => {
    setChildBusy(true);
    setChildError(null);
    const result = await runEnvelopeMutation<components["schemas"]["MutationParams"], SlaCredit>({
      describe: `Cancel service credit ${credit.id}`,
      entityId: credit.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client) =>
        // The cancel route takes no body: the credit id in the path is the
        // whole instruction, and a repeat cancel answers with the same row.
        client.POST("/api/v1/contracts/{id}/sla-credits/{credit_id}/cancel", {
          params: { path: { id: contractId, credit_id: credit.id } },
        }),
    });
    setChildBusy(false);
    if (!result.ok) {
      setChildError(result.friendly ?? rejectionText(result, "The cancellation was rejected."));
      return;
    }
    void load();
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
  const isPerUnit = contract.contract_type === "per_unit";
  const isPerIncident = contract.contract_type === "per_incident";
  // Two different questions, two different answers. A minimum tops a
  // period up against base + consumed labor, and the engine prices
  // consumed labor for T&M as well as retainer, so a T&M contract carries
  // a floor. An escalator walks billing anniversaries off the anchor, so
  // it needs a billing frequency — the types that have one.
  const minimumBearing = needsAmount || isTm;
  const escalatorBearing = needsAmount;
  const nowForTerm = Date.now();
  const recordRenewal = renewalSignal(contract.effective_until, nowForTerm);
  const recordQuoteDue = renewalQuoteDueDate(contract.effective_until);
  const clientLabel = contractClientName;

  return (
    <div className="mx-auto grid w-full max-w-[1600px] gap-4 p-4 md:p-6">
      <RecordFrame
        context={
          <>
            <RecordSection label="Client">
              <Link
                href={`/clients/${contract.client_id}`}
                className="text-[16px] font-semibold underline decoration-transparent underline-offset-2 hover:decoration-[var(--m-border-strong)]"
              >
                {clientLabel}
              </Link>
            </RecordSection>
            <RecordSection label="Contract">
              <p className="text-[16px] font-semibold">{contract.name}</p>
              <p className="font-mono text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                {contract.number}
              </p>
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <Tag tone={contractStatusTone(contract.status)}>{contract.status}</Tag>
                <Badge tone="neutral">{contract.contract_type.replaceAll("_", " ")}</Badge>
              </div>
            </RecordSection>
            <RecordSection label="Term">
              <RecordRow label="From">{formatPackDate(contract.effective_from) ?? "—"}</RecordRow>
              <RecordRow label="Until">
                {formatPackDate(contract.effective_until) ?? "Open-ended"}
              </RecordRow>
              {recordRenewal !== null ? (
                /* BLUSH for a renewal inside its window. Only an ELAPSED term
                   takes red, and only then. */
                <span className="mt-1.5 inline-flex">
                  <Badge tone={recordRenewal.tone === "danger" ? "danger" : "due"}>
                    <span className="inline-flex items-center gap-1">
                      <ClockGlyph />
                      {recordRenewal.label}
                    </span>
                  </Badge>
                </span>
              ) : null}
            </RecordSection>
            <RecordSection label="Value">
              <RecordRow label="Per period">
                {contract.amount_minor != null
                  ? `${money(contract.amount_minor, contract.currency)} / ${contract.billing_frequency ?? "period"}`
                  : "Not priced"}
              </RecordRow>
            </RecordSection>
          </>
        }
        content={
          <>
      <div className="m-rise">
        <div className="flex flex-wrap items-center gap-3">
          <Link href="/contracts" className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
            ← Contracts
          </Link>
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

      {/* The renewal strip: blush, and it names the date the renewal quote
          should go out, so the lead time is written down rather than assumed. */}
      {recordRenewal !== null ? (
        <div
          role="status"
          data-testid="contract-renewal-strip"
          className="mt-3 flex flex-wrap items-center gap-3 rounded-xl px-4 py-2.5"
          style={{ background: "var(--m-status-due-bg, var(--m-time))", color: "var(--m-status-due-fg, var(--m-time-fg)" }}
        >
          <ClockGlyph />
          <span className="text-[13px]">
            <strong className="font-bold">{recordRenewal.label}.</strong>{" "}
            {recordQuoteDue !== null
              ? `The renewal quote should go out by ${formatPackDate(recordQuoteDue)}, 60 days before the end.`
              : "The renewal quote should go out before the end."}
          </span>
          <span className="flex-1" />
          {/* On the contract's own record, "start renewal" means moving the
              term — so this focuses the field that does it rather than
              opening a second, parallel editor somewhere else. */}
          <Button
            size="sm"
            variant="secondary"
            data-testid="contract-start-renewal"
            onClick={() => {
              const field = document.getElementById("cd-until");
              if (field instanceof HTMLElement) {
                field.focus();
                field.scrollIntoView({ block: "center" });
              }
            }}
          >
            Start renewal
          </Button>
        </div>
      ) : null}

      {/* The contract's terms as four figures — the card the pack makes the
          page. */}
      <div
        className="mt-3 grid grid-cols-2 gap-4 rounded-2xl border p-4 lg:grid-cols-4"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        data-testid="contract-terms-card"
      >
        <Figure label="Value" emphasis
          value={contract.amount_minor != null ? `${money(contract.amount_minor, contract.currency)} a ${shortFreq(contract.billing_frequency)}` : "—"} />
        <Figure label="Term"
          value={`${formatPackDate(contract.effective_from) ?? "—"} to ${formatPackDate(contract.effective_until) ?? "open"}`} />
        <Figure label="Billing"
          value={contract.billing_frequency != null ? `${shortFreq(contract.billing_frequency)}, on the ${billingDay(contract.billing_anchor)}` : "—"} />
        <Figure label="Payment terms" value={contract.po_number != null && contract.po_number !== "" ? `Net 14 · ${contract.po_number}` : "Net 14"} />
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
          {isPerUnit ? (
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="cd-unit-price">Price per unit / period (USD)</Label>
                <Input id="cd-unit-price" type="number" min={0} step="0.01" value={unitPrice} onChange={(e) => setUnitPrice(e.target.value)} />
                <FieldHint>Counted at each evaluation, so the line follows the inventory.</FieldHint>
              </div>
              <div>
                <Label htmlFor="cd-unit-basis">Count units by</Label>
                <Select value={unitBasis} onValueChange={(v) => setUnitBasis(v as "devices" | "users")}>
                  <SelectTrigger id="cd-unit-basis" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="devices">Devices (active client assets)</SelectItem>
                    <SelectItem value="users">Users (org seats)</SelectItem>
                  </SelectContent>
                </Select>
                <FieldHint>Changing the basis re-prices the next period, not past invoices.</FieldHint>
              </div>
            </div>
          ) : null}
          {isPerIncident ? (
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="cd-incident-price">Price per closed ticket (USD)</Label>
                <Input id="cd-incident-price" type="number" min={0} step="0.01" value={incidentPrice} onChange={(e) => setIncidentPrice(e.target.value)} />
                <FieldHint>One line per ticket closed inside the run&apos;s window.</FieldHint>
              </div>
              <div>
                <Label htmlFor="cd-incident-priority">Ticket band</Label>
                <Select value={incidentPriority} onValueChange={setIncidentPriority}>
                  <SelectTrigger id="cd-incident-priority" className="w-full">
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
            <Label htmlFor="cd-po">Client PO number</Label>
            <Input id="cd-po" value={poNumber} onChange={(e) => setPoNumber(e.target.value)} placeholder="PO-2026-014" />
            <FieldHint>
              Copied onto every invoice this contract produces. Tax is not computed here — it stays
              an accounting-export setting. A blank field leaves the stored PO alone.
            </FieldHint>
          </div>
          {minimumBearing ? (
            <div>
              <Label htmlFor="cd-minimum">Minimum commitment / period (USD)</Label>
              <Input id="cd-minimum" type="number" min={0} step="0.01" value={minimum} onChange={(e) => setMinimum(e.target.value)} />
              <FieldHint>
                A true-up line tops a short period up to this, counting the labor the period
                consumed. Empty clears it.
              </FieldHint>
            </div>
          ) : null}
          {escalatorBearing ? (
            <div>
              <Label htmlFor="cd-escalator">Annual escalator (bps)</Label>
              <div className="flex items-center gap-2">
                <Input
                  id="cd-escalator"
                  className="w-28"
                  type="number"
                  min={1}
                  max={5000}
                  step={50}
                  disabled={dropEscalator}
                  value={escalatorBps}
                  onChange={(e) => setEscalatorBps(e.target.value)}
                />
                <Input
                  aria-label="First uplift on"
                  type="date"
                  disabled={dropEscalator}
                  value={escalatorFrom}
                  onChange={(e) => setEscalatorFrom(e.target.value)}
                />
              </div>
              <FieldHint>
                300 is +3.0% at each billing anniversary.
                {contract.escalator ? (
                  <>
                    {" "}
                    Currently {contract.escalator.uplift_bps} bps
                    {contract.escalator.first_uplift_on
                      ? ` from ${contract.escalator.first_uplift_on}`
                      : " from the first full period"}
                    .{" "}
                    <button
                      type="button"
                      className="underline underline-offset-2"
                      onClick={() => setDropEscalator((v) => !v)}
                    >
                      {dropEscalator ? "Keep it" : "Remove on save"}
                    </button>
                  </>
                ) : null}
              </FieldHint>
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

      {childError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="contract-child-error">
          {childError}
        </p>
      ) : null}

      {isTm || isRetainer ? (
        <Card data-testid="contract-blocks">
          <h2 className="p-4 pb-0 text-sm font-semibold">Prepaid blocks</h2>
          <p className="p-4 pb-0 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Hours the client paid for up front. Draws come off the oldest block first, and minutes
            past the balance spill onto the overage rate. The balance itself lives in the
            verification workspace, which is where a period is checked.
          </p>
          {contract.blocks.length === 0 ? (
            <div className="p-4">
              <EmptyState message="No prepaid blocks. Every approved entry bills at the rate card until one is sold." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>Block</Th>
                  <Th className="text-right">Sold</Th>
                  <Th className="text-right">Sale price</Th>
                  <Th className="text-right">Block rate</Th>
                  <Th>Unused</Th>
                  <Th>Expires</Th>
                </tr>
              </thead>
              <tbody>
                {contract.blocks.map((block) => (
                  <Tr key={block.id}>
                    <Td>{block.label}</Td>
                    <Td className="text-right tabular-nums">{block.total_minutes} min</Td>
                    <Td className="text-right tabular-nums">{money(block.sale_price_minor, contract.currency)}</Td>
                    <Td className="text-right tabular-nums">
                      {block.block_rate_minor !== null && block.block_rate_minor !== undefined
                        ? `${money(block.block_rate_minor, contract.currency)}/h`
                        : "card rate"}
                    </Td>
                    <Td>{block.rollover_policy}</Td>
                    <Td>{block.expires_on ?? "—"}</Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
          <div className="p-4">
            <BlockSpecEditor
              key={`blocks:${contract.updated_at}`}
              existing={contract.blocks.map((b) => ({
                label: b.label,
                total_minutes: String(b.total_minutes),
                sale: b.sale_price_minor === null || b.sale_price_minor === undefined ? "" : String(b.sale_price_minor / 100),
                rate:
                  b.block_rate_minor === null || b.block_rate_minor === undefined
                    ? ""
                    : String(b.block_rate_minor / 100),
                rollover: b.rollover_policy as "expire" | "rollover" | "none",
                expires: b.expires_on ?? "",
              }))}
              onApply={(rows) =>
                void save({
                  blocks: rows.map<BlockSpec>((r) => ({
                    label: r.label,
                    total_minutes: Math.round(Number(r.total_minutes)),
                    sale_price_minor: Math.round(Number(r.sale) * 100),
                    rollover_policy: r.rollover,
                    block_rate_minor: r.rate === "" ? null : Math.round(Number(r.rate) * 100),
                    expires_on: r.expires === "" ? null : r.expires,
                  })),
                })
              }
              saving={saving}
            />
          </div>
        </Card>
      ) : null}

      {isTm ? (
        <Card data-testid="contract-caps">
          <h2 className="p-4 pb-0 text-sm font-semibold">Not-to-exceed caps</h2>
          <p className="p-4 pb-0 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            A ceiling on billable work, and what happens when it is reached. A cap is never silent:
            each one says whether billing stops, a warning rides the evaluation, or the overage
            bills at its own rate.
          </p>
          {contract.caps.length === 0 ? (
            <div className="p-4">
              <EmptyState message="No caps. Labor bills in full however high the period runs." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>Scope</Th>
                  <Th>Work type</Th>
                  <Th className="text-right">Cap</Th>
                  <Th>At the cap</Th>
                  <Th className="text-right">Overage rate</Th>
                </tr>
              </thead>
              <tbody>
                {contract.caps.map((cap) => (
                  <Tr key={cap.id}>
                    <Td>{cap.scope === "period" ? "Per period" : "Whole contract"}</Td>
                    <Td>{cap.work_type ?? "all labor"}</Td>
                    <Td className="text-right tabular-nums">{money(cap.cap_minor, contract.currency)}</Td>
                    <Td>
                      {cap.reached_behavior === "stop"
                        ? "stops billing"
                        : cap.reached_behavior === "warn"
                          ? "warns only"
                          : "bills the overage"}
                    </Td>
                    <Td className="text-right tabular-nums">
                      {cap.overage_rate_minor !== null && cap.overage_rate_minor !== undefined
                        ? `${money(cap.overage_rate_minor, contract.currency)}/h`
                        : "—"}
                    </Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
          <div className="p-4">
            <CapSpecEditor
              key={`caps:${contract.updated_at}`}
              existing={contract.caps.map((c) => ({
                scope: c.scope as "period" | "contract",
                cap: String(c.cap_minor / 100),
                behavior: c.reached_behavior as "stop" | "warn" | "overage_billable",
                workType: c.work_type ?? "",
                overage:
                  c.overage_rate_minor === null || c.overage_rate_minor === undefined
                    ? ""
                    : String(c.overage_rate_minor / 100),
              }))}
              onApply={(rows) =>
                void save({
                  caps: rows.map<CapSpec>((r) => ({
                    scope: r.scope,
                    cap_minor: Math.round(Number(r.cap) * 100),
                    reached_behavior: r.behavior,
                    work_type: r.workType === "" ? null : r.workType,
                    overage_rate_minor: r.overage === "" ? null : Math.round(Number(r.overage) * 100),
                  })),
                })
              }
              saving={saving}
            />
          </div>
        </Card>
      ) : null}

      <Card data-testid="contract-change-orders">
        <h2 className="p-4 pb-0 text-sm font-semibold">Change orders</h2>
        <p className="p-4 pb-0 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Approved scope changes. A draft prices nothing; an approved order bills as its own line
          inside the window it was approved, and the invoice run marks it billed.
        </p>
        {contract.change_orders.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No change orders on this agreement." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Number</Th>
                <Th>Title</Th>
                <Th className="text-right">Amount</Th>
                <Th>Status</Th>
                <Th>Approved</Th>
                <Th>Billed on</Th>
                <Th />
              </tr>
            </thead>
            <tbody>
              {contract.change_orders.map((order) => (
                <Tr key={order.id}>
                  <Td className="font-mono text-[12px]">{order.number}</Td>
                  <Td>
                    <div className="flex flex-col">
                      <span>{order.title}</span>
                      {order.description ? (
                        <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                          {order.description}
                        </span>
                      ) : null}
                    </div>
                  </Td>
                  <Td className="text-right tabular-nums">{money(order.amount_minor, contract.currency)}</Td>
                  <Td>
                    <Tag tone={orderTone(order.status)}>{order.status}</Tag>
                  </Td>
                  <Td>{order.approved_at?.slice(0, 10) ?? "—"}</Td>
                  <Td>
                    {order.billed_invoice_id === null || order.billed_invoice_id === undefined ? (
                      "—"
                    ) : (
                      <Link href={`/billing/${order.billed_invoice_id}`} className="underline underline-offset-2">
                        invoice
                      </Link>
                    )}
                  </Td>
                  <Td>
                    {order.status === "draft" ? (
                      <div className="flex items-center gap-1">
                        <Button size="sm" disabled={childBusy} onClick={() => void transitionOrder(order, "approve")}>
                          Approve
                        </Button>
                        <Button size="sm" variant="ghost" disabled={childBusy} onClick={() => void transitionOrder(order, "reject")}>
                          Reject
                        </Button>
                        <Button size="sm" variant="ghost" disabled={childBusy} onClick={() => void transitionOrder(order, "cancel")}>
                          Cancel
                        </Button>
                      </div>
                    ) : order.status === "approved" ? (
                      <Button size="sm" variant="ghost" disabled={childBusy} onClick={() => void transitionOrder(order, "cancel")}>
                        Cancel
                      </Button>
                    ) : null}
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
        <div className="grid gap-3 p-4">
          <div className="grid grid-cols-3 gap-2">
            <div>
              <Label htmlFor="co-number">Number</Label>
              <Input
                id="co-number"
                value={orderDraft.number}
                placeholder="CO-2026-004"
                onChange={(e) => setOrderDraft({ ...orderDraft, number: e.target.value })}
              />
            </div>
            <div>
              <Label htmlFor="co-title">Title</Label>
              <Input
                id="co-title"
                value={orderDraft.title}
                placeholder="Additional controllers"
                onChange={(e) => setOrderDraft({ ...orderDraft, title: e.target.value })}
              />
            </div>
            <div>
              <Label htmlFor="co-amount">Amount (USD)</Label>
              <Input
                id="co-amount"
                type="number"
                min={0}
                step="0.01"
                value={orderDraft.amount}
                onChange={(e) => setOrderDraft({ ...orderDraft, amount: e.target.value })}
              />
            </div>
          </div>
          <div>
            <Label htmlFor="co-description">Description (optional)</Label>
            <Input
              id="co-description"
              value={orderDraft.description}
              onChange={(e) => setOrderDraft({ ...orderDraft, description: e.target.value })}
            />
          </div>
          <div>
            <Button
              onClick={() => void createChangeOrder()}
              disabled={childBusy || orderDraft.number.trim() === "" || orderDraft.title.trim() === "" || orderDraft.amount.trim() === ""}
              data-testid="create-change-order"
            >
              {childBusy ? <Spinner /> : null} Add change order
            </Button>
          </div>
        </div>
      </Card>

      <Card data-testid="contract-sla-credits">
        <h2 className="p-4 pb-0 text-sm font-semibold">Service credits</h2>
        <p className="p-4 pb-0 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          What the agreement owes back when a target was missed. A pending credit whose date falls
          inside a run&apos;s window becomes a negative line; the run then marks it applied, and an
          applied credit is immutable — issue a new one to correct it.
        </p>
        {contract.sla_credits.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No service credits issued." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Reason</Th>
                <Th className="text-right">Amount</Th>
                <Th>Applies on</Th>
                <Th>Status</Th>
                <Th />
              </tr>
            </thead>
            <tbody>
              {contract.sla_credits.map((credit) => (
                <Tr key={credit.id}>
                  <Td>{credit.reason}</Td>
                  <Td className="text-right tabular-nums">{money(credit.amount_minor, credit.currency)}</Td>
                  <Td>{credit.applies_on}</Td>
                  <Td>
                    <Tag tone={creditTone(credit.status)}>{credit.status}</Tag>
                  </Td>
                  <Td>
                    {credit.status === "pending" ? (
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={childBusy}
                        onClick={() => void cancelCredit(credit)}
                      >
                        Cancel
                      </Button>
                    ) : null}
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
        <div className="grid gap-3 p-4">
          <div className="grid grid-cols-3 gap-2">
            <div>
              <Label htmlFor="sla-reason">What was missed</Label>
              <Input
                id="sla-reason"
                value={creditDraft.reason}
                placeholder="resolution target breached by 2d 4h"
                onChange={(e) => setCreditDraft({ ...creditDraft, reason: e.target.value })}
              />
            </div>
            <div>
              <Label htmlFor="sla-amount">Amount (USD)</Label>
              <Input
                id="sla-amount"
                type="number"
                min={0}
                step="0.01"
                value={creditDraft.amount}
                onChange={(e) => setCreditDraft({ ...creditDraft, amount: e.target.value })}
              />
            </div>
            <div>
              <Label htmlFor="sla-applies">Applies on</Label>
              <Input
                id="sla-applies"
                type="date"
                value={creditDraft.appliesOn}
                onChange={(e) => setCreditDraft({ ...creditDraft, appliesOn: e.target.value })}
              />
              <FieldHint>The billing window this credit lands in.</FieldHint>
            </div>
          </div>
          <div>
            <Button
              onClick={() => void createCredit()}
              disabled={childBusy || creditDraft.reason.trim() === "" || creditDraft.amount.trim() === "" || creditDraft.appliesOn === ""}
              data-testid="create-sla-credit"
            >
              {childBusy ? <Spinner /> : null} Issue credit
            </Button>
          </div>
        </div>
      </Card>

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
          </>
        }
        summary={
          <>
            <p
              className="mb-1 text-[11px] font-semibold uppercase tracking-[0.08em]"
              style={{ color: "var(--m-text-muted)" }}
            >
              Right now
            </p>
            <SummaryCard title="Status">
              <RecordRow label="Status">
                <Tag tone={contractStatusTone(contract.status)}>{contract.status}</Tag>
              </RecordRow>
              <RecordRow label="Type">{contract.contract_type.replaceAll("_", " ")}</RecordRow>
              <RecordRow label="Currency">{contract.currency}</RecordRow>
            </SummaryCard>
            <SummaryCard title="Renewal">
              {recordRenewal === null ? (
                <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  {contract.effective_until == null || contract.effective_until === ""
                    ? "Open-ended: there is nothing to renew."
                    : "Nothing due inside the renewal window."}
                </span>
              ) : (
                <>
                  <RecordRow label="Term ends">
                    {formatPackDate(contract.effective_until)}
                  </RecordRow>
                  <RecordRow label="Signal">
                    <Badge tone={recordRenewal.tone === "danger" ? "danger" : "due"}>
                      <span className="inline-flex items-center gap-1">
                        <ClockGlyph />
                        {recordRenewal.label}
                      </span>
                    </Badge>
                  </RecordRow>
                  {recordQuoteDue !== null ? (
                    <RecordRow label="Quote by">{formatPackDate(recordQuoteDue)}</RecordRow>
                  ) : null}
                </>
              )}
            </SummaryCard>
            <SummaryCard title="Billing">
              <RecordRow label="Per period">
                {contract.amount_minor != null
                  ? money(contract.amount_minor, contract.currency)
                  : "Not priced"}
              </RecordRow>
              <RecordRow label="Frequency">{contract.billing_frequency ?? "—"}</RecordRow>
              <Link
                href="/billing"
                className="text-[13px] underline underline-offset-2"
                style={{ color: "var(--m-text-secondary)" }}
              >
                Open billing →
              </Link>
            </SummaryCard>
          </>
        }
      />
    </div>
  );
}

type BlockRowState = {
  label: string;
  total_minutes: string;
  sale: string;
  rate: string;
  rollover: "expire" | "rollover" | "none";
  expires: string;
};

type CapRowState = {
  scope: "period" | "contract";
  cap: string;
  behavior: "stop" | "warn" | "overage_billable";
  workType: string;
  overage: string;
};

const EMPTY_BLOCK: BlockRowState = {
  label: "",
  total_minutes: "",
  sale: "",
  rate: "",
  rollover: "expire",
  expires: "",
};

const EMPTY_CAP: CapRowState = {
  scope: "period",
  cap: "",
  behavior: "warn",
  workType: "",
  overage: "",
};

function orderTone(status: string): "success" | "warning" | "danger" | "neutral" {
  if (status === "approved") return "success";
  if (status === "draft") return "neutral";
  if (status === "billed") return "warning";
  return "danger";
}

function creditTone(status: string): "success" | "warning" | "danger" | "neutral" {
  if (status === "applied") return "success";
  if (status === "pending") return "warning";
  return "neutral";
}

/** Blocks and caps are child rows: PATCH replaces the whole set, so the
 *  editor opens on what is stored and writes the set the admin leaves
 *  behind. A save with an empty set is a real removal, and the button says
 *  so rather than silently doing nothing. */
function BlockSpecEditor({
  existing,
  onApply,
  saving,
}: {
  existing: BlockRowState[];
  onApply: (rows: BlockRowState[]) => void;
  saving: boolean;
}) {
  const [rows, setRows] = useState<BlockRowState[]>(existing);
  const complete = rows.every((r) => r.label.trim() !== "" && r.total_minutes !== "" && r.sale !== "");
  return (
    <div className="grid gap-3">
      {rows.map((row, i) => (
        <div
          key={i}
          className="flex flex-wrap items-end gap-2"
        >
          {/* Each control sits in a sized wrapper: the Input primitive is
              w-full, so a width class on the input itself loses the
              cascade and every field would take a whole line. */}
          <div className="min-w-40 flex-1">
            <FieldHint>Label</FieldHint>
            <Input
              aria-label="Block label"
              placeholder="50-hour block, March"
              value={row.label}
              onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, label: e.target.value } : r)))}
            />
          </div>
          <div className="w-24">
            <FieldHint>Minutes</FieldHint>
            <Input
              aria-label="Block minutes"
              type="number"
              min={1}
              step={15}
              value={row.total_minutes}
              onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, total_minutes: e.target.value } : r)))}
            />
          </div>
          <div className="w-32">
            <FieldHint>Sale price</FieldHint>
            <Input
              aria-label="Block sale price (USD)"
              type="number"
              min={0}
              step="0.01"
              value={row.sale}
              onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, sale: e.target.value } : r)))}
            />
          </div>
          <div className="w-32">
            <FieldHint>Block rate</FieldHint>
            <Input
              aria-label="Block draw-down rate (USD/hour)"
              type="number"
              min={0}
              step="0.01"
              placeholder="card rate"
              value={row.rate}
              onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, rate: e.target.value } : r)))}
            />
          </div>
          <div className="w-40">
            <FieldHint>Unused</FieldHint>
            <Select
              value={row.rollover}
              onValueChange={(v) =>
                setRows(
                  rows.map((r, j) =>
                    j === i ? { ...r, rollover: v as BlockRowState["rollover"] } : r,
                  ),
                )
              }
            >
              <SelectTrigger aria-label="Rollover policy" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="expire">lapse at expiry</SelectItem>
                <SelectItem value="rollover">roll forward</SelectItem>
                <SelectItem value="none">hold</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="w-36">
            <FieldHint>Expires</FieldHint>
            <Input
              aria-label="Block expires on"
              type="date"
              value={row.expires}
              onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, expires: e.target.value } : r)))}
            />
          </div>
          <Button variant="ghost" size="sm" onClick={() => setRows(rows.filter((_, j) => j !== i))}>
            Remove
          </Button>
        </div>
      ))}
      <div className="flex items-center gap-2">
        <Button variant="secondary" size="sm" onClick={() => setRows([...rows, EMPTY_BLOCK])}>
          Add block
        </Button>
        <Button size="sm" disabled={saving || !complete} onClick={() => onApply(rows)} data-testid="apply-blocks">
          {saving ? <Spinner /> : null} {rows.length === 0 ? "Remove all blocks" : "Replace blocks"}
        </Button>
      </div>
    </div>
  );
}

function CapSpecEditor({
  existing,
  onApply,
  saving,
}: {
  existing: CapRowState[];
  onApply: (rows: CapRowState[]) => void;
  saving: boolean;
}) {
  const [rows, setRows] = useState<CapRowState[]>(existing);
  const complete = rows.every((r) => r.cap !== "");
  return (
    <div className="grid gap-3">
      {rows.map((row, i) => (
        <div key={i} className="flex flex-wrap items-end gap-2">
          <div className="w-40">
            <FieldHint>Scope</FieldHint>
            <Select
              value={row.scope}
              onValueChange={(v) =>
                setRows(rows.map((r, j) => (j === i ? { ...r, scope: v as CapRowState["scope"] } : r)))
              }
            >
              <SelectTrigger aria-label="Cap scope" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="period">Per period</SelectItem>
                <SelectItem value="contract">Whole contract</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="w-32">
            <FieldHint>Cap</FieldHint>
            <Input
              aria-label="Cap amount (USD)"
              type="number"
              min={0}
              step="0.01"
              value={row.cap}
              onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, cap: e.target.value } : r)))}
            />
          </div>
          <div className="w-48">
            <FieldHint>At the cap</FieldHint>
            <Select
              value={row.behavior}
              onValueChange={(v) =>
                setRows(
                  rows.map((r, j) => (j === i ? { ...r, behavior: v as CapRowState["behavior"] } : r)),
                )
              }
            >
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
          <div className="w-40">
            <FieldHint>Work type</FieldHint>
            <Input
              aria-label="Cap work type (optional)"
              placeholder="all labor"
              value={row.workType}
              onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, workType: e.target.value } : r)))}
            />
          </div>
          <div className="w-32">
            <FieldHint>Overage rate</FieldHint>
            <Input
              aria-label="Overage rate (USD/hour)"
              type="number"
              min={0}
              step="0.01"
              placeholder="card rate"
              value={row.overage}
              onChange={(e) => setRows(rows.map((r, j) => (j === i ? { ...r, overage: e.target.value } : r)))}
            />
          </div>
          <Button variant="ghost" size="sm" onClick={() => setRows(rows.filter((_, j) => j !== i))}>
            Remove
          </Button>
        </div>
      ))}
      <div className="flex items-center gap-2">
        <Button variant="secondary" size="sm" onClick={() => setRows([...rows, EMPTY_CAP])}>
          Add cap
        </Button>
        <Button size="sm" disabled={saving || !complete} onClick={() => onApply(rows)} data-testid="apply-caps">
          {saving ? <Spinner /> : null} {rows.length === 0 ? "Remove all caps" : "Replace caps"}
        </Button>
      </div>
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

/** The contract's terms as figures. The value figure is the one that earns
 * the larger type, because it is the number an account manager is asked for. */
function Figure({ label, value, emphasis = false }: { label: string; value: string; emphasis?: boolean }) {
  return (
    <div>
      <p
        className="text-[11px] font-semibold uppercase tracking-[0.08em]"
        style={{ color: "var(--m-text-muted)" }}
      >
        {label}
      </p>
      <p
        className="mt-0.5 font-semibold tabular-nums"
        style={{ fontSize: emphasis ? "20px" : "13px" }}
      >
        {value}
      </p>
    </div>
  );
}

function shortFreq(f: string | null | undefined): string {
  switch (f) {
    case "monthly": return "month";
    case "quarterly": return "quarter";
    case "semiannual": return "half year";
    case "annual": return "year";
    case "weekly": return "week";
    default: return "period";
  }
}

function billingDay(anchor: string | null | undefined): string {
  if (anchor == null || anchor === "") return "1st";
  const day = Number(anchor.slice(8, 10));
  if (!Number.isFinite(day) || day < 1) return "1st";
  if (day === 1) return "1st";
  if (day === 2) return "2nd";
  if (day === 3) return "3rd";
  return `${day}th`;
}

/** The clock that pairs with every renewal word. Colour is never alone, so
 * the pill carries an icon and the number as well as the tone. */
function ClockGlyph() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </svg>
  );
}
