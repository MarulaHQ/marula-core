// /billing/[id] — one invoice, as the money cluster's document surface (Wave 9
// lane 1 close-out; Wave 17 lane 5 UX pass; Wave 41 lane 8 rebuilt to the
// Marula surface pack's approvals reference, states 08/09/10).
//
// Purpose: the invoice aggregate, where approved time becomes a number someone
// trusts: the Lines card with its link back to the entries that built it, the
// Subtotal / Paid / Balance due block, and — beside it — the three fact cards
// (Client, Payments, Activity). The actions sit in the header in order of risk:
// Send reminder, Issue credit, then the primary Record payment, and recording a
// payment says out loud what it will do before it is done.
//
// The colour of a money status means one thing (pack §7): Draft neutral, Sent
// gold tint, Paid green with a check, and Overdue BLUSH with the days in words.
// Red on this screen would claim a breach that has not happened; it stays with
// a breached SLA.
//
// Non-purpose: no editing of issued lines; the document is an artifact of
// the invoice run's contract evaluation. Issued documents keep the
// locked_after_invoicing read-only posture — only ledger-level actions
// (payments, dunning, void) mutate, never the lines.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import { ErrorNote } from "../../../components/knowledge/ledger";
import {
  Badge,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  FieldHint,
  Input,
  Kbd,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import { LocalTime } from "../../../components/local-time";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, apiBaseUrl, loadTokens, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";
import {
  ageingBucket,
  ageingLabel,
  invoicePill,
  overBalanceMessage,
  payNote,
} from "../../../lib/money-ui";
import {
  AgeingPill,
  MoneyCard,
  MoneyHeader,
  MoneyKv,
  MoneyNote,
  MoneyPillTag,
} from "../../../components/money";

type InvoiceDetail = components["schemas"]["InvoiceDetail"];
type Client = components["schemas"]["Client"];
type DunningState = components["schemas"]["DunningState"];

const DUNNING_LADDER: DunningState[] = [
  "none",
  "reminder_sent",
  "overdue",
  "final_notice",
  "collections",
];

const STATUS_COPY: Record<DunningState, string> = {
  none: "Current on payment",
  reminder_sent: "Reminder sent",
  overdue: "Payment overdue",
  final_notice: "Final notice",
  collections: "In collections",
};

const PAYMENT_METHODS = ["EFT", "Card", "Cash", "Other"] as const;

function todayIso(): string {
  const now = new Date();
  const local = new Date(now.getTime() - now.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 10);
}

export default function InvoiceDetailPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const invoiceId = params.id;

  const [invoice, setInvoice] = useState<InvoiceDetail | null>(null);
  const [client, setClient] = useState<Client | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [payOpen, setPayOpen] = useState(false);
  const [payAmount, setPayAmount] = useState("");
  const [payDate, setPayDate] = useState(todayIso());
  const [payMethod, setPayMethod] = useState<string>(PAYMENT_METHODS[0]);
  const [payNotes, setPayNotes] = useState("");
  const [payBusy, setPayBusy] = useState(false);
  const [payError, setPayError] = useState<string | null>(null);

  const [dunningTarget, setDunningTarget] = useState<DunningState | "">("");
  const [dunningNote, setDunningNote] = useState("");
  const [dunningBusy, setDunningBusy] = useState(false);
  const [dunningError, setDunningError] = useState<string | null>(null);

  const [voidBusy, setVoidBusy] = useState(false);
  const [actionNote, setActionNote] = useState<string | null>(null);

  const [statementBusy, setStatementBusy] = useState(false);
  const [statementNote, setStatementNote] = useState<string | null>(null);
  const [statementError, setStatementError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/invoices/{id}", {
      params: { path: { id: invoiceId } },
    });
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load this invoice.");
      return;
    }
    setInvoice(data);
    setLoadError(null);
  }, [invoiceId]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // The invoice carries a client id, not a name. One directory read names it.
  const clientId = invoice?.client_id ?? null;
  useEffect(() => {
    if (clientId === null) return;
    let cancelled = false;
    void api.api
      .GET("/api/v1/clients")
      .then(({ data }) => {
        if (cancelled) return;
        setClient((data ?? []).find((c) => c.id === clientId) ?? null);
      })
      .catch(() => {
        if (!cancelled) setClient(null);
      });
    return () => {
      cancelled = true;
    };
  }, [clientId]);

  const outstanding = invoice === null ? 0 : invoice.total_minor - invoice.total_paid_minor;

  /** The typed amount the dialog is describing, in minor units. */
  const payAmountMinor = useMemo(() => {
    const major = Number(payAmount.replace(/[^0-9.]/g, ""));
    if (!Number.isFinite(major)) return 0;
    return Math.round(major * 100);
  }, [payAmount]);
  /** The live note: closed or still open, with the new balance spelled out. */
  const payPreview =
    invoice === null
      ? null
      : payNote(payAmountMinor, outstanding, (minor) => money(minor, invoice.currency));
  const payBlock =
    invoice === null
      ? null
      : overBalanceMessage(payAmountMinor, outstanding, (minor) => money(minor, invoice.currency));

  const recordPayment = async () => {
    if (invoice === null) return;
    const amountMinor = payAmountMinor;
    if (amountMinor <= 0) {
      setPayError("Enter the amount received.");
      return;
    }
    if (amountMinor > outstanding) {
      setPayError(
        `That is more than the balance of ${money(outstanding, invoice.currency)}. Record the balance, then a credit for the rest.`,
      );
      return;
    }
    setPayBusy(true);
    setPayError(null);
    const before = invoice;
    const result = await runEnvelopeMutation<
      components["schemas"]["RecordPaymentRequest"],
      components["schemas"]["PaymentMutationResponse"]
    >({
      describe: "Record payment",
      entityId: invoice.id,
      optimistic: () =>
        setInvoice({
          ...invoice,
          total_paid_minor: invoice.total_paid_minor + amountMinor,
          payments: [
            ...invoice.payments,
            {
              id: "pending",
              org_id: invoice.org_id,
              invoice_id: invoice.id,
              amount_minor: amountMinor,
              currency: invoice.currency,
              provider: "none",
              provider_ref: payNotes === "" ? null : payNotes,
              received_at: new Date(`${payDate}T00:00:00`).toISOString(),
              created_by: null,
              notes: payNotes === "" ? null : payNotes,
            },
          ],
        }),
      revert: () => setInvoice(before),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        amount_minor: amountMinor,
        notes: payNotes === "" ? null : payNotes,
      }),
      request: (client_, body) =>
        client_.POST("/api/v1/invoices/{id}/payments", {
          params: { path: { id: invoice.id } },
          body,
        }),
      versions: (response) => [[invoice.id, response.mutation.version]],
    });
    setPayBusy(false);
    if (!result.ok) {
      setPayError(rejectionText(result, "The payment was rejected."));
      return;
    }
    setPayOpen(false);
    setPayAmount("");
    setPayNotes("");
    setActionNote(
      `Recorded ${money(amountMinor, invoice.currency)} against ${invoice.number}. ${
        amountMinor >= outstanding
          ? "The invoice is paid."
          : "The invoice stays open."
      } A payment can be reversed from the invoice.`,
    );
    void refresh();
  };

  const advanceDunning = async () => {
    if (invoice === null || dunningTarget === "") return;
    setDunningBusy(true);
    setDunningError(null);
    const before = invoice;
    const result = await runEnvelopeMutation<
      components["schemas"]["DunningTransitionRequest"],
      components["schemas"]["InvoiceMutationResponse"]
    >({
      describe: "Advance dunning",
      entityId: invoice.id,
      optimistic: () =>
        setInvoice({
          ...invoice,
          dunning_state: dunningTarget,
          dunning_note: dunningNote === "" ? null : dunningNote,
          dunning_updated_at: new Date().toISOString(),
        }),
      revert: () => setInvoice(before),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        to_state: dunningTarget,
        note: dunningNote === "" ? null : dunningNote,
      }),
      request: (client_, body) =>
        client_.POST("/api/v1/invoices/{id}/dunning", {
          params: { path: { id: invoice.id } },
          body,
        }),
      versions: (response) => [[invoice.id, response.mutation.version]],
    });
    setDunningBusy(false);
    if (!result.ok) {
      setDunningError(
        result.reason === "dunning_not_forward"
          ? "Dunning moves forward only."
          : (rejectionText(result, "The transition was rejected.")),
      );
      return;
    }
    setDunningTarget("");
    setDunningNote("");
    void refresh();
  };

  const voidInvoice = async () => {
    if (invoice === null) return;
    setVoidBusy(true);
    const before = invoice;
    const result = await runEnvelopeMutation<
      components["schemas"]["VoidInvoiceRequest"],
      components["schemas"]["InvoiceMutationResponse"]
    >({
      describe: "Void invoice",
      entityId: invoice.id,
      optimistic: () => setInvoice({ ...invoice, status: "void" }),
      revert: () => setInvoice(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client_, body) =>
        client_.POST("/api/v1/invoices/{id}/void", {
          params: { path: { id: invoice.id } },
          body,
        }),
      versions: (response) => [[invoice.id, response.mutation.version]],
    });
    setVoidBusy(false);
    if (!result.ok) {
      setActionNote(rejectionText(result, "Void was rejected."));
      return;
    }
    setActionNote("Invoice voided.");
    void refresh();
  };

  const openPrintDocument = async () => {
    const tokens = loadTokens();
    const response = await fetch(
      `${apiBaseUrl()}/api/v1/invoices/${invoiceId}/document`,
      {
        headers: tokens === null ? {} : { authorization: `Bearer ${tokens.access_token}` },
      },
    );
    if (!response.ok) {
      setActionNote("The print document is not available for this invoice.");
      return;
    }
    const html = await response.text();
    const url = URL.createObjectURL(new Blob([html], { type: "text/html" }));
    window.open(url, "_blank", "noopener");
  };

  /** Send-statement dunning action (p11-backend-gaps): generates a statement
   * of the client's open invoices and queues the email to client contacts.
   * In none-mode mail transport the message STAYS QUEUED — surfaced honestly. */
  const sendStatement = async () => {
    if (invoice === null) return;
    setStatementBusy(true);
    setStatementError(null);
    setStatementNote(null);
    const { data, error } = await api.api.POST("/api/v1/invoices/{id}/dunning/statement", {
      params: { path: { id: invoice.id } },
      body: { idempotency_key: `statement-${invoice.id}-${Date.now()}` },
    });
    setStatementBusy(false);
    if (error !== undefined || data === undefined) {
      const body = error as { error?: { code?: number; message?: string } } | undefined;
      setStatementError(
        body?.error?.code === 422
          ? "No contact emails on the client to deliver the statement to."
          : (body?.error?.message ?? "The statement could not be generated."),
      );
      return;
    }
    setStatementNote(
      data.queued_to.length === 0
        ? "Statement generated, but no contact emails were available to queue delivery."
        : `Statement queued for delivery to ${data.queued_to.join(", ")} — ${
            data.open_invoices.length
          } open invoice(s), ${money(data.total_outstanding_minor, data.currency)} outstanding. With the none mail transport it stays queued (no external send).`,
    );
  };

  if (auth.status !== "authed") return null;

  if (invoice === null) {
    return (
      <div className="mx-auto grid w-full max-w-[1400px] gap-4 p-8">
        {loadError !== null ? (
          <ErrorNote>{loadError}</ErrorNote>
        ) : (
          <Spinner />
        )}
      </div>
    );
  }

  const ladderIndex = DUNNING_LADDER.indexOf(invoice.dunning_state);
  const forwardStates = DUNNING_LADDER.slice(ladderIndex + 1);
  const unpaid = invoice.total_minor - invoice.total_paid_minor;
  const pill = invoicePill(invoice.status, invoice.total_paid_minor, invoice.total_minor);
  const bucket = ageingBucket({
    status: invoice.status,
    dueOn: invoice.due_on,
    unpaidMinor: unpaid,
  });
  const late = ageingLabel(invoice.due_on, bucket);
  const fmt = (minor: number): string => money(minor, invoice.currency);
  const isOpen = invoice.status === "issued";

  return (
    <div className="m-money-page mx-auto w-full max-w-[1400px] p-4 md:p-6">
      <nav className="m-money-crumbs" aria-label="Breadcrumb">
        <Link href="/billing">Invoices</Link>
        <span aria-hidden="true">›</span>
        <span>{invoice.number}</span>
      </nav>

      <MoneyHeader
        className="m-money-invhead"
        eyebrow="Invoices"
        nav={false}
        title={invoice.number}
        stats={
          <>
            <MoneyPillTag pill={pill} testId="invoice-status-pill" />
            <AgeingPill bucket={bucket} label={late} testId="invoice-ageing-pill" />
          </>
        }
        actions={
          <div style={{ flex: 1 }}>
          {/* In order of risk: a reminder touches nothing, a credit reverses
              value, a payment closes the invoice. */}
          {isOpen && forwardStates.length > 0 ? (
            <Button variant="secondary" onClick={() => void sendStatement()} disabled={statementBusy}>
              {statementBusy ? <Spinner /> : null} Send reminder
            </Button>
          ) : (
            <Button variant="secondary" onClick={() => void openPrintDocument()}>
              Print document
            </Button>
          )}
          <Link
            href={`/billing/credit-notes?client=${invoice.client_id}&invoice=${invoice.id}`}
            style={{ textDecoration: "none" }}
          >
            <Button variant="secondary">Issue credit</Button>
          </Link>
          <Dialog
            open={payOpen}
            onOpenChange={(o) => {
              setPayOpen(o);
              setPayError(null);
              // The common case is the whole balance arriving at once, so the
              // field opens holding it (pack 09); a partial payment is a trim
              // of a full number, not a typed one.
              setPayAmount(o ? (outstanding / 100).toFixed(2) : "");
            }}
          >
            <DialogTrigger asChild>
              <Button variant="primary" disabled={!isOpen} data-testid="record-payment">
                Record payment
              </Button>
            </DialogTrigger>
            <DialogContent
              title="Record payment"
              description={`${invoice.number}${client === null ? "" : `, ${client.name}`}. Balance due ${fmt(outstanding)}.`}
            >
              <div className="grid gap-3 p-4">
                <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
                  <div>
                    <Label htmlFor="pay-amount">Amount</Label>
                    <Input
                      id="pay-amount"
                      inputMode="decimal"
                      placeholder="0.00"
                      value={payAmount}
                      onChange={(e) => setPayAmount(e.target.value)}
                      aria-invalid={payBlock !== null}
                    />
                    {payBlock !== null ? (
                      <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                        {payBlock}
                      </p>
                    ) : null}
                  </div>
                  <div>
                    <Label htmlFor="pay-date">Received on</Label>
                    <Input
                      id="pay-date"
                      type="date"
                      value={payDate}
                      onChange={(e) => setPayDate(e.target.value)}
                    />
                  </div>
                </div>
                <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
                  <div>
                    <Label htmlFor="pay-method">Method</Label>
                    <Select value={payMethod} onValueChange={setPayMethod}>
                      <SelectTrigger id="pay-method" aria-label="Payment method">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {PAYMENT_METHODS.map((m) => (
                          <SelectItem key={m} value={m}>
                            {m}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div>
                    <Label htmlFor="pay-notes">Reference</Label>
                    <Input
                      id="pay-notes"
                      value={payNotes}
                      onChange={(e) => setPayNotes(e.target.value)}
                      placeholder="Bank reference"
                    />
                    <FieldHint>Optional. Recorded with the payment.</FieldHint>
                  </div>
                </div>
                {payPreview !== null ? <MoneyNote>{payPreview.text}</MoneyNote> : null}
                {payError !== null ? (
                  <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {payError}
                  </p>
                ) : null}
                <FieldHint>
                  Recorded with your name and the time. The `none` payment provider approves
                  locally, so a receipt is a ledger entry and no external charge is made.
                </FieldHint>
                <DialogActions>
                  <Button variant="ghost" onClick={() => setPayOpen(false)}>
                    Cancel
                  </Button>
                  <Button
                    variant="primary"
                    onClick={() => void recordPayment()}
                    disabled={payBusy || payAmountMinor <= 0 || payAmountMinor > outstanding}
                    data-testid="record-payment-save"
                  >
                    {payBusy ? <Spinner /> : null} Record {payAmountMinor > 0 ? fmt(payAmountMinor) : fmt(0)}
                  </Button>
                </DialogActions>
              </div>
            </DialogContent>
          </Dialog>
        </div>
        }
      />

      {actionNote !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {actionNote}
        </p>
      ) : null}
      {statementNote !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {statementNote}
        </p>
      ) : null}
      {statementError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {statementError}
        </p>
      ) : null}

      <div className="m-money-inv">
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          {/* Where the numbers came from: the lines card links back to the
              approved entries an invoice run consumed. */}
          <MoneyCard
            testId="invoice-lines"
            title="Lines"
            action={
              <Link
                href="/approvals"
                className="m-money-linkbtn"
                data-testid="invoice-built-from"
              >
                Built from approved entries
              </Link>
            }
          >
            <div className="m-money-line m-money-line-head">
              <span>Description</span>
              <span className="m-money-line-r">Qty</span>
              <span className="m-money-line-r">Rate</span>
              <span className="m-money-line-r">Amount</span>
            </div>
            {invoice.lines.map((line) => (
              <div className="m-money-line" key={line.id}>
                <span style={{ fontWeight: 500 }}>{line.description}</span>
                <span className="m-money-line-r">
                  {line.quantity_minutes > 0 ? `${(line.quantity_minutes / 60).toFixed(2)} h` : "—"}
                </span>
                <span className="m-money-line-r">{fmt(line.unit_amount_minor)}</span>
                <span className="m-money-line-r">{fmt(line.amount_minor)}</span>
              </div>
            ))}
            <div className="m-money-tot">
              <div>
                <span>Subtotal</span>
                <span>{fmt(invoice.total_minor)}</span>
              </div>
              <div>
                <span>Paid</span>
                <span>{fmt(invoice.total_paid_minor)}</span>
              </div>
              <div className="m-money-tot-bal">
                <span>Balance due</span>
                <span>{fmt(unpaid)}</span>
              </div>
            </div>
          </MoneyCard>

          <MoneyCard title="Payments">
            {invoice.payments.length === 0 ? (
              <p
                className="text-[13px]"
                style={{ color: "var(--m-text-secondary)", padding: "14px 18px", margin: 0 }}
              >
                No payments yet.
              </p>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Received</Th>
                    <Th>Method</Th>
                    <Th>Reference</Th>
                    <Th className="text-right">Amount</Th>
                  </tr>
                </thead>
                <tbody>
                  {invoice.payments.map((p) => (
                    <Tr key={p.id} className="row-hover">
                      <Td className="text-[12px]"><LocalTime value={p.received_at} kind="datetime" /></Td>
                      <Td>
                        <Badge tone="neutral">{p.provider === "none" ? "EFT" : p.provider}</Badge>
                      </Td>
                      <Td>{p.notes ?? "—"}</Td>
 <Td className="text-right text-[12px] tabular-nums">
                        {money(p.amount_minor, p.currency)}
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </MoneyCard>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <MoneyCard title="Client">
            <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: "14px 18px" }}>
              <MoneyKv label="Client">
                {client === null ? (
                  <Link href={`/clients/${invoice.client_id}`} style={{ color: "var(--m-accent-text)" }}>
                    open the client →
                  </Link>
                ) : (
                  <Link href={`/clients/${client.id}`} style={{ color: "var(--m-accent-text)" }}>
                    {client.name}
                  </Link>
                )}
              </MoneyKv>
              <MoneyKv label="Currency">{invoice.currency}</MoneyKv>
              <MoneyKv label="Issued">{invoice.issued_at}</MoneyKv>
              <MoneyKv label="Due">{invoice.due_on}</MoneyKv>
              <MoneyKv label="Period">
                {invoice.period_from} → {invoice.period_to}
              </MoneyKv>
              <MoneyKv label="Posture">{STATUS_COPY[invoice.dunning_state]}</MoneyKv>
            </div>
          </MoneyCard>

          <MoneyCard title="Activity">
            <div style={{ padding: "6px 18px 10px" }} className="m-money-act">
              <div>
                <span className="m-money-act-tm">{invoice.issued_at.slice(0, 10)}</span>
                <span>
                  Issued by the invoice run
                  {invoice.period_from !== invoice.period_to
                    ? ` over ${invoice.period_from} → ${invoice.period_to}`
                    : ""}
                  .
                </span>
              </div>
              {invoice.dunning_updated_at !== null && invoice.dunning_updated_at !== undefined ? (
                <div>
                  <span className="m-money-act-tm">
                    {invoice.dunning_updated_at.slice(0, 10)}
                  </span>
                  <span>
                    {STATUS_COPY[invoice.dunning_state]}
                    {invoice.dunning_note !== null ? ` — “${invoice.dunning_note}”` : ""}.
                  </span>
                </div>
              ) : null}
              {invoice.payments.slice(0, 2).map((p) => (
                <div key={p.id}>
                  <span className="m-money-act-tm">{p.received_at.slice(0, 10)}</span>
                  <span>
                    Payment of {money(p.amount_minor, p.currency)} recorded
                    {p.notes !== null ? ` — “${p.notes}”` : ""}.
                  </span>
                </div>
              ))}
            </div>
          </MoneyCard>

          {/* The collections ladder: forward-only, drawn honestly — steps behind
              stay filled, the current one is outlined, steps ahead stay empty. */}
          <MoneyCard title="Collections">
            <ol
              className="flex flex-wrap gap-1 px-4 pb-1"
              aria-label="Dunning ladder"
              style={{ listStyle: "none", margin: 0 }}
            >
              {DUNNING_LADDER.map((state, i) => {
                const reached = i <= ladderIndex;
                const current = i === ladderIndex;
                return (
                  <li key={state}>
                    <span
                      aria-current={current ? "step" : undefined}
                      className="inline-flex items-center rounded-full px-2.5 py-1 text-[11px] font-semibold"
                      style={{
                        background: reached ? "var(--m-status-warning-bg)" : "transparent",
                        color: reached ? "var(--m-status-warning-fg)" : "var(--m-text-muted)",
                        border: current
                          ? "1px solid var(--m-status-warning-fg)"
                          : "1px solid var(--m-border)",
                      }}
                    >
                      {i === 0 ? "current" : STATUS_COPY[state]}
                    </span>
                  </li>
                );
              })}
            </ol>
            <div className="grid gap-3 p-4">
              {isOpen ? (
                forwardStates.length > 0 ? (
                  <>
                    <div style={{ display: "flex", flexWrap: "wrap", alignItems: "flex-end", gap: 12 }}>
                      <div>
                        <Label htmlFor="dunning-target">Advance to</Label>
                        <Select
                          value={dunningTarget}
                          onValueChange={(v) => setDunningTarget(v as DunningState)}
                        >
                          <SelectTrigger
                            id="dunning-target"
                            style={{ width: 180 }}
                            aria-label="Advance dunning to"
                          >
                            <SelectValue placeholder="Choose state" />
                          </SelectTrigger>
                          <SelectContent>
                            {forwardStates.map((s) => (
                              <SelectItem key={s} value={s}>
                                {STATUS_COPY[s]}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div style={{ minWidth: 200, flex: 1 }}>
                        <Label htmlFor="dunning-note">Note</Label>
                        <Input
                          id="dunning-note"
                          value={dunningNote}
                          onChange={(e) => setDunningNote(e.target.value)}
                        />
                      </div>
                      <Button
                        onClick={() => void advanceDunning()}
                        disabled={dunningBusy || dunningTarget === ""}
                      >
                        {dunningBusy ? <Spinner /> : null} Advance
                      </Button>
                    </div>
                    <div
                      className="grid gap-2 border-t pt-3"
                      style={{ borderColor: "var(--m-border)" }}
                      data-testid="send-statement"
                    >
                      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                        <Button
                          variant="secondary"
                          onClick={() => void sendStatement()}
                          disabled={statementBusy}
                        >
                          {statementBusy ? <Spinner /> : null} Send statement
                        </Button>
                        <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>
                          Emails the client a statement of every open invoice. None-mode transport
                          keeps it queued.
                        </span>
                      </div>
                    </div>
                  </>
                ) : (
                  <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                    The invoice is at the end of the collections ladder.
                  </p>
                )
              ) : (
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  Collections actions apply to issued invoices only.
                </p>
              )}
              {dunningError !== null ? (
                <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                  {dunningError}
                </p>
              ) : null}
              {invoice.dunning_note !== null ? (
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  Last note: “{invoice.dunning_note}”
                  {invoice.dunning_updated_at != null
                    ? (
                        <>
                          {" ("}
                          <LocalTime value={invoice.dunning_updated_at} kind="datetime" />
                          {")"}
                        </>
                      )
                    : ""}
                </p>
              ) : null}
              {isOpen ? (
                <div>
                  <Button variant="ghost" onClick={() => void voidInvoice()} disabled={voidBusy}>
                    Void invoice
                  </Button>
                </div>
              ) : null}
            </div>
          </MoneyCard>

          <p className="m-money-foot" style={{ fontSize: 12 }}>
            <Kbd>P</Kbd>
            <span>prints the read-only document</span>
          </p>
        </div>
      </div>
    </div>
  );
}
