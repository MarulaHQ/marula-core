// /portal/quotes — the quote list.
//
// Purpose: derived from the tickets list (DERIVATION-GUIDE.md: "Quote list |
// Tickets | The same card list, with the quote pills"). Same card list, same
// row, different pills: a quote waiting for her answer wears the gold tint,
// because it is the one thing on this screen that needs her to decide.
//
// Non-purpose: no staff lifecycle, no send, no revise, no draft quotes. A
// draft never leaves the staff surface, so it is not here.
"use client";

import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { friendlyMutationText } from "../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import { portalApi, useRequirePortalAuth, type PortalQuote } from "../../../lib/portal-api";
import { longDate, money } from "../../../lib/portal-copy";
import {
  PortalCard,
  PortalEmpty,
  PortalIcon,
  PortalRow,
  StatusPill,
} from "../../../components/portal/portal-ui";

/** The quote's own five states, in the client's words. */
function quotePill(status: PortalQuote["status"]) {
  switch (status) {
    case "sent":
      // Gold: it is waiting on her, and the only such state in the portal.
      return <StatusPill status="waiting" note="Waiting for your answer" />;
    case "accepted":
      return <StatusPill status="resolved" note="Accepted" />;
    case "declined":
      return <span className="portal-pill portal-pill--received">Declined</span>;
    case "expired":
      return <span className="portal-pill portal-pill--received">Expired</span>;
  }
}

export default function PortalQuotesPage() {
  useRequirePortalAuth();
  const router = useRouter();
  const [quotes, setQuotes] = useState<PortalQuote[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .listQuotes()
      .then((list) => {
        if (!cancelled) setQuotes(list);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(friendlyMutationText(err, "We couldn't load your quotes. Try again in a moment."));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const waiting = useMemo(
    () => (quotes === null ? [] : quotes.filter((q) => q.status === "sent")),
    [quotes],
  );
  const decided = useMemo(
    () => (quotes === null ? [] : quotes.filter((q) => q.status !== "sent")),
    [quotes],
  );

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

  if (quotes === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading your quotes" />
      </div>
    );
  }

  if (quotes.length === 0) {
    return (
      <>
        <div>
          <h1 className="portal-title">Quotes</h1>
          <p className="portal-lead">Prices for work your service desk has quoted.</p>
        </div>
        <PortalCard>
          <PortalEmpty title="Nothing to decide right now">
            When your service desk sends you a price for work, it appears here with everything you
            need to say yes, no, or ask for a change.
          </PortalEmpty>
        </PortalCard>
      </>
    );
  }

  const row = (q: PortalQuote) => (
    <PortalRow
      key={q.id}
      title={q.notes ?? `Quote ${q.number}`}
      meta={`${q.number} · Valid until ${longDate(q.valid_until)}`}
      columns="minmax(0, 1fr) 130px auto"
      highlight={q.status === "sent"}
      onClick={() => router.push(`/portal/quotes/${q.id}`)}
      trailing={
        <>
          <b className="portal-row__amount">{money(q.total_minor, q.currency)}</b>
          {quotePill(q.status)}
        </>
      }
      testId={`portal-quote-${q.number}`}
    />
  );

  return (
    <>
      <div>
        <h1 className="portal-title">Quotes</h1>
        <p className="portal-lead">Prices for work your service desk has quoted.</p>
      </div>

      {waiting.length > 0 && (
        <PortalCard
          title="Waiting for you"
          aside={<span className="portal-hint">{waiting.length} quotes</span>}
          testId="portal-quotes-waiting"
        >
          {waiting.map(row)}
        </PortalCard>
      )}

      {decided.length > 0 && (
        <PortalCard title="Already decided" testId="portal-quotes-decided">
          {decided.map(row)}
        </PortalCard>
      )}
    </>
  );
}
