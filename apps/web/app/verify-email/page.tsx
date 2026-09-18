// /verify-email — email verification confirm/resend surface (Wave 19
// lane 5). The mail link lands here (core-api link builder pages
// "verify-email", 24h token).
//
// Purpose: WITH ?token= POSTs /api/v1/auth/email-verification/confirm and
// asserts the typed VerifyStatus answer; WITHOUT it, the REQUEST half —
// an email resend form that POSTs /request and always shows one uniform
// answer (no mailbox enumeration). Typed failures only: 429
// `auth_rate_limited` (throttled resend), 401 envelope on an unknown /
// used / expired token — never fabricated success.
// Non-purpose: no backend logic; the admin email-verified override lives
// in the team admin surfaces.
"use client";

import { Button, Card, Input, Label, Spinner } from "@meridian/ui";
import { applyTheme } from "@meridian/ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";

import { AuthBrand } from "../../components/auth/auth-brand";
import { friendlyMutationText } from "../../lib/error-copy";
import { api } from "../../lib/api";

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
    // Public page: always render in the customer theme (light), never
    // inherit a staff member's persisted preference on a shared device.
    applyTheme("light");
  }, []);

  // Auto-confirm when a token is present; the effect stays at top level
  // (hooks rules) and self-guards on the empty-token case.
  useEffect(() => {
    if (token === "") return;
    if (done === null && !busy && error === null) void confirmToken();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, done, busy, error]);

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
            "This verification link is invalid, used, or expired — request a fresh verification email below.",
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

  function Body(): ReactNode {
    return (
      <Card className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7" data-testid="verify-email-card">
        <AuthBrand />
        {token === "" ? (
          // --- REQUEST (RESEND) MODE ---------------------------------
          <>
            <h1 className="mt-4 font-heading text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
              Verify your email
            </h1>
            {sentOnce ? (
              <p className="mt-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }} role="status">
                If an unverified account exists for <strong>{email.trim()}</strong>,
                a fresh single-use verification link is on its way.
              </p>
            ) : (
              <form onSubmit={requestVerification} className="mt-2 grid gap-3">
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  Enter the email you signed up with and we&apos;ll send a
                  single-use verification link (valid 24 hours).
                </p>
                <label className="grid gap-1">
                  <Label htmlFor="verify-email">Email</Label>
                  <Input
                    id="verify-email"
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
                    data-testid="verify-email-error"
                  >
                    {error}
                  </p>
                )}
                <Button type="submit" variant="primary" disabled={busy}>
                  {busy ? <Spinner /> : null} Send verification link
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
        ) : done === true ? (
          // --- CONFIRMED ---------------------------------------------
          <div className="m-rise mt-4 grid gap-4" style={{ animationDelay: "40ms" }}>
            <span
              aria-hidden="true"
              className="mx-auto flex h-10 w-10 items-center justify-center rounded-full"
              style={{
                background: "var(--m-status-success-bg)",
                color: "var(--m-status-success-fg)",
              }}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M4 12.5 10 18 20 6" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
            <h1 className="text-center font-heading text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
              Email verified
            </h1>
            <p className="text-center text-[13px]" style={{ color: "var(--m-text-secondary)" }} role="status">
              Your email is confirmed. You can sign in with your usual
              password.
            </p>
            <Button variant="primary" onClick={() => router.replace("/login")}>
              Back to sign-in
            </Button>
          </div>
        ) : done === false ? (
          // --- CONFIRM FAILED → resend --------------------------------
          <div className="m-rise mt-4 grid gap-2" style={{ animationDelay: "40ms" }}>
            <h1 className="font-heading text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
              Email verification
            </h1>
            {error !== null && (
              <p
                role="alert"
                className="text-[13px]"
                style={{ color: "var(--m-status-danger-fg)" }}
              >
                {error}
              </p>
            )}
            <button
              type="button"
              onClick={() => {
                window.history.replaceState(null, "", "/verify-email");
                router.refresh();
              }}
              className="text-xs underline underline-offset-2 transition-opacity duration-100 hover:opacity-70"
              style={{ color: "var(--m-accent)" }}
            >
              Send a new verification email
            </button>
          </div>
        ) : (
          // --- CONFIRMING ---------------------------------------------
          <div className="mt-4 flex items-center justify-center py-6">
            <Spinner label="Verifying email" />
          </div>
        )}
      </Card>
    );
  }

  return (
    <main
      className="flex min-h-full items-center justify-center p-4 md:p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Body />
    </main>
  );
}

export default function VerifyEmailPage(): ReactNode {
  return (
    <Suspense
      fallback={
        <main className="flex min-h-full items-center justify-center p-6" style={{ background: "var(--m-canvas)" }}>
          <Spinner />
        </main>
      }
    >
      <VerifyEmailInner />
    </Suspense>
  );
}
