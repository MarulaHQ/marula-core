// /portal/login — the way in, with a link as the primary.
//
// WHY A LINK AND NOT A PASSWORD FIELD. The pack draws this as progressive
// disclosure: "Email me a sign-in link" is the primary, and the password is a
// quiet centred link underneath that reveals a field when someone needs it
// (portal/sand/states/01-login.png). The audience is a client's finance person
// who was emailed something and does not think of herself as using a system, so
// a password box she has to invent a password for is the wrong first thing to
// ask for. This was lane 13's first recorded divergence; it is now closed by
// the portal magic-link endpoints (migration 0119) rather than by weakening the
// copy.
//
// BOTH PATHS ARE REAL. The link is the default and it works
// (POST /api/v1/portal/magic-link/request + /confirm, portal's own one-use
// token family). "Use a password instead" reveals the existing password login,
// because a person who has a password and wants to use it should not be made to
// dig for their inbox.
//
// THE LINK HALF IS DENY-BY-DEFAULT, SO THE COPY IS DELIBERATELY VAGUER THAN THE
// API. The server answers `{ok:true}` for every address, so the screen says "if
// an account matches" and never claims a message reached a named person. The
// one thing it may name is the address the person just typed, which is theirs.
//
// WITH `?token` the page consumes the link on arrival and hands over, because a
// person who clicks a link in an email wants to be in, not to press a second
// button.
//
// Non-purpose: no registration, no SSO, no MFA. The portal is invite-only,
// provisioned by the service desk.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";
import { friendlyMutationText } from "../../../lib/error-copy";
import { portalApi, usePortalAuth } from "../../../lib/portal-api";
import { DESK_NAME } from "../../../components/portal/portal-shell";
import { Field } from "../../../components/portal/portal-ui";

/** Seconds before the resend is offered again, so nobody hammers the button. */
const RESEND_SECONDS = 60;

function mmss(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function PortalLoginInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";
  const { status } = usePortalAuth();

  const [mode, setMode] = useState<"link" | "password">("link");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  const [countdown, setCountdown] = useState(RESEND_SECONDS);
  const [consuming, setConsuming] = useState(token !== "");

  useEffect(() => {
    if (status === "authed") router.replace("/portal/tickets");
  }, [status, router]);

  // Consume the emailed link on arrival, then hand over.
  useEffect(() => {
    if (token === "") return;
    let cancelled = false;
    (async () => {
      setConsuming(true);
      setError(null);
      try {
        await portalApi.confirmMagicLink(token);
        if (!cancelled) {
          // Full navigation so the portal session is picked up everywhere.
          window.location.assign("/portal/tickets");
        }
      } catch (err) {
        if (!cancelled) {
          setConsuming(false);
          setError(
            friendlyMutationText(
              err,
              "This sign-in link is invalid, used, or expired. Ask for a fresh one and it will arrive in a moment.",
            ),
          );
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [token]);

  // The resend cool-down ticks for real. The pack freezes its digits for the
  // screenshot; a frozen button in a product would be a lie.
  useEffect(() => {
    if (!sent) return;
    setCountdown(RESEND_SECONDS);
    const timer = window.setInterval(() => {
      setCountdown((value) => (value <= 1 ? 0 : value - 1));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [sent]);

  async function sendLink(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await portalApi.requestMagicLink(email.trim());
      setSent(true);
    } catch (err) {
      setError(
        friendlyMutationText(err, "We couldn't send that link. Try again in a moment."),
      );
    } finally {
      setBusy(false);
    }
  }

  async function signInWithPassword(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await portalApi.login(email.trim(), password);
      router.replace("/portal/tickets");
    } catch (err) {
      // Blush, with words that say what happens next — never red, and never
      // "something went wrong" (DESIGN-NOTES-portal §2, contract rule 2).
      setError(
        friendlyMutationText(
          err,
          "That email and password don't match. Try again, or ask your service desk to reset it.",
        ),
      );
    } finally {
      setBusy(false);
    }
  }

  if (consuming) {
    return (
      <div className="portal-login" data-testid="portal-login">
        <div className="portal-login__desk">
          <span className="portal-desk__mark" aria-hidden />
          <span className="portal-desk">{DESK_NAME}</span>
        </div>
        <div className="portal-login__card">
          <h1>Signing you in…</h1>
          <p className="portal-lead" style={{ margin: "4px 0 0", fontSize: 14 }}>
            One moment while we check your link.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div
      style={{ display: "flex", flex: 1, flexDirection: "column" }}
      data-testid="portal-login"
    >
      <div style={{ height: 56 }} />
      <div className="portal-login">
        <div className="portal-login__desk">
          <span className="portal-desk__mark" aria-hidden />
          <span className="portal-desk">{DESK_NAME}</span>
        </div>

        {sent && mode === "link" ? (
          <div className="portal-login__card" data-testid="portal-link-sent">
            <div>
              <h1>Your link is on its way.</h1>
              <p className="portal-lead" style={{ margin: "4px 0 0", fontSize: 14 }}>
                If an account matches{" "}
                <b style={{ color: "var(--m-text)" }}>{email.trim()}</b>, a sign-in link is on
                its way. It works once and lasts 30 minutes.
              </p>
            </div>
            <button
              type="button"
              className="portal-btn portal-btn--secondary portal-btn--full"
              disabled={countdown > 0 || busy}
              onClick={(e) => void sendLink(e)}
              data-testid="portal-link-resend"
            >
              {countdown > 0 ? `Send it again in ${mmss(countdown)}` : "Send it again"}
            </button>
            <div style={{ textAlign: "center" }}>
              <button
                type="button"
                className="portal-link"
                style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                onClick={() => {
                  setSent(false);
                  setError(null);
                }}
              >
                Use a different email
              </button>
            </div>
            <p className="portal-note">
              <PortalNoteIcon />
              <span>
                Nothing in your inbox? Check spam, and make sure you used the address your service
                desk invited.
              </span>
            </p>
          </div>
        ) : (
          <div className="portal-login__card">
            <div>
              <h1>Sign in</h1>
              <p className="portal-lead" style={{ margin: "4px 0 0", fontSize: 14 }}>
                Your tickets, invoices and quotes, in one place.
              </p>
            </div>

            {mode === "link" ? (
              <form className="portal-field" onSubmit={sendLink}>
                <Field label="Work email" htmlFor="portal-email">
                  <input
                    id="portal-email"
                    className="portal-input"
                    type="email"
                    autoComplete="email"
                    required
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    placeholder="you@company.com"
                  />
                </Field>
                {error !== null && (
                  <p className="portal-field__error" role="alert" data-testid="portal-link-error">
                    <PortalNoteIcon />
                    <span>{error}</span>
                  </p>
                )}
                <button
                  type="submit"
                  className="portal-btn portal-btn--full"
                  disabled={busy}
                  data-testid="portal-link-send"
                >
                  {busy ? "Sending…" : "Email me a sign-in link"}
                </button>
              </form>
            ) : (
              <form className="portal-field" onSubmit={signInWithPassword}>
                <Field label="Work email" htmlFor="portal-email">
                  <input
                    id="portal-email"
                    className="portal-input"
                    type="email"
                    autoComplete="email"
                    required
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    placeholder="you@company.com"
                  />
                </Field>
                <Field
                  label="Password"
                  htmlFor="portal-password"
                  error={error}
                  hint="Your service desk sets this, and can reset it for you."
                >
                  <input
                    id="portal-password"
                    className="portal-input"
                    type="password"
                    autoComplete="current-password"
                    required
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    aria-invalid={error !== null}
                    aria-describedby={
                      error ? "portal-password-error" : "portal-password-hint"
                    }
                  />
                </Field>
                <button
                  type="submit"
                  className="portal-btn portal-btn--full"
                  disabled={busy}
                  data-testid="portal-password-submit"
                >
                  {busy ? "Signing in…" : "Sign in"}
                </button>
              </form>
            )}

            {/* The quiet alternative, exactly as the pack draws it: a centred
                link that swaps the form rather than a second button. */}
            <div style={{ textAlign: "center" }}>
              {mode === "link" ? (
                <button
                  type="button"
                  className="portal-link"
                  style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                  onClick={() => {
                    setMode("password");
                    setError(null);
                  }}
                  data-testid="portal-use-password"
                >
                  Use a password instead
                </button>
              ) : (
                <button
                  type="button"
                  className="portal-link"
                  style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                  onClick={() => {
                    setMode("link");
                    setError(null);
                  }}
                >
                  Use a link instead
                </button>
              )}
            </div>
          </div>
        )}

        <p className="portal-hint" style={{ textAlign: "center" }}>
          Not able to sign in? Ask your service desk to invite you.
        </p>
      </div>
    </div>
  );
}

/** The small note glyph, so the two note blocks match. */
function PortalNoteIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      aria-hidden
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5M12 8h.01" />
    </svg>
  );
}

export default function PortalLoginPage(): ReactNode {
  return (
    <Suspense
      fallback={
        <div style={{ display: "flex", flex: 1 }} data-testid="portal-login" />
      }
    >
      <PortalLoginInner />
    </Suspense>
  );
}
