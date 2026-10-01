// /verify-email — nearly there. Rust, because this moment welcomes.
//
// Purpose: the pack's state 07. The fruit line drawing, the serif headline
// "Nearly there", the address the link went to, a resend, a wrong-address
// escape, and the two facts that settle the waiting: the link lasts 24 hours,
// and this page can be closed — it will carry on when the link is opened.
//
// The confirm half runs the token automatically when one is present, because a
// person who clicks a link in an email wants to be in, not to press a second
// button. The resend half stays available on the same screen, because the most
// common reason somebody is here is that the first email went to spam.
//
// Non-purpose: no address editing (that is a different, authenticated flow), no
// delivery diagnostics, and no "we'll remind you" nagging.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";
import { Spinner } from "@marula-core/ui";
import { applyEntryTheme } from "../../lib/entry-theme";
import { friendlyMutationText } from "../../lib/error-copy";
import { api } from "../../lib/api";
import {
  EntryCard,
  EntryHeadline,
  EntryIcon,
  EntryNote,
  EntryShell,
} from "../../components/auth/entry";

function VerifyEmailInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<boolean | null>(null);
  const [sentOnce, setSentOnce] = useState(false);

  useEffect(() => {
    // Public page: always render in the light theme, never inherit a staff
    // member's persisted preference on a shared device.
    applyEntryTheme();
  }, []);

  const confirmToken = async () => {
    setBusy(true);
    setError(null);
    try {
      const { data, error: apiError } = await api.api.POST(
        "/api/v1/auth/email-verification/confirm",
        { body: { token } },
      );
      if (apiError !== undefined || data === undefined) {
        const body = apiError as { error?: { code?: number } } | undefined;
        if (body?.error?.code === 401) {
          setError(
            "This verification link is invalid, used, or expired. Ask for a fresh one and it will arrive in a moment.",
          );
          setDone(false);
          return;
        }
        setError(friendlyMutationText(apiError));
        setDone(false);
        return;
      }
      setDone(data.email_verified);
    } catch {
      setError("You're offline — check your connection and try again.");
    } finally {
      setBusy(false);
    }
  };

  // Auto-confirm when a token is present; the effect stays at top level (hooks
  // rules) and self-guards on the empty-token case.
  useEffect(() => {
    if (token === "") return;
    if (done === null && !busy && error === null) void confirmToken();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, done, busy, error]);

  const requestVerification = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const { error: apiError } = await api.api.POST(
        "/api/v1/auth/email-verification/request",
        { body: { email: email.trim() } },
      );
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError));
        return;
      }
      setSentOnce(true);
    } catch {
      setError("You're offline — check your connection and try again.");
    } finally {
      setBusy(false);
    }
  };

  // The confirm half: the link carried the token, so there is nothing to press.
  if (token !== "") {
    return (
      <EntryShell background="rust" fruit="half">
        <EntryCard testId="verify-email-card">
          {busy ? (
            <div style={{ display: "flex", justifyContent: "center", padding: "24px 0" }}>
              <Spinner label="Verifying your email" />
            </div>
          ) : done === true ? (
            <>
              <div className="entry-centred">
                <EntryHeadline>Verified.</EntryHeadline>
                <p className="entry-sub">
                  Your email address is confirmed. Carry on and we&apos;ll take you to Marula.
                </p>
              </div>
              <button
                type="button"
                className="entry-btn"
                onClick={() => router.replace("/today")}
                data-testid="verify-continue"
              >
                Continue
              </button>
            </>
          ) : done === false ? (
            <>
              <div className="entry-centred">
                <EntryHeadline>Not quite.</EntryHeadline>
                <p className="entry-sub">{error}</p>
              </div>
              <button
                type="button"
                className="entry-btn"
                onClick={() => router.replace("/login")}
              >
                Back to sign in
              </button>
            </>
          ) : null}
        </EntryCard>
      </EntryShell>
    );
  }

  // The request half: the pack's state 07.
  return (
    <EntryShell background="rust" fruit="half">
      <EntryCard testId="verify-email-card">
        <div className="entry-centred">
          <EntryHeadline>Nearly there.</EntryHeadline>
          <p className="entry-sub">
            Verify your email.{" "}
            {sentOnce ? (
              <>
                If an account matches{" "}
                <b style={{ color: "var(--m-text)" }}>{email.trim()}</b>, a fresh link is on its
                way.
              </>
            ) : (
              <>
                We sent a link to the address you signed up with. Open it and you&apos;re done.
              </>
            )}
          </p>
        </div>

        {sentOnce ? (
          <button
            type="button"
            className="entry-btn entry-btn--secondary entry-btn--full"
            onClick={() => {
              setSentOnce(false);
              setError(null);
            }}
            data-testid="verify-resend-again"
          >
            Resend the email
          </button>
        ) : (
          <form onSubmit={requestVerification} className="entry-field">
            <label className="entry-label" htmlFor="verify-email">
              Work email
            </label>
            <input
              id="verify-email"
              className="entry-input"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@company.com"
              autoComplete="email"
              required
            />
            {error !== null && (
              <p className="entry-error" role="alert" data-testid="verify-error">
                {error}
              </p>
            )}
            <button type="submit" className="entry-btn" disabled={busy} data-testid="verify-send">
              {busy ? "Sending…" : "Resend the email"}
            </button>
          </form>
        )}

        <div style={{ textAlign: "center" }}>
          <button
            type="button"
            className="entry-link"
            style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
            onClick={() => router.replace("/login")}
          >
            Wrong address? Change it
          </button>
        </div>

        <EntryNote>
          The link lasts 24 hours. You can close this page. It will carry on when you open the link.
        </EntryNote>

        {sentOnce && (
          <p className="entry-hint" style={{ textAlign: "center", margin: 0 }}>
            <EntryIcon name="mail" size={14} /> Nothing after a few minutes? Check spam, then send
            it again.
          </p>
        )}
      </EntryCard>
    </EntryShell>
  );
}

export default function VerifyEmailPage(): ReactNode {
  return (
    <Suspense fallback={<EntryShell background="rust"><EntryCard /></EntryShell>}>
      <VerifyEmailInner />
    </Suspense>
  );
}
