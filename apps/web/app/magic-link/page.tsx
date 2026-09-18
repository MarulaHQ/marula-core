// /magic-link — passwordless sign-in request + confirm surface (Wave 19
// lane 5). Deny-by-default org setting `orgs.magic_link_sign_in` — an
// org without it is indistinguishable from an unknown email (uniform
// {ok:true} either way). STAFF family only (the portal is NOT included).
//
// Purpose: WITHOUT ?token= the REQUEST half — email → POST /request with
// one uniform answer and typed 429 `auth_rate_limited`. WITH ?token= the
// mail-link confirm: the link's POST goes through the SDK transport
// mechanics (credentials + `x-meridian-csrf` echo, mirroring authCall in
// @meridian/sdk — the double-submit guard applies when a session cookie
// already exists) and lands a TokenPair, stored through the same
// memory-jar path as login, then a full navigation so AuthProvider
// re-bootstraps. Envelope failures are typed, never fabricated.
"use client";

import { Button, Card, Input, Label, Spinner } from "@meridian/ui";
import { applyTheme } from "@meridian/ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";

import { AuthBrand } from "../../components/auth/auth-brand";
import { friendlyMutationText } from "../../lib/error-copy";
import { resolveApiBaseUrl, saveTokens } from "../../lib/api";

const API_BASE = resolveApiBaseUrl();

/** Echo of the SDK's csrf double-submit (cookie-less pre-auth calls are
 * an unguarded PASS, but a sitting session must present the header). */
function csrfEcho(): Record<string, string> {
  if (typeof document === "undefined") return {};
  const csrf = document.cookie
    .split(";")
    .map((c) => c.trim())
    .find((c) => c.startsWith("meridian_csrf="));
  return csrf ? { "x-meridian-csrf": csrf.slice("meridian_csrf=".length) } : {};
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

  useEffect(() => {
    // Public page: always render in the customer theme (light), never
    // inherit a staff member's persisted preference on a shared device.
    applyTheme("light");
  }, []);

  // Auto-confirm the mail link (same transport shape the SDK authCall
  // uses — credentials + csrf echo — against the same envelope errors).
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
            setError("This sign-in link is invalid, used, or expired. Request a fresh one from the sign-in page.");
            return;
          }
          if (response.status === 429) {
            setError(friendlyMutationText({ body: envelope, status: response.status }));
            return;
          }
          if (response.status === 403) {
            setError("This browser didn't present the cross-site sign-in check required to consume the link. Request a fresh one.");
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

  const requestLink = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const { error: apiError } = await apiCall:
      // typed path call
      const res = await api.api.POST("/api/v1/auth/magic-link/request", {
        body: { email: email.trim() },
      });
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      setSentOnce(true);
    } catch {
      setError("You're offline — check your connection and try again.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <main
      className="flex min-h-full items-center justify-center p-4 md:p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7" data-testid="magic-link-card">
        <AuthBrand />
        {token === "" ? (
          <>
            <h1 className="mt-4 font-heading text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
              Sign in with a link
            </h1>
            {sentOnce ? (
              <p className="mt-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }} role="status">
                If passwordless sign-in is enabled for your workspace and an
                account matches <strong>{email.trim()}</strong>, a single-use
                sign-in link (valid 15 minutes) is on its way. Follow it to
                land signed in.
              </p>
            ) : (
              <form onSubmit={requestLink} className="mt-2 grid gap-3">
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  Enter your email and follow the link we send — no password
                  needed. Signed-in links are the same full sessions a
                  password sign-in issues.
                </p>
                <label className="grid gap-1">
                  <Label htmlFor="magic-email">Email</Label>
                  <Input
                    id="magic-email"
                    type="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    placeholder="you@company.com"
                    autoComplete="email"
                    required
                  />
                </label>
                {error !== null && (
                  <p
                    role="alert"
                    className="text-[13px]"
                    style={{ color: "var(--m-status-danger-fg)" }}
                    data-testid="magic-link-error"
                  >
                    {error}
                  </p>
                )}
                <Button type="submit" variant="primary" disabled={busy}>
                  {busy ? <Spinner /> : null} Send sign-in link
                </Button>
              </form>
            )}
            <button
              type="button"
              onClick={() => router.replace("/login")}
              className="mt-5 w-full text-xs underline underline-offset-2 transition-opacity duration-100 hover:opacity-70"
              style={{ color: "var(--m-text-muted)" }}
            >
              Back to sign-in
            </button>
          </>
        ) : done ? (
          <div className="m-rise mt-2 text-center" style={{ animationDelay: "40ms" }}>
            <span
              aria-hidden="true"
              className="mx-auto mb-3 flex h-10 w-10 items-center justify-center rounded-full"
              style={{
                background: "var(--m-status-success-bg)",
                color: "var(--m-status-success-fg)",
              }}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M4 12.5 10 18 20 6" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
            <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              Signed in — continuing into Meridian…
            </p>
          </div>
        ) : (
          <div className="mt-2 flex items-center justify-center py-6">
            {busy ? (
              <Spinner label="Consuming sign-in link" />
            ) : (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {error}
              </p>
            )}
          </div>
        )}
      </Card>
    </main>
  );
}

export default function MagicLinkPage(): ReactNode {
  return (
    <Suspense
      fallback={
        <main className="flex min-h-full items-center justify-center p-6" style={{ background: "var(--m-canvas)" }}>
          <Spinner />
        </main>
      }
    >
      <MagicLinkInner />
    </Suspense>
  );
}
