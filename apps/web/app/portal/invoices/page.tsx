// /portal/invoices — what you owe, first.
//
// Purpose: the most important fact on the page is the most important size.
// The pack's state 05 puts "You owe $3,860.00" at 32px in a card with a BLUSH
// border, a blush "Overdue 12 days" pill and a sentence saying how to pay. The
// list of every invoice sits below it.
//
// OVERDUE IS BLUSH, WITH THE NUMBER OF DAYS. Never red. Red means a genuine
// breach (rule 2 of the DERIVATION-GUIDE contract), and an unpaid invoice is
// not hers to be frightened by — it is a fact with a date on it. The pill
// carries a clock and the number, so the state never depends on colour alone.
//
// Non-purpose: no staff actions on this list (remind, write off, apply
// credit), and no payment is taken here — the pinned API's pay operation is
// not something a client invoice page should reach for. How to pay is a
// sentence, and the money goes where the invoice email says.
"use client";

import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { friendlyMutationText } from "../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import { portalApi, useRequirePortalAuth, type PortalInvoice } from "../../../lib/portal-api";
import { longDate, money, overdueDays } from "../../../lib/portal-copy";
import {
  OverduePill,
  PaidPill,
  PortalCard,
  PortalIcon,
  PortalRow,
} from "../../../components/portal/portal-ui";

export default function PortalInvoicesPage() {
  useRequirePortalAuth();
  const router = useRouter();
  const [invoices, setInvoices] = useState<PortalInvoice[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .listInvoices()
      .then((list) => {
        if (!cancelled) setInvoices(list);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(friendlyMutationText(err, "We couldn't load your invoices. Try again in a moment."));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const owing = useMemo(() => {
    if (invoices === null) return null;
    return invoices
      .filter((i) => i.outstanding_minor > 0)
      .map((i) => ({ invoice: i, days: overdueDays(i.due_on) }))
      .sort((a, b) => (a.days ?? -1) - (b.days ?? -1));
  }, [invoices]);

  if (error !== null) {
    return (
      <PortalCard>
        <p className="portal-field__error" role="alert" style={{ margin: 0 }}>
          <PortalIcon name="alert" size={14} />
          <span>{error}</span>
        </p>
      </PortalCard>
    );
  }

  if (invoices === null || owing === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading your invoices" />
      </div>
    );
  }

  const totalOwed = owing.reduce((sum, row) => sum + row.invoice.outstanding_minor, 0);
  const currency = owing[0]?.invoice.currency ?? invoices[0]?.currency ?? "USD";
  // The reference is the LONGEST-OVERDUE invoice's number, because that is the
  // one a payment will actually be matched against.
  const oldest = owing.find((r) => r.days !== null) ?? owing[0];

  return (
    <>
      <div>
        <h1 className="portal-title">Invoices</h1>
        <p className="portal-lead">What you owe, and what you&apos;ve paid.</p>
      </div>

      {owing.length > 0 ? (
        <div
          className="portal-card"
          style={{ borderColor: "var(--m-blush)" }}
          data-testid="portal-owes"
        >
          <div className="portal-owes">
            <div style={{ flex: 1 }}>
              <div className="portal-hint">You owe</div>
              <div className="portal-owes__amount">{money(totalOwed, currency)}</div>
              <div style={{ marginTop: 6 }}>
                {owing.some((r) => r.days !== null) ? (
                  <OverduePill days={Math.max(...owing.map((r) => r.days ?? 0))} />
                ) : (
                  <span className="portal-pill portal-pill--paid">
                    <PortalIcon name="check" size={14} />
                    Not due yet
                  </span>
                )}
              </div>
            </div>
            <div style={{ flex: 1.2, fontSize: 14, lineHeight: 1.6 }}>
              {/* "How to pay." is BOLD and not underlined — the pack sets it as a
                  label, not a link (portal/sand/states/05-invoices.png). */}
              <b style={{ color: "var(--m-text)" }}>How to pay.</b> Pay by EFT and put{" "}
              <b style={{ color: "var(--m-text)" }}>{oldest?.invoice.number}</b> as the
              reference. Questions? Just reply to the invoice email, or use Contact us below.
            </div>
          </div>
        </div>
      ) : (
        <PortalCard testId="portal-owes-nothing">
          <div className="portal-empty" style={{ padding: "32px 24px" }}>
            <h2>Nothing to pay</h2>
            <p>Every invoice your service desk has sent is settled. New ones appear here.</p>
          </div>
        </PortalCard>
      )}

      <PortalCard title="All invoices" testId="portal-invoice-list">
        {invoices.length === 0 ? (
          <div className="portal-empty" style={{ padding: "32px 24px" }}>
            <p className="portal-hint">
              No invoices yet. They appear here as soon as your service desk sends one.
            </p>
          </div>
        ) : (
          invoices.map((inv) => {
            const days = overdueDays(inv.due_on);
            const outstanding = inv.outstanding_minor > 0;
            return (
              <PortalRow
                key={inv.id}
                title={inv.number}
                meta={
                  outstanding
                    ? `Due ${longDate(inv.due_on)}`
                    : `Issued ${longDate(inv.issued_at)}`
                }
                onClick={() => router.push(`/portal/invoices/${inv.id}`)}
                columns="86px minmax(0, 1fr) 110px 130px 36px"
                trailing={
                  <>
                    <b className="portal-row__amount">{money(inv.total_minor, inv.currency)}</b>
                    {outstanding ? (
                      days === null ? (
                        <span className="portal-pill portal-pill--received">Due {longDate(inv.due_on)}</span>
                      ) : (
                        /* The row pill says "Overdue"; the day count lives ONCE in
                           the hero above, so it is not repeated down the list
                           (portal/sand/states/05-invoices.png). */
                        <span className="portal-pill portal-pill--overdue">
                          <PortalIcon name="clock" size={14} />
                          Overdue
                        </span>
                      )
                    ) : (
                      <PaidPill />
                    )}
                    <span className="portal-row__download">
                      <PortalIcon name="download" size={18} />
                    </span>
                  </>
                }
                testId={`portal-invoice-${inv.number}`}
              />
            );
          })
        )}
      </PortalCard>
    </>
  );
}
