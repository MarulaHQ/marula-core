// /portal/quotes/[id] — the quote, and the one decision the client makes here.
//
// Purpose: the pack's state 06. The quote's title, its number, the date it is
// valid until, and a GOLD "Waiting for your answer" pill — because this is the
// screen where she is the one who has to act. The lines, the total, and a
// sentence that says how the money works in words: half on acceptance, half
// when the work is finished. Then the two ways forward: accept, or decline or
// ask for a change.
//
// ACCEPTING TAKES TWO THINGS, AND THE DIALOG SAYS SO. The pack's state 07: a
// typed full name (recorded with the date, so the service desk knows who
// agreed) and a ticked box (she has read it, and she knows the payment split).
// The footer says "Nothing is charged today" — the most useful sentence on the
// screen, because her real fear is that saying yes moves money. The primary
// stays disabled until both are present, and the error is BLUSH with words.
//
// THE HUMAN BOUNDARY. Accepting is a decision the client records, and the
// scheduled work that follows is booked by a person at the service desk. No
// client-visible commitment is ever made by this screen alone.
"use client";

import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { friendlyMutationText } from "../../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalQuoteDetail,
} from "../../../../lib/portal-api";
import { useTimeFormat } from "../../../../lib/use-time-format";
import { longDate, money } from "../../../../lib/portal-copy";
import {
  Field,
  Note,
  PortalCard,
  PortalIcon,
  StatusPill,
} from "../../../../components/portal/portal-ui";

function quantity(qtyMilli: number): string {
  const value = qtyMilli / 1000;
  return value % 1 === 0 ? String(value) : value.toFixed(2);
}

export default function PortalQuotePage() {
  useRequirePortalAuth();
  const { timeZone } = useTimeFormat();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [quote, setQuote] = useState<PortalQuoteDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The accept dialog's own state. It is a decision, so it is asked once and
  // the dialog says what accepting and declining each mean.
  const [accepting, setAccepting] = useState(false);
  const [name, setName] = useState("");
  const [agreed, setAgreed] = useState(false);
  const [nameError, setNameError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [declineReason, setDeclineReason] = useState("");
  const [declining, setDeclining] = useState(false);

  const load = useCallback(async () => {
    try {
      setQuote(await portalApi.getQuote(params.id));
    } catch (err) {
      setError(friendlyMutationText(err, "We couldn't open this quote. Try again in a moment."));
    }
  }, [params.id]);

  useEffect(() => {
    void load();
  }, [load]);

  async function accept(e: React.FormEvent) {
    e.preventDefault();
    if (quote === null) return;
    if (name.trim().length === 0) {
      // Blush, with the reason and the fix — never red, never a bare "invalid".
      setNameError("Type your full name so we know who agreed.");
      return;
    }
    if (!agreed) {
      setNameError(null);
      return;
    }
    setBusy(true);
    setNameError(null);
    try {
      await portalApi.approveQuote(quote.id);
      setAccepting(false);
      setName("");
      setAgreed(false);
      await load();
    } catch (err) {
      setNameError(
        friendlyMutationText(
          err,
          "We couldn't record that yet. Nothing was accepted — try again in a moment.",
        ),
      );
    } finally {
      setBusy(false);
    }
  }

  async function decline() {
    if (quote === null) return;
    setBusy(true);
    try {
      await portalApi.declineQuote(quote.id, declineReason.trim() === "" ? undefined : declineReason.trim());
      setDeclining(false);
      setDeclineReason("");
      await load();
    } catch (err) {
      setError(friendlyMutationText(err, "We couldn't record that. Nothing was sent."));
    } finally {
      setBusy(false);
    }
  }

  if (error !== null && quote === null) {
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
              onClick={() => router.push("/portal/quotes")}
            >
              Back to your quotes
            </button>
          </p>
        </div>
      </PortalCard>
    );
  }

  if (quote === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading this quote" />
      </div>
    );
  }

  const title = quote.notes ?? `Quote ${quote.number}`;
  const waiting = quote.status === "sent";

  return (
    <>
      <div>
        <button
          type="button"
          className="portal-back"
          onClick={() => router.push("/portal/quotes")}
        >
          <PortalIcon name="back" size={14} />
          Your quotes
        </button>
      </div>

      <div className="portal-heading-row">
        <div>
          <h1 className="portal-title">{title}</h1>
          <p className="portal-lead">
            {quote.number} · Valid until {longDate(quote.valid_until, timeZone)}
          </p>
        </div>
        {quote.status === "sent" ? (
          <StatusPill status="waiting" note="Waiting for your answer" />
        ) : quote.status === "accepted" ? (
          <StatusPill status="resolved" note="Accepted" />
        ) : (
          <span className="portal-pill portal-pill--received">
            {quote.status === "declined" ? "Declined" : "Expired"}
          </span>
        )}
      </div>

      <PortalCard testId="portal-quote-lines">
        <div className="portal-line-head">
          <span>What you get</span>
          <span>Qty</span>
          <span>Price</span>
        </div>
        {quote.lines.map((l, i) => (
          <div className="portal-line" key={`${l.description}-${i}`}>
            <span>{l.description}</span>
            <span>{quantity(l.quantity_milli)}</span>
            <span>{money(l.amount_minor, quote.currency)}</span>
          </div>
        ))}
        {quote.tax_minor > 0 && (
          <div className="portal-line">
            <span>Tax</span>
            <span>&mdash;</span>
            <span>{money(quote.tax_minor, quote.currency)}</span>
          </div>
        )}
        <div className="portal-total">
          <span>Total</span>
          <span>{money(quote.total_minor, quote.currency)}</span>
        </div>
      </PortalCard>

      <p className="portal-hint" style={{ fontSize: 14, margin: 0 }}>
        {quote.terms !== null && quote.terms !== "" ? `${quote.terms} ` : ""}
        Payment: half on acceptance, half when the work is finished. Work starts once you accept,
        and we&apos;ll book a date with you.
      </p>

      {quote.status === "accepted" && (
        <Note>
          Accepted{quote.decided_at !== null ? ` on ${longDate(quote.decided_at, timeZone)}` : ""}. Your
          service desk will book a date with you, and it will appear under Schedule.
        </Note>
      )}

      {waiting && (
        <div style={{ display: "flex", flexWrap: "wrap", gap: 10 }}>
<button
              type="button"
              className="portal-btn"
              onClick={() => setAccepting(true)}
              data-testid="portal-quote-accept"
            >
              {/* The decision carries a check, so the primary says what it does
                 (portal/sand/states/07-accept-quote.png). */}
              <PortalIcon name="check" size={16} />
              Accept this quote
            </button>
          <button
            type="button"
            className="portal-btn portal-btn--secondary"
            onClick={() => setDeclining((v) => !v)}
            data-testid="portal-quote-decline"
          >
            Decline or ask for a change
          </button>
        </div>
      )}

      {declining && (
        <PortalCard title="Tell your service desk why" testId="portal-quote-decline-form">
          <div className="portal-card__body" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <Field
              label="What would need to change?"
              htmlFor="portal-decline-reason"
              hint="A sentence is plenty. A person reads this and comes back to you."
            >
              <textarea
                id="portal-decline-reason"
                className="portal-composer"
                style={{ height: 90, padding: 12 }}
                value={declineReason}
                onChange={(e) => setDeclineReason(e.target.value)}
              />
            </Field>
            <div style={{ display: "flex", gap: 10 }}>
              <button
                type="button"
                className="portal-btn"
                onClick={() => void decline()}
                disabled={busy}
              >
                {busy ? "Sending…" : "Send this back"}
              </button>
              <button
                type="button"
                className="portal-btn portal-btn--secondary"
                onClick={() => setDeclining(false)}
              >
                Keep thinking
              </button>
            </div>
            <p className="portal-hint" style={{ margin: 0 }}>
              Declining costs nothing and sends no money. If you only want the price changed, say
              so here and they&apos;ll re-quote.
            </p>
          </div>
        </PortalCard>
      )}

      {accepting && (
        <div className="portal-scrim" role="presentation" onClick={() => setAccepting(false)}>
          <form
            className="portal-dialog"
            role="dialog"
            aria-modal="true"
            aria-label={`Accept quote ${quote.number}`}
            onClick={(e) => e.stopPropagation()}
            onSubmit={accept}
          >
            <div className="portal-dialog__hd">
              <div>
                <h2>Accept quote {quote.number}</h2>
                <p>
                  {money(quote.total_minor, quote.currency)} · {title}
                </p>
              </div>
              <span style={{ flex: 1 }} />
              {/* A small bordered dismiss button, not a text "Close"
                 (portal/sand/states/07-accept-quote.png). */}
              <button
                type="button"
                className="portal-dialog__close"
                onClick={() => setAccepting(false)}
                aria-label="Close"
              >
                <PortalIcon name="x" size={16} />
              </button>
            </div>
            <div className="portal-dialog__mb">
              <Field
                label="Type your full name to accept"
                htmlFor="portal-accept-name"
                error={nameError}
                hint="It is recorded with today's date, so we know who agreed."
              >
                <input
                  id="portal-accept-name"
                  className="portal-input"
                  value={name}
                  onChange={(e) => {
                    setName(e.target.value);
                    if (nameError !== null) setNameError(null);
                  }}
                  placeholder="Your full name"
                  autoComplete="name"
                  aria-invalid={nameError !== null}
                  aria-describedby={nameError ? "portal-accept-name-error" : "portal-accept-name-hint"}
                />
              </Field>
              <label className="portal-check">
                <input
                  type="checkbox"
                  checked={agreed}
                  onChange={(e) => setAgreed(e.target.checked)}
                />
                <span>
                  I have read the quote and I&apos;m happy to go ahead. Payment is half now, half
                  on completion.
                </span>
              </label>
            </div>
            <div className="portal-dialog__mf">
              <span className="portal-hint">Nothing is charged today.</span>
              <button
                type="button"
                className="portal-btn portal-btn--secondary portal-btn--small"
                onClick={() => setAccepting(false)}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="portal-btn portal-btn--small"
                disabled={busy || name.trim().length === 0 || !agreed}
                data-testid="portal-quote-accept-confirm"
              >
                {busy ? "Recording…" : "Accept the quote"}
              </button>
            </div>
          </form>
        </div>
      )}
    </>
  );
}
