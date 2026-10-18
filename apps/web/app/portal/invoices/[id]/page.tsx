// /portal/invoices/[id] — one invoice, in the client's own words.
//
// Purpose: derived from the invoices list plus the staff invoice detail
// (DERIVATION-GUIDE.md, "Invoice detail"). The lines and the total, the way to
// pay, and a download of the document. What it drops from the staff version is
// everything internal: no staff actions, no payment recording, no credit
// notes, no ageing buckets — a client sees what she owes and what to do about
// it.
//
// OVERDUE STAYS BLUSH, WITH THE DAYS. The rule is the same on the list and on
// the detail: an unpaid invoice is a fact with a date, not a breach, and the
// number of days is the whole message.
"use client";

import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { friendlyMutationText } from "../../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import { portalApi, useRequirePortalAuth, type PortalInvoiceDetail } from "../../../../lib/portal-api";
import { useTimeFormat } from "../../../../lib/use-time-format";
import { longDate, money, overdueDays } from "../../../../lib/portal-copy";
import {
  Note,
  OverduePill,
  PaidPill,
  PortalCard,
  PortalIcon,
} from "../../../../components/portal/portal-ui";

function lineLabel(line: { kind: string; description: string; quantity_minutes: number }): string {
  // Time is billed by the quarter hour; anything else reads as a count.
  if (line.quantity_minutes > 0) {
    const hours = line.quantity_minutes / 60;
    return `${hours % 1 === 0 ? hours : hours.toFixed(2)} h`;
  }
  return line.kind === "line" ? "1" : "—";
}

export default function PortalInvoicePage() {
  useRequirePortalAuth();
  const { timeZone } = useTimeFormat();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [detail, setDetail] = useState<PortalInvoiceDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setDetail(await portalApi.getInvoice(params.id));
    } catch (err) {
      setError(friendlyMutationText(err, "We couldn't open this invoice. Try again in a moment."));
    }
  }, [params.id]);

  useEffect(() => {
    void load();
  }, [load]);

  if (error !== null) {
    return (
      <PortalCard>
        <div className="portal-card__body">
          <p className="portal-field__error" role="alert">
            <PortalIcon name="alert" size={14} />
            <span>{error}</span>
          </p>
          <p>
            <button
              type="button"
              className="portal-link"
              onClick={() => router.push("/portal/invoices")}
            >
              Back to your invoices
            </button>
          </p>
        </div>
      </PortalCard>
    );
  }

  if (detail === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading this invoice" />
      </div>
    );
  }

  const inv = detail.invoice;
  const days = overdueDays(inv.due_on);
  const outstanding = inv.outstanding_minor > 0;
  const tax = detail.lines
    .filter((l) => l.kind === "tax")
    .reduce((sum, l) => sum + l.amount_minor, 0);
  const net = inv.total_minor - tax;

  return (
    <>
      <div>
        <button
          type="button"
          className="portal-back"
          onClick={() => router.push("/portal/invoices")}
        >
          <PortalIcon name="back" size={14} />
          Your invoices
        </button>
      </div>

      <div className="portal-heading-row">
        <div>
          <h1 className="portal-title">{inv.number}</h1>
          <p className="portal-lead">
            Issued {longDate(inv.issued_at, timeZone)} · Due {longDate(inv.due_on, timeZone)}
          </p>
        </div>
        {outstanding ? (
          days === null ? (
            <span className="portal-pill portal-pill--received">Due {longDate(inv.due_on, timeZone)}</span>
          ) : (
            <OverduePill days={days} />
          )
        ) : (
          <PaidPill />
        )}
      </div>

      <PortalCard>
        <div className="portal-line-head">
          <span>What this is</span>
          <span>Time or qty</span>
          <span>Amount</span>
        </div>
        {detail.lines
          .filter((l) => l.kind !== "tax")
          .map((l) => (
            <div className="portal-line" key={l.id}>
              <span>{l.description}</span>
              <span>{lineLabel(l)}</span>
              <span>{money(l.amount_minor, inv.currency)}</span>
            </div>
          ))}
        {tax > 0 && (
          <div className="portal-line">
            <span>Tax</span>
            <span>—</span>
            <span>{money(tax, inv.currency)}</span>
          </div>
        )}
        <div className="portal-total">
          <span>Total</span>
          <span>{money(inv.total_minor, inv.currency)}</span>
        </div>
        {inv.total_paid_minor > 0 && (
          <div className="portal-total" style={{ fontSize: 14, fontWeight: 500 }}>
            <span style={{ color: "var(--m-text-secondary)" }}>Already paid</span>
            <span>{money(inv.total_paid_minor, inv.currency)}</span>
          </div>
        )}
        {outstanding && (
          <div
            className="portal-total"
            style={{ fontSize: 14, fontWeight: 500, borderTop: "none" }}
          >
            <span style={{ color: "var(--m-text-secondary)" }}>Still to pay</span>
            <span>{money(inv.outstanding_minor, inv.currency)}</span>
          </div>
        )}
      </PortalCard>

      {outstanding && (
        <PortalCard title="How to pay" testId="portal-invoice-pay">
          <div className="portal-card__body" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <p style={{ margin: 0, fontSize: 14, lineHeight: 1.6 }}>
              Pay by EFT and put <b>{inv.number}</b> as the reference. We match the reference to
              this invoice, so nothing else is needed.
            </p>
            <Note>
              Questions about a charge? Reply to the invoice email and a person will pick it up
              &mdash; billing questions reach the same service desk as everything else.
            </Note>
          </div>
        </PortalCard>
      )}

      {detail.payments.length > 0 && (
        <PortalCard title="Payments received" testId="portal-invoice-payments">
          {detail.payments.map((p) => (
            <div className="portal-row" key={p.id}>
              <span className="portal-row__title">
                <b>{longDate(p.received_at, timeZone)}</b>
                <span>{p.provider.replace(/_/g, " ")}</span>
              </span>
              <b className="portal-row__amount">{money(p.amount_minor, p.currency)}</b>
            </div>
          ))}
        </PortalCard>
      )}

      <button
        type="button"
        className="portal-btn portal-btn--secondary"
        onClick={() => {
          void portalApi.invoiceDocumentUrl(inv.id).then((url) => {
            const a = document.createElement("a");
            a.href = url;
            a.download = `${inv.number}.pdf`;
            document.body.appendChild(a);
            a.click();
            a.remove();
            window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
          });
        }}
        data-testid="portal-invoice-download"
      >
        <PortalIcon name="download" size={16} />
        Download this invoice
      </button>
    </>
  );
}
