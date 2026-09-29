// /accept-invite?token=… — the public invite-redemption page.
//
// Purpose: the customer-facing half of the org-users invite flow (Wave 11
// lane 2 backend, migration 0047). The invited member opens the one-time
// setup link out-of-band; this page accepts the setup token, takes a new
// password, and redeems `POST /org/users/accept-invite` — which returns a
// standard token pair, so the member lands logged in. Unknown / used /
// expired tokens are ALL a bare 404 server-side and render one friendly
// state here. Lives OUTSIDE the authed shell (headerless route).
// Non-purpose: no staff surfaces, no branding beyond Workbench tokens.
//
// Wave 17 lane 9 UX pass: Tangerine brand header, show/hide password
// affordances on both fields, icon'd error row, spinner submit, success
// check state. Security copy and testids unchanged; forced light theme
// preserved on the shared device.
"use client";

import type { components } from "@marula-core/sdk";
import { Button, Card, Input, Label, Spinner } from "@marula-core/ui";
import { applyTheme } from "@marula-core/ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";

import { AuthBrand } from "../../components/auth/auth-brand";
import { saveTokens } from "../../lib/api";
import { api } from "../../lib/api";
import { OFFLINE_RETRY_COPY } from "../../lib/error-copy";

/** Show/hide affordance shared by both password fields. */
function ToggleShowPassword({
  on,
  onClick,
}: {
  on: boolean;
  onClick: () => void;
}): ReactNode {
  return (
    <button
      type="button"
      aria-label={on ? "Hide password" : "Show password"}
      aria-pressed={on}
      onClick={onClick}
      className="absolute inset-y-0 right-0 flex w-9 items-center justify-center transition-opacity duration-100 hover:opacity-70"
      style={{ color: "var(--m-text-muted)" }}
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
        {on ? (
          <>
            <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6z" strokeLinejoin="round" />
            <circle cx="12" cy="12" r="2.5" />
            <path d="M4 4l16 16" strokeLinecap="round" />
          </>
        ) : (
          <>
            <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6z" strokeLinejoin="round" />
            <circle cx="12" cy="12" r="2.5" />
          </>
        )}
      </svg>
    </button>
  );
}

function AcceptInviteInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";

  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [confirm, setConfirm] = useState("");
  const [showConfirm, setShowConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  useEffect(() => {
    // Public page: always render in the customer theme (light), never
    // inherit a staff member's persisted preference on a shared device.
    applyTheme("light");
  }, []);

  const redeem = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    if (password.length < 10) {
      setError("Password must be at least 10 characters.");
      return;
    }
    if (password !== confirm) {
      setError("The two passwords do not match.");
      return;
    }
    setBusy(true);
    try {
      const { data, error: apiError } = await api.api.POST("/api/v1/org/users/accept-invite", {
        body: { token, password },
      });
      if (apiError !== undefined || data === undefined) {
        const body = apiError as { error?: { code?: number; message?: string } } | undefined;
        if (body?.error?.code === 404) {
          setError(
            "This invite link is not valid — it may already have been used or has expired. Ask your administrator for a fresh invite.",
          );
          return;
        }
        setError(body?.error?.message ?? "The invite could not be redeemed.");
        return;
      }
      // Standard token pair: persist exactly like a login and do a full
      // navigation so AuthProvider re-bootstraps from the fresh tokens.
      saveTokens(data as components["schemas"]["TokenPair"]);
      setDone(true);
      // Wave 26: first-run starts here (profile/MFA/orientation/defaults,
      // every step skippable); it lands on /today when resolved or skipped.
      window.location.assign("/onboarding");
    } catch {
      setError(OFFLINE_RETRY_COPY);
    } finally {
      setBusy(false);
    }
  };

  if (token === "") {
    return (
      <main className="flex min-h-full items-center justify-center p-4 md:p-6" style={{ background: "var(--m-canvas)" }}>
        <Card className="m-rise w-full max-w-sm rounded-xl p-6">
          <AuthBrand subtitle="Accept invite" />
          <p className="mt-3 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            This page needs the setup link your administrator sent you (it should look like
            {" "}
            <code className="font-mono text-[12px]">…/accept-invite?token=…</code>).
          </p>
          <Button className="mt-4" variant="secondary" onClick={() => router.replace("/")}>
            Go home
          </Button>
        </Card>
      </main>
    );
  }

  return (
    <main className="flex min-h-full items-center justify-center p-4 md:p-6" style={{ background: "var(--m-canvas)" }}>
      <Card className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7" data-testid="accept-invite-card">
        <AuthBrand />
        <h1 className="mt-4 text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
          Set your password
        </h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Finish setting up your Marula account. You will land signed in.
        </p>
        <form className="m-rise mt-4 grid gap-3" onSubmit={redeem} style={{ animationDelay: "40ms" }}>
          <div className="grid gap-1">
            <Label htmlFor="invite-password">New password</Label>
            <div className="relative">
              <Input
                id="invite-password"
                type={showPassword ? "text" : "password"}
                autoComplete="new-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="pr-12"
                required
                minLength={10}
              />
              <ToggleShowPassword on={showPassword} onClick={() => setShowPassword((v) => !v)} />
            </div>
          </div>
          <div className="grid gap-1">
            <Label htmlFor="invite-password-confirm">Confirm password</Label>
            <div className="relative">
              <Input
                id="invite-password-confirm"
                type={showConfirm ? "text" : "password"}
                autoComplete="new-password"
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                className="pr-12"
                required
              />
              <ToggleShowPassword on={showConfirm} onClick={() => setShowConfirm((v) => !v)} />
            </div>
          </div>
          {error !== null ? (
            <p
              role="alert"
              className="flex items-start gap-1.5 text-[13px]"
              style={{ color: "var(--m-status-danger-fg)" }}
              data-testid="accept-invite-error"
            >
              <svg aria-hidden="true" className="mt-0.5 shrink-0" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M12 3 2 21h20L12 3z" strokeLinejoin="round" />
                <path d="M12 10v5" strokeLinecap="round" />
                <circle cx="12" cy="18" r="0.5" fill="currentColor" />
              </svg>
              {error}
            </p>
          ) : null}
          <Button type="submit" variant="primary" disabled={busy || done}>
            {busy ? <Spinner /> : null} Set password and sign in
          </Button>
        </form>
      </Card>
    </main>
  );
}

export default function AcceptInvitePage(): ReactNode {
  return (
    <Suspense
      fallback={
        <main className="flex min-h-full items-center justify-center p-6" style={{ background: "var(--m-canvas)" }}>
          <Spinner />
        </main>
      }
    >
      <AcceptInviteInner />
    </Suspense>
  );
}
