// /billing/[id] — one invoice (Wave 9 lane 1 close-out).
//
// Purpose: the invoice aggregate: document header, lines, payment
// receipts, dunning posture, and the actions that write through the
// instant-first envelope — record payment (the `none` provider approves
// locally and is presented as ledger-only), forward-only dunning
// transitions, void, and the print-ready HTML document.
//
// Non-purpose: no editing of issued lines; the document is an artifact of
// the invoice run's contract evaluation.
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
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
import { useParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { api, apiBaseUrl, loadTokens, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { money } from "../../../lib/format";

type InvoiceDetail = components["schemas"]["InvoiceDetail"];
type DunningState = components["schemas"]["DunningState"];

const DUNNING_LADDER: DunningState[] = [
  "none",
  "reminder_sent",
  "overdue",
  "final_notice",
  "collections",
];

export default function InvoiceDetailPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const invoiceId = params.id;

  const [invoice, setInvoice] = useState<InvoiceDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [payOpen, setPayOpen] = useState(false);
  const [payAmount, setPayAmount] = useState("");
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

  const outstanding = invoice === null ? 0 : invoice.total_minor - invoice.total_paid_minor;

  const recordPayment = async () => {
    if (invoice === null) return;
    const amount = Number(payAmount);
    if (!Number.isFinite(amount) || amount <= 0) {
      setPayError("Enter an amount greater than zero.");
      return;
    }
    const amountMinor = Math.round(amount * 100);
    if (amountMinor > outstanding) {
      setPayError("The amount exceeds the outstanding balance.");
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
              provider_ref: null,
              received_at: new Date().toISOString(),
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
      request: (client, body) =>
        client.POST("/api/v1/invoices/{id}/payments", {
          params: { path: { id: invoice.id } },
          body,
        }),
      versions: (response) => [[invoice.id, response.mutation.version]],
    });
    setPayBusy(false);
    if (!result.ok) {
      setPayError(result.message ?? "The payment was rejected.");
      return;
    }
    setPayOpen(false);
    setPayAmount("");
    setPayNotes("");
    setActionNote("Payment recorded (ledger-only — the `none` provider approves locally).");
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
      request: (client, body) =>
        client.POST("/api/v1/invoices/{id}/dunning", {
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
          : (result.message ?? "The transition was rejected."),
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
      request: (client, body) =>
        client.POST("/api/v1/invoices/{id}/void", {
          params: { path: { id: invoice.id } },
          body,
        }),
      versions: (response) => [[invoice.id, response.mutation.version]],
    });
    setVoidBusy(false);
    if (!result.ok) {
      setActionNote(result.message ?? "Void was rejected.");
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
      <div className="grid gap-4 p-8">
        {loadError !== null ? (
          <p role="alert" className="text-[13px]">{loadError}</p>
        ) : (
          <Spinner />
        )}
      </div>
    );
  }

  const ladderIndex = DUNNING_LADDER.indexOf(invoice.dunning_state);
  const forwardStates = DUNNING_LADDER.slice(ladderIndex + 1);

  return (
    <div className="grid gap-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold tabular-nums">{invoice.number}</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Issued {invoice.issued_at} · Due {invoice.due_on}
            {invoice.period_from !== null && invoice.period_to !== null
              ? ` · Period ${invoice.period_from} → ${invoice.period_to}`
              : ""}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Tag>{invoice.status}</Tag>
          {invoice.dunning_state !== "none" ? <Tag>{invoice.dunning_state}</Tag> : null}
          <Button variant="secondary" onClick={() => void openPrintDocument()}>
            Print document
          </Button>
          <Link
            href={`/billing/credit-notes?client=${invoice.client_id}&invoice=${invoice.id}`}
            className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
            style={{ color: "var(--m-text-secondary)" }}
          >
            Issue credit note
          </Link>
          {invoice.status === "issued" ? (
            <>
              <Dialog open={payOpen} onOpenChange={(o) => { setPayOpen(o); setPayError(null); }}>
                <DialogTrigger asChild>
                  <Button>Record payment</Button>
                </DialogTrigger>
                <DialogContent
                  title="Record payment"
                  description="The `none` payment provider approves locally — receipts are ledger entries only, no external charge is made."
                >
                  <div className="grid gap-3 p-4">
                    <div>
                      <Label htmlFor="pay-amount">Amount ({invoice.currency})</Label>
                      <Input
                        id="pay-amount"
                        type="number"
                        min={0}
                        step="0.01"
                        value={payAmount}
                        onChange={(e) => setPayAmount(e.target.value)}
                      />
                      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                        Outstanding {money(outstanding, invoice.currency)}
                      </p>
                    </div>
                    <div>
                      <Label htmlFor="pay-notes">Notes</Label>
                      <Input
                        id="pay-notes"
                        value={payNotes}
                        onChange={(e) => setPayNotes(e.target.value)}
                      />
                    </div>
                    {payError !== null ? (
                      <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                        {payError}
                      </p>
                    ) : null}
                    <DialogActions>
                      <Button variant="ghost" onClick={() => setPayOpen(false)}>
                        Cancel
                      </Button>
                      <Button onClick={() => void recordPayment()} disabled={payBusy}>
                        {payBusy ? <Spinner /> : null} Record
                      </Button>
                    </DialogActions>
                  </div>
                </DialogContent>
              </Dialog>
              <Button variant="ghost" onClick={() => void voidInvoice()} disabled={voidBusy}>
                Void
              </Button>
            </>
          ) : null}
        </div>
      </div>

      {actionNote !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {actionNote}
        </p>
      ) : null}

      <Card>
        <h2 className="p-4 pb-0 text-sm font-semibold">Lines</h2>
        <Table>
          <thead>
            <tr>
              <Th>Description</Th>
              <Th>Kind</Th>
              <Th>Period</Th>
              <Th className="text-right">Qty</Th>
              <Th className="text-right">Amount</Th>
            </tr>
          </thead>
          <tbody>
            {invoice.lines.map((line) => (
              <Tr key={line.id}>
                <Td>{line.description}</Td>
                <Td>
                  <Tag>{line.kind}</Tag>
                </Td>
                <Td>
                  {line.period_start} → {line.period_end}
                </Td>
                <Td className="text-right tabular-nums">
                  {line.quantity_minutes > 0
                    ? `${(line.quantity_minutes / 60).toFixed(1)} h`
                    : "—"}
                </Td>
                <Td className="text-right tabular-nums">
                  {money(line.amount_minor, invoice.currency)}
                </Td>
              </Tr>
            ))}
            <Tr>
              <Td colSpan={4} className="text-right font-semibold">
                Total
              </Td>
              <Td className="text-right tabular-nums font-semibold">
                {money(invoice.total_minor, invoice.currency)}
              </Td>
            </Tr>
          </tbody>
        </Table>
      </Card>

      <Card>
        <h2 className="p-4 pb-0 text-sm font-semibold">Payments</h2>
        {invoice.payments.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No payments recorded yet." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Received</Th>
                <Th>Provider</Th>
                <Th>Notes</Th>
                <Th className="text-right">Amount</Th>
              </tr>
            </thead>
            <tbody>
              {invoice.payments.map((p) => (
                <Tr key={p.id}>
                  <Td>{new Date(p.received_at).toLocaleString()}</Td>
                  <Td>
                    <Tag>{p.provider === "none" ? "none (ledger-only)" : p.provider}</Tag>
                  </Td>
                  <Td>{p.notes ?? "—"}</Td>
                  <Td className="text-right tabular-nums">{money(p.amount_minor, p.currency)}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>

      <Card>
        <h2 className="p-4 pb-0 text-sm font-semibold">Collections</h2>
        <div className="grid gap-3 p-4">
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Current state <Tag>{invoice.dunning_state}</Tag>
            {invoice.dunning_note !== null ? ` — ${invoice.dunning_note}` : ""}
            {invoice.dunning_updated_at != null
              ? ` (updated ${new Date(invoice.dunning_updated_at).toLocaleString()})`
              : ""}
          </p>
          {invoice.status === "issued" ? (
            forwardStates.length > 0 ? (
              <div className="flex items-end gap-3">
                <div>
                  <Label htmlFor="dunning-target">Advance to</Label>
                  <Select
                    value={dunningTarget}
                    onValueChange={(v) => setDunningTarget(v as DunningState)}
                  >
                    <SelectTrigger id="dunning-target" className="w-44" aria-label="Advance dunning to">
                      <SelectValue placeholder="Choose state" />
                    </SelectTrigger>
                    <SelectContent>
                      {forwardStates.map((s) => (
                        <SelectItem key={s} value={s}>
                          {s}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="flex-1">
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
          {invoice.status === "issued" ? (
            <div className="grid gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }} data-testid="send-statement">
              <div className="flex items-center gap-2">
                <Button variant="secondary" onClick={() => void sendStatement()} disabled={statementBusy}>
                  {statementBusy ? <Spinner /> : null} Send statement
                </Button>
                <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  Emails the client a statement of every open invoice. None-mode transport keeps it queued.
                </span>
              </div>
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
            </div>
          ) : null}
        </div>
      </Card>
    </div>
  );
}
