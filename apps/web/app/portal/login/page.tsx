// /portal/login — the way in. No navigation, one card, the service desk's name.
//
// Purpose: the client's sign-in, exactly as the pack draws it: no navigation
// at all, a centred card under the SERVICE DESK's name, the line "Your
// tickets, invoices and quotes, in one place", and the footer crediting the
// service desk with their hours and the after-hours number.
//
// ONE DOCUMENTED DIVERGENCE FROM THE PACK. DESIGN-NOTES-portal.md §5 makes an
// emailed link the primary path and the password "a quiet alternative". The
// portal API has exactly one authentication endpoint —
// `POST /api/v1/portal/login`, email + password — and no link-request endpoint
// (checked against the pinned OpenAPI: portal/login, portal/refresh, portal/me
// and the read surfaces, nothing else). A primary button that cannot send a
// link would be a dead button, which this repo forbids outright, so the
// working action is the primary and the pack's composition is kept intact
// around it. The emailed-link path is a backend follow-up, not something a
// screen may pretend to do; it is recorded as an open item in the landing
// record rather than faked here.
//
// Non-purpose: no staff login, no registration, no SSO, no MFA. The portal is
// invite-only, provisioned by the service desk.
"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { friendlyMutationText } from "../../../lib/error-copy";
import { portalApi, usePortalAuth } from "../../../lib/portal-api";
import { DESK_NAME } from "../../../components/portal/portal-shell";
import { Field } from "../../../components/portal/portal-ui";

export default function PortalLoginPage() {
  const router = useRouter();
  const { status } = usePortalAuth();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  useEffect(() => {
    if (status === "authed") router.replace("/portal/tickets");
  }, [status, router]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await portalApi.login(email.trim(), password);
      router.replace("/portal/tickets");
    } catch (err) {
      // Blush, with words that say what happens next — never red, and never
      // "something went wrong" (DESIGN-NOTES-portal §2 and rule 2 of the
      // DERIVATION-GUIDE contract).
      setError(friendlyMutationText(err, "That email and password don't match. Try again, or ask your service desk to reset it."));
    } finally {
      setBusy(false);
    }
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
        <div className="portal-login__card">
          <div>
            <h1>Sign in</h1>
            <p className="portal-lead" style={{ margin: "4px 0 0", fontSize: 14 }}>
              Your tickets, invoices and quotes, in one place.
            </p>
          </div>
          <form className="portal-field" onSubmit={submit}>
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
                aria-describedby={error ? "portal-password-error" : "portal-password-hint"}
              />
            </Field>
            <button type="submit" className="portal-btn portal-btn--full" disabled={busy}>
              {busy ? "Signing in…" : "Sign in"}
            </button>
          </form>
        </div>
        <p className="portal-hint" style={{ textAlign: "center" }}>
          Not able to sign in? Ask your service desk to invite you.
        </p>
      </div>
    </div>
  );
}
