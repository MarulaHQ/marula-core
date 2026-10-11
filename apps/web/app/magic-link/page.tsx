// /magic-link — a link on its way. Rust, because this moment welcomes.
//
// Purpose: passwordless sign-in. WITHOUT `?token` it is the request half; the
// pack's state 04 is the answer, and the answer is the whole design: the
// headline in the brand's serif, the address named back, the two facts that
// matter ("it works once and lasts 15 minutes"), a resend with a visible
// countdown so nobody hammers the button, and one sentence about spam.
//
// ONE UNIFORM ANSWER, AND WHY THE COPY IS DELIBERATELY VAGUER THAN THE API.
// This endpoint is deny-by-default: an org without magic-link sign-in is
// indistinguishable from an unknown address, and the server answers
// `{ok:true}` either way. So the screen says "if an account matches, a link is
// on its way" and never claims a message was sent to a specific person. The
// pack's shorter line is kept for the address the person typed, which is
// theirs alone and reveals nothing.
//
// WITH `?token` it consumes the mail link, through the same transport the SDK
// uses (credentials + `x-marula-csrf` echo), lands a token pair, and hands over
// to the app with a full navigation so AuthProvider re-bootstraps.
//
// Non-purpose: this is the STAFF family. The portal has its own sign-in and
// its own token family, and this page is not part of it.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";
import { applyTheme } from "@marula-core/ui";
import { Spinner } from "@marula-core/ui";
import {
  EntryCard,
  EntryFruit,
  EntryHeadline,
  EntryNote,
  EntryShell,
  EntrySprout,
} from "../../components/auth/entry";
import { applyEntryTheme } from "../../lib/entry-theme";
import { friendlyMutationText } from "../../lib/error-copy";
import { api, resolveApiBaseUrl, saveTokens } from "../../lib/api";

const API_BASE = resolveApiBaseUrl();

/** Seconds before the resend is offered again. Matches the server's own
 *  cool-down closely enough that the button is never a way to get rate
 *  limited by accident. */
const RESEND_SECONDS = 60;

/** Echo of the SDK's csrf double-submit (cookie-less pre-auth calls are an
 *  unguarded pass, but a sitting session must present the header). */
function csrfEcho(): Record<string, string> {
  if (typeof document === "undefined") return {};
  const csrf = document.cookie
    .split(";")
    .map((c) => c.trim())
    .find((c) => c.startsWith("marula_csrf="));
  return csrf ? { "x-marula-csrf": csrf.slice("marula_csrf=".length) } : {};
}

function mmss(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

function MagicLinkInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sentOnce, setSentOnce] = useState(false);
  const [done, setDone] = useState(false);
  const [countdown, setCountdown] = useState(RESEND_SECONDS);

  useEffect(() => {
    // Public page: always render in the light theme, never inherit a staff
    // member's persisted preference on a shared device.
    applyEntryTheme();
  }, []);

  // The resend cool-down. The pack freezes its digits for the screenshot; the
  // product counts down for real, because a frozen button would be a lie.
  useEffect(() => {
    if (!sentOnce) return;
    setCountdown(RESEND_SECONDS);
    const timer = window.setInterval(() => {
      setCountdown((value) => (value <= 1 ? 0 : value - 1));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [sentOnce]);

  // Auto-confirm the mail link (same transport shape the SDK authCall uses —
  // credentials + csrf echo — against the same envelope errors).
  useEffect(() => {
    if (token === "" || done || busy) return;
    let cancelled = false;
    (async () => {
      setBusy(true);
      setError(null);
      try {
        const response = await fetch(`${API_BASE}/api/v1/auth/magic-link/confirm`, {
          method: "POST",
          headers: { "content-type": "application/json", ...csrfEcho() },
          credentials: "include",
          body: JSON.stringify({ token }),
        });
        if (!response.ok) {
          const envelope = (await response.json().catch(() => null)) as {
            error?: { code?: number; reason?: string };
          } | null;
          if (response.status === 401) {
            setError(
              "This sign-in link is invalid, used, or expired. Ask for a fresh one and it will arrive in a moment.",
            );
            return;
          }
          if (response.status === 429) {
            setError(friendlyMutationText({ body: envelope, status: response.status }));
            return;
          }
          if (response.status === 403) {
            setError(
              "This browser didn't present the cross-site check the link needs. Ask for a fresh one from this browser.",
            );
            return;
          }
          setError(friendlyMutationText(envelope ?? new Error("magic-link confirm failed")));
          return;
        }
        const tokens = (await response.json()) as Parameters<typeof saveTokens>[0];
        saveTokens(tokens);
        setDone(true);
        // Full navigation so AuthProvider re-bootstraps from the fresh tokens.
        window.location.assign("/today");
      } catch {
        if (!cancelled) setError("You're offline — check your connection and try again.");
      } finally {
        if (!cancelled) setBusy(false);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, done]);

  const sendLink = async () => {
    setBusy(true);
    setError(null);
    try {
      const { error: apiError } = await api.api.POST("/api/v1/auth/magic-link/request", {
        body: { email: email.trim() },
      });
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

  const requestLink = async (e: React.FormEvent) => {
    e.preventDefault();
    await sendLink();
  };

  if (token !== "") {
    return (
      <EntryShell background="rust" fruit="none">
        <EntryCard>
          {done ? (
            <div className="entry-centred" role="status">
              <h2 style={{ margin: 0, fontSize: 20, fontWeight: 600 }}>Signed in</h2>
              <p className="entry-sub">Taking you into Marula…</p>
            </div>
          ) : (
            <div style={{ display: "flex", justifyContent: "center", padding: "24px 0" }}>
              {busy ? (
                <Spinner label="Consuming your sign-in link" />
              ) : (
                <p className="entry-error" role="alert">
                  {error}
                </p>
              )}
            </div>
          )}
        </EntryCard>
      </EntryShell>
    );
  }

  return (
    <EntryShell background="rust" fruit="half">
      <EntryCard data-testid="magic-link-card">
        {sentOnce ? (
          <>
            <div className="entry-centred">
              <EntrySprout />
              <EntryFruit half />
              <EntryHeadline>Your link is on its way.</EntryHeadline>
              <p className="entry-sub">
                If an account matches <b style={{ color: "var(--m-text)" }}>{email.trim()}</b>, a
                sign-in link is on its way. It works once and lasts 15 minutes.
              </p>
            </div>
            <button
              type="button"
              className="entry-btn entry-btn--secondary entry-btn--full"
              disabled={countdown > 0 || busy}
              onClick={() => void sendLink()}
              data-testid="magic-link-resend"
            >
              {countdown > 0 ? `Send it again in ${mmss(countdown)}` : "Send it again"}
            </button>
            <div style={{ textAlign: "center" }}>
              <button
                type="button"
                className="entry-link"
                style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                onClick={() => {
                  setSentOnce(false);
                  setCountdown(RESEND_SECONDS);
                  setError(null);
                }}
                data-testid="magic-link-different-email"
              >
                Use a different email
              </button>
            </div>
            {/* The pack words this to the ADMIN, not the service desk: the
                person who invited them is who can check the address.
                (auth/neutral/states/04-magic-link.png) */}
            <EntryNote>
              Nothing in your inbox? Check spam, and make sure you used the address your admin
              invited.
            </EntryNote>
          </>
        ) : (
          <>
            <div>
              <EntryHeadline>Sign in with a link.</EntryHeadline>
              <p className="entry-sub">
                Enter your work email and we&apos;ll send a link. No password to remember, and the
                link signs in this browser for a full session.
              </p>
            </div>
            <form
              onSubmit={requestLink}
              className="entry-field"
              style={{ display: "flex", flexDirection: "column", gap: 16 }}
            >
              <label className="entry-label" htmlFor="magic-email">
                Work email
              </label>
              <input
                id="magic-email"
                className="entry-input"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="you@company.com"
                autoComplete="email"
                required
              />
              {error !== null && (
                <p className="entry-error" role="alert" data-testid="magic-link-error">
                  {error}
                </p>
              )}
              {/* The submit MUST live inside the form: outside it, type=submit
                  is inert and the button does nothing — the pack's state 04 is
                  only reachable through a submit, and Enter-only submission
                  hides a dead primary control. (/login gets this right.) */}
              <button
                type="submit"
                className="entry-btn"
                disabled={busy}
                data-testid="magic-link-send"
              >
                {busy ? "Sending…" : "Email me a sign-in link"}
              </button>
            </form>
            <EntryNote>
              We only use your address to get you in. Nothing else is sent to it.
            </EntryNote>
            {/* The pack's sent card (state 04) draws no way back — the link
                belongs to the request half, where going back is still on the
                person's mind. */}
            <button
              type="button"
              className="entry-link"
              style={{ fontWeight: 500, color: "var(--m-text-secondary)", textAlign: "center" }}
              onClick={() => router.replace("/login")}
              data-testid="magic-link-back-to-signin"
            >
              Back to sign in
            </button>
          </>
        )}
      </EntryCard>
    </EntryShell>
  );
}

export default function MagicLinkPage(): ReactNode {
  return (
    <Suspense
      fallback={
        <div className="entry-root entry-root--rust">
          <main className="entry-main">
            <div className="entry-card">
              <Spinner />
            </div>
          </main>
        </div>
      }
    >
      <MagicLinkInner />
    </Suspense>
  );
}
