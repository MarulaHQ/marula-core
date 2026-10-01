// /portal/csat — How did we do?
//
// Purpose: the pack's state 08. Five choices in WORDS, not a star rating and
// not a number alone: 5 Great, 4 Good, 3 Okay, 2 Not good, 1 Poor. The number
// is there because the service desk sorts by it; the word is there because
// she is reading it. The chosen one gets a green border and tint, so the state
// is never colour alone. An optional comment sits under it, and Send is
// disabled until she has chosen.
//
// The thank-you state thanks her and says what happens next, which is the
// pack's own line: every one is read, and if something was not right, a person
// is in touch today.
//
// Non-purpose: no re-prompting, no nudges, no "rate your service desk again".
// One answer, once.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState } from "react";
import { friendlyMutationText } from "../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import { portalApi, useRequirePortalAuth, type PortalCsatView } from "../../../lib/portal-api";
import {
  PortalCard,
  PortalEmpty,
  PortalIcon,
} from "../../../components/portal/portal-ui";

/** The five choices, in words. The number is for the desk; the word is for her. */
const CHOICES: readonly { value: number; word: string }[] = [
  { value: 5, word: "Great" },
  { value: 4, word: "Good" },
  { value: 3, word: "Okay" },
  { value: 2, word: "Not good" },
  { value: 1, word: "Poor" },
];

function CsatInner() {
  const router = useRouter();
  const params = useSearchParams();
  const ticket = params.get("ticket") ?? "";
  const [view, setView] = useState<PortalCsatView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [score, setScore] = useState<number | null>(null);
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    if (ticket === "") {
      setError("This link is missing its ticket. Open it from the ticket you want to rate.");
      return;
    }
    try {
      setView(await portalApi.csatView(ticket));
    } catch (err) {
      setError(friendlyMutationText(err, "We couldn't open this. It may have expired."));
    }
  }, [ticket]);

  useEffect(() => {
    void load();
  }, [load]);

  async function submit() {
    if (view === null || score === null || busy) return;
    setBusy(true);
    setError(null);
    try {
      setView(
        await portalApi.csatSubmit(view.ticket_id, score, comment.trim() === "" ? undefined : comment.trim()),
      );
    } catch (err) {
      setError(
        friendlyMutationText(err, "We couldn't send that. Your answer is kept below, try again."),
      );
    } finally {
      setBusy(false);
    }
  }

  if (error !== null && view === null) {
    return (
      <PortalCard>
        <div className="portal-card__body">
          <p className="portal-field__error" role="alert">
            <PortalIcon name="alert" size={14} />
            <span>{error}</span>
          </p>
          <p>
            <button type="button" className="portal-link" onClick={() => router.push("/portal/tickets")}>
              Back to your tickets
            </button>
          </p>
        </div>
      </PortalCard>
    );
  }

  if (view === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading the question" />
      </div>
    );
  }

  if (view.status === "responded") {
    return (
      <>
        <div>
          <h1 className="portal-title">How did we do?</h1>
          <p className="portal-lead">{view.ticket_subject}</p>
        </div>
        <PortalCard testId="portal-csat-thanks">
          <PortalEmpty
            title="Thank you"
            action={
              <button
                type="button"
                className="portal-btn"
                onClick={() => router.push("/portal/tickets")}
                style={{ marginTop: 8 }}
              >
                Back to your tickets
              </button>
            }
          >
            We read every one. If something wasn&apos;t right, someone from your service desk will
            be in touch today.
          </PortalEmpty>
        </PortalCard>
      </>
    );
  }

  return (
    <>
      <div>
        <button
          type="button"
          className="portal-back"
          onClick={() => router.push("/portal/tickets")}
        >
          <PortalIcon name="back" size={14} />
          Your tickets
        </button>
      </div>
      <div>
        <h1 className="portal-title">How did we do?</h1>
        <p className="portal-lead">
          {view.ticket_subject} · #{view.ticket_number}
        </p>
      </div>

      <div className="portal-scale" role="radiogroup" aria-label="How did we do?">
        {CHOICES.map((c) => (
          <button
            key={c.value}
            type="button"
            role="radio"
            aria-checked={score === c.value}
            onClick={() => setScore(c.value)}
            data-testid={`portal-csat-${c.value}`}
          >
            <b>{c.value}</b>
            <span>{c.word}</span>
          </button>
        ))}
      </div>

      {/* "Optional" rides the label line in the pack rather than sitting in the
          hint underneath, so the field reads as asked-for-but-not-required
          (portal/sand/states/08-feedback.png). */}
      <div className="portal-field">
        <div className="entry-label-like">
          <label className="portal-field__label" htmlFor="portal-csat-comment">
            Anything you&apos;d like to tell us?
          </label>
          <span className="portal-hint">Optional</span>
        </div>
        <input
          id="portal-csat-comment"
          className="portal-input"
          value={comment}
          onChange={(e) => setComment(e.target.value)}
          placeholder="What went well, or what didn't"
        />
      </div>

      {error !== null && (
        <p className="portal-field__error" role="alert">
          <PortalIcon name="alert" size={14} />
          <span>{error}</span>
        </p>
      )}

      <div style={{ display: "flex", flexWrap: "wrap", gap: 10 }}>
        <button
          type="button"
          className="portal-btn"
          disabled={busy || score === null}
          onClick={() => void submit()}
          data-testid="portal-csat-send"
        >
          {busy ? "Sending…" : "Send feedback"}
        </button>
        <button
          type="button"
          className="portal-btn portal-btn--secondary"
          onClick={() => router.push("/portal/tickets")}
        >
          Not now
        </button>
      </div>
    </>
  );
}

export default function PortalCsatPage() {
  useRequirePortalAuth();
  return (
    <Suspense
      fallback={
        <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
          <Spinner aria-label="Loading the question" />
        </div>
      }
    >
      <CsatInner />
    </Suspense>
  );
}
