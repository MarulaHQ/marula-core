// /portal/invoices/[id] — one invoice: lines, receipts, and the pay box
// (Portal v2). The document link opens the SAME print-ready HTML staff
// print. Payment rides the PaymentProvider seam in its none-mode: nothing
// is charged externally, the receipt records that the client settled the
// amount — the copy below says exactly that (security copy stays exact).
//
// Wave 17 lane 8 UX pass: monospace tabular money everywhere, past-due /
// due-soon signals off the real due date, quick "pay in full" affordance,
// staggered entrance.
"use client";

import { friendlyMutationText } from "../../../../lib/error-copy";
import { Button, Card, CardContent, CardHeader, Spinner, StatusTag } from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalInvoiceDetail,
} from "../../../../lib/portal-api";

function money(minor: number, currency: string): string {
  return new Intl.NumberFormat(undefined, { style: "currency", currency }).format(minor / 100);
}

function dateTime(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function dateOnly(iso: string): string {
  return new Date(`${iso}T00:00:00`).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function daysUntil(iso: string): number {
  const today = new Date().setHours(0, 0, 0, 0);
  const due = new Date(`${iso}T00:00:00`).setHours(0, 0, 0, 0);
  return Math.round((due - today) / 86_400_000);
}

export default function PortalInvoiceDetailPage() {
  useRequirePortalAuth();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [detail, setDetail] = useState<PortalInvoiceDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [amount, setAmount] = useState<string>("");
  const [notes, setNotes] = useState<string>("");
  const [paying, setPaying] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(() => {
    let cancelled = false;
    portalApi
      .getInvoice(params.id)
      .then((d) => {
        if (cancelled) return;
        setDetail(d);
        setAmount((d.invoice.outstanding_minor / 100).toFixed(2));
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load invoice"));
      });
    return () => {
      cancelled = true;
    };
  }, [params.id]);

  useEffect(() => load(), [load]);

  async function pay() {
    if (detail === null) return;
    const parsed = Number.parseFloat(amount);
    if (!Number.isFinite(parsed) || parsed <= 0) {
      setNotice("Enter an amount greater than zero.");
      return;
    }
    const minor = Math.round(parsed * 100);
    if (minor > detail.invoice.outstanding_minor) {
      setNotice("The amount exceeds what is still open on this invoice.");
      return;
    }
    setPaying(true);
    setNotice(null);
    try {
      const result = await portalApi.payInvoice(params.id, minor, notes || undefined);
      setDetail((prev) => (prev === null ? prev : { ...prev, invoice: result.invoice }));
      setNotice(
        result.invoice.status === "paid"
          ? "Invoice settled — the receipt has been recorded on your account."
          : "Payment recorded — the receipt has been added to your account.",
      );
      setNotes("");
    } catch (err: unknown) {
      setNotice(friendlyMutationText(err, "Recording the payment failed; try again."));
    } finally {
      setPaying(false);
    }
  }

  async function openDocument() {
    try {
      const url = await portalApi.invoiceDocumentUrl(params.id);
      window.open(url, "_blank", "noopener");
      window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
    } catch {
      setNotice("Opening the printable document failed.");
    }
  }

  if (error !== null) {
    return <p className="text-sm" style={{ color: "var(--m-status-danger)" }}>{error}</p>;
  }
  if (detail === null) {
    return (
      <div className="flex justify-center py-16">
        <Spinner aria-label="Loading invoice" />
      </div>
    );
  }

  const inv = detail.invoice;
  const outstanding = inv.outstanding_minor;
  const providerNone = detail.payments.some((p) => p.provider === "none");
  const overdue = inv.status === "issued" && daysUntil(inv.due_on) < 0;
  const soon = inv.status === "issued" && !overdue && daysUntil(inv.due_on) <= 7;

  return (
    <section className="m-rise space-y-4">
      <div>
        <button
          type="button"
          onClick={() => router.push("/portal/invoices")}
          className="text-xs transition-colors duration-100 hover:underline"
          style={{ color: "var(--m-text-muted)" }}
        >
          ← Invoices
        </button>
        <div className="mt-1 flex flex-wrap items-center justify-between gap-3">
          <h1 className="font-heading text-xl font-semibold tracking-tight tabular-nums">
            {inv.number}
          </h1>
          <div className="flex items-center gap-2">
            {overdue && <StatusTag tone="danger">Past due</StatusTag>}
            {soon && <StatusTag tone="warning">Due soon</StatusTag>}
            <StatusTag tone={inv.status === "paid" ? "success" : "neutral"}>
              {inv.status === "paid" ? "Paid" : "Open"}
            </StatusTag>
          </div>
        </div>
        <p className="mt-0.5 text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
          Issued {dateOnly(inv.issued_at)} · Due {dateOnly(inv.due_on)}
        </p>
      </div>

      <Card>
        <CardHeader>Summary</CardHeader>
        <CardContent>
          <dl className="grid grid-cols-3 gap-x-4 gap-y-1 text-sm">
            <dt style={{ color: "var(--m-text-muted)" }}>Total</dt>
            <dd className="col-span-2 text-right font-mono text-sm tabular-nums">
              {money(inv.total_minor, inv.currency)}
            </dd>
            <dt style={{ color: "var(--m-text-muted)" }}>Paid</dt>
            <dd className="col-span-2 text-right font-mono text-sm tabular-nums">
              {money(inv.total_paid_minor, inv.currency)}
            </dd>
            <dt style={{ color: "var(--m-text-muted)" }}>Outstanding</dt>
            <dd className="col-span-2 text-right font-semibold font-mono text-sm tabular-nums">
              {money(outstanding, inv.currency)}
            </dd>
          </dl>
          <Button variant="secondary" size="sm" className="mt-3" onClick={openDocument}>
            Printable document
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>Line items</CardHeader>
        <CardContent>
          <ul className="space-y-2 text-sm">
            {detail.lines.map((line, i) => (
              <li
                key={line.id}
                className="m-rise flex items-baseline justify-between gap-3"
                style={{ animationDelay: `${Math.min(i * 30, 180)}ms` }}
              >
                <span className="min-w-0">
                  {line.description}
                  {line.quantity_minutes > 0 && (
                    <span className="ml-2 font-mono text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                      {line.quantity_minutes} min
                    </span>
                  )}
                </span>
                <span className="shrink-0 font-mono text-sm tabular-nums">
                  {money(line.amount_minor, inv.currency)}
                </span>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>

      {detail.payments.length > 0 && (
        <Card>
          <CardHeader>Receipts</CardHeader>
          <CardContent>
            <ul className="space-y-2 text-sm">
              {detail.payments.map((p) => (
                <li key={p.id} className="flex items-baseline justify-between gap-3">
                  <span className="text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                    {dateTime(p.received_at)}
                  </span>
                  <span className="shrink-0 font-mono text-sm tabular-nums">
                    {money(p.amount_minor, p.currency)}
                    {p.provider === "none" && (
                      <span className="ml-2 font-sans text-xs" style={{ color: "var(--m-text-muted)" }}>
                        recorded on account
                      </span>
                    )}
                  </span>
                </li>
              ))}
            </ul>
            {providerNone && (
              <p className="mt-2 text-xs" style={{ color: "var(--m-text-muted)" }}>
                Your provider has no card processor connected. Payments here
                are ledger entries: they record what you settled and are
                reconciled by your provider.
              </p>
            )}
          </CardContent>
        </Card>
      )}

      {inv.status === "issued" && outstanding > 0 && (
        <Card>
          <CardHeader>Record a payment</CardHeader>
          <CardContent>
            <div className="flex flex-wrap items-end gap-2">
              <label className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                Amount ({inv.currency})
                <input
                  type="number"
                  min="0"
                  step="0.01"
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  className="mt-1 w-32 rounded-[0.75rem] border bg-transparent px-2 py-1.5 font-mono text-sm tabular-nums outline-none transition-colors duration-100 focus:border-[var(--m-accent)]"
                  style={{ borderColor: "var(--m-border)" }}
                />
              </label>
              <button
                type="button"
                onClick={() => setAmount((outstanding / 100).toFixed(2))}
                className="mb-0.5 rounded-[999px] border px-2.5 py-1 text-xs transition-colors duration-100"
                style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)", background: "var(--m-canvas)" }}
              >
                Full amount ({money(outstanding, inv.currency)})
              </button>
              <label className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                Reference (optional)
                <input
                  type="text"
                  value={notes}
                  onChange={(e) => setNotes(e.target.value)}
                  placeholder="Check number, transfer id…"
                  className="mt-1 w-56 rounded-[0.75rem] border bg-transparent px-2 py-1.5 text-sm outline-none transition-colors duration-100 focus:border-[var(--m-accent)]"
                  style={{ borderColor: "var(--m-border)" }}
                />
              </label>
              <Button size="sm" onClick={pay} disabled={paying}>
                {paying ? "Recording…" : "Record payment"}
              </Button>
            </div>
            {notice !== null && (
              <p className="mt-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                {notice}
              </p>
            )}
          </CardContent>
        </Card>
      )}
    </section>
  );
}
