// /portal/quotes/[id] — one quote: the document, its lines, and the
// decision (Portal v2, Wave 34). Approve binds the SAME lifecycle leg
// the staff flow runs (sent → accepted); the provider then turns the
// accepted quote into the contract and invoice on their side. Decline
// takes an optional reason, recorded for the provider — it is never
// shown back as an error. Expired and validity-lapsed quotes say so
// honestly instead of hiding the decision controls behind an error.
"use client";

import { friendlyMutationText } from "../../../../lib/error-copy";
import { Button, Card, CardContent, CardHeader, Spinner, StatusTag } from "@marula-core/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalQuoteDetail,
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

function qty(quantityMilli: number, unit: string): string {
  return `${quantityMilli / 1000} ${unit}`;
}

export default function PortalQuoteDetailPage() {
  useRequirePortalAuth();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [detail, setDetail] = useState<PortalQuoteDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [declineOpen, setDeclineOpen] = useState(false);
  const [reason, setReason] = useState("");

  const load = useCallback(() => {
    let cancelled = false;
    portalApi
      .getQuote(params.id)
      .then((d) => {
        if (!cancelled) setDetail(d);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load quote"));
      });
    return () => {
      cancelled = true;
    };
  }, [params.id]);

  useEffect(() => load(), [load]);

  async function decide(kind: "approve" | "decline") {
    if (detail === null) return;
    setBusy(true);
    setNotice(null);
    try {
      const next =
        kind === "approve"
          ? await portalApi.approveQuote(params.id, undefined)
          : await portalApi.declineQuote(params.id, reason.trim() || undefined);
      setDetail({ ...next, lines: detail.lines });
      setDeclineOpen(false);
      setNotice(
        kind === "approve"
          ? "Quote approved — your provider has been notified and will set up the contract and invoice on their side."
          : "Quote declined — your provider has been notified" +
              (reason.trim() ? " with your note." : "."),
      );
      setReason("");
    } catch (err: unknown) {
      setNotice(friendlyMutationText(err, "Recording your decision failed; try again."));
    } finally {
      setBusy(false);
    }
  }

  if (error !== null) {
    return <p className="text-sm" style={{ color: "var(--m-status-danger)" }}>{error}</p>;
  }
  if (detail === null) {
    return (
      <div className="flex justify-center py-16">
        <Spinner aria-label="Loading quote" />
      </div>
    );
  }

  const quote = detail;
  const lapsed = quote.status === "sent" && daysUntil(quote.valid_until) < 0;
  const decidable = quote.status === "sent" && !lapsed;
  const dec =
    quote.status === "accepted"
      ? "success"
      : quote.status === "declined"
        ? "danger"
        : quote.status === "sent"
          ? "warning"
          : "neutral";
  const decLabel =
    quote.status === "sent"
      ? lapsed
        ? "Valid until passed"
        : "Awaiting your decision"
      : quote.status === "accepted"
        ? "Approved"
        : quote.status === "declined"
          ? "Declined"
          : "Expired";

  return (
    <section className="m-rise space-y-4">
      <div>
        <button
          type="button"
          onClick={() => router.push("/portal/quotes")}
          className="text-xs transition-colors duration-100 hover:underline"
          style={{ color: "var(--m-text-muted)" }}
        >
          ← Quotes
        </button>
        <div className="mt-1 flex flex-wrap items-center justify-between gap-3">
          <h1 className="text-xl font-semibold tracking-tight tabular-nums">{quote.number}</h1>
          <StatusTag tone={dec}>{decLabel}</StatusTag>
        </div>
        <p className="mt-0.5 text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
          Issued {dateOnly(quote.issued_on)} · Valid until {dateOnly(quote.valid_until)}
          {quote.sent_at !== null && <> · Sent {dateTime(quote.sent_at)}</>}
          {quote.decided_at !== null && <> · Decided {dateTime(quote.decided_at)}</>}
        </p>
      </div>

      {quote.status === "accepted" && (
        <div
          className="rounded-[0.5rem] px-3 py-2 text-sm"
          style={{ background: "var(--m-status-success-bg)", color: "var(--m-status-success-fg)" }}
          data-testid="quote-accepted-note"
        >
          You approved this quote. Your provider will create the contract and
          invoice from it on their side — nothing is billed through this
          portal.
        </div>
      )}
      {quote.status === "declined" && (
        <div
          className="rounded-[0.5rem] px-3 py-2 text-sm"
          style={{ background: "var(--m-surface)", border: "1px solid var(--m-border)" }}
        >
          You declined this quote. If your provider revises it, a new quote
          will appear in your list.
        </div>
      )}
      {quote.status === "expired" && (
        <div
          className="rounded-[0.5rem] px-3 py-2 text-sm"
          style={{ background: "var(--m-surface)", border: "1px solid var(--m-border)" }}
        >
          This quote expired before a decision was made. Ask your provider
          for a fresh quote.
        </div>
      )}
      {lapsed && (
        <div
          className="rounded-[0.5rem] px-3 py-2 text-sm"
          style={{ background: "var(--m-status-warning-bg)", color: "var(--m-status-warning-fg)" }}
        >
          This quote's validity window has passed. Your provider can still
          confirm it or send a fresh quote.
        </div>
      )}

      <Card>
        <CardHeader>Line items</CardHeader>
        <CardContent>
          <table className="w-full text-sm" data-testid="quote-lines">
            <thead>
              <tr className="text-left text-xs" style={{ color: "var(--m-text-muted)" }}>
                <th className="pb-1 font-medium">Description</th>
                <th className="pb-1 text-right font-medium">Qty</th>
                <th className="pb-1 text-right font-medium">Unit price</th>
                <th className="pb-1 text-right font-medium">Amount</th>
              </tr>
            </thead>
            <tbody>
              {detail.lines.map((line) => (
                <tr key={line.position} className="border-t" style={{ borderColor: "var(--m-border)" }}>
                  <td className="py-1.5 pr-3">
                    {line.description}
                    {line.taxable && (
                      <span className="ml-2 text-xs" style={{ color: "var(--m-text-muted)" }}>
                        taxable
                      </span>
                    )}
                  </td>
                  <td className="py-1.5 text-right font-mono text-xs tabular-nums">
                    {qty(line.quantity_milli, line.unit)}
                  </td>
                  <td className="py-1.5 text-right font-mono text-xs tabular-nums">
                    {money(line.unit_amount_minor, quote.currency)}
                  </td>
                  <td className="py-1.5 text-right font-mono text-xs tabular-nums">
                    {money(line.amount_minor, quote.currency)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <dl className="mt-3 ml-auto max-w-60 space-y-1 text-sm">
            <div className="flex justify-between">
              <dt style={{ color: "var(--m-text-muted)" }}>Subtotal</dt>
              <dd className="font-mono tabular-nums">{money(quote.subtotal_minor, quote.currency)}</dd>
            </div>
            <div className="flex justify-between">
              <dt style={{ color: "var(--m-text-muted)" }}>Tax ({quote.tax_rate_bps / 100}%)</dt>
              <dd className="font-mono tabular-nums">{money(quote.tax_minor, quote.currency)}</dd>
            </div>
            <div className="flex justify-between border-t pt-1 font-semibold" style={{ borderColor: "var(--m-border)" }}>
              <dt>Total</dt>
              <dd className="font-mono tabular-nums" data-testid="quote-total">
                {money(quote.total_minor, quote.currency)}
              </dd>
            </div>
          </dl>
        </CardContent>
      </Card>

      {(quote.notes !== null || quote.terms !== null) && (
        <Card>
          <CardHeader>Notes &amp; terms</CardHeader>
          <CardContent>
            <div className="space-y-2 text-sm">
              {quote.notes !== null && <p style={{ color: "var(--m-text-secondary)" }}>{quote.notes}</p>}
              {quote.terms !== null && <p style={{ color: "var(--m-text-muted)" }}>{quote.terms}</p>}
            </div>
          </CardContent>
        </Card>
      )}

      {decidable && (
        <Card data-testid="quote-decision">
          <CardHeader>Your decision</CardHeader>
          <CardContent>
            <p className="text-sm" style={{ color: "var(--m-text-secondary)" }}>
              Approving accepts this quote as scoped and priced. Your
              provider then creates the contract and invoice from it —
              billing happens on their side, not here. Declining is final
              for this quote; you can always ask for a revised one.
            </p>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <Button
                size="sm"
                disabled={busy}
                onClick={() => decide("approve")}
                data-testid="quote-approve"
              >
                Approve quote
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={busy}
                onClick={() => setDeclineOpen((v) => !v)}
                data-testid="quote-decline-toggle"
              >
                Decline…
              </Button>
            </div>
            {declineOpen && (
              <div className="m-rise mt-3">
                <label htmlFor="quote-decline-reason" className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                  Reason (optional, shared with your provider)
                </label>
                <textarea
                  id="quote-decline-reason"
                  value={reason}
                  onChange={(e) => setReason(e.target.value)}
                  rows={3}
                  maxLength={1000}
                  className="mt-1 w-full rounded-[0.375rem] px-2 py-1.5 text-sm"
                  style={{ background: "var(--m-bg)", border: "1px solid var(--m-border)", color: "var(--m-text)" }}
                  data-testid="quote-decline-reason"
                />
                <div className="mt-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={busy}
                    onClick={() => decide("decline")}
                    data-testid="quote-decline-confirm"
                  >
                    Decline quote
                  </Button>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {notice !== null && (
        <p className="text-sm" style={{ color: "var(--m-text-secondary)" }} data-testid="quote-notice" role="status">
          {notice}
        </p>
      )}
    </section>
  );
}
