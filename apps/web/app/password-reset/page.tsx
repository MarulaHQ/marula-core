// /password-reset — public password recovery surface (Wave 19 lane 5).
//
// Purpose: dual mode. WITHOUT ?token= it is the REQUEST half — takes an
// email, POSTs /api/v1/auth/password-reset/request, and always shows one
// uniform answer (the endpoint answers {ok:true} identically for known
// and unknown mailboxes — no account enumeration). WITH ?token= (the
// single-use mail link lands here; core-api link builder pages
// "password-reset") it is the CONFIRM half — new password + confirm →
// POST /confirm; the confirm revokes ALL of the user's refresh sessions
// server-side, then the member returns to /login to sign in fresh.
// Typed failure states only: 429 `auth_rate_limited` on the request
// surface, 401 envelope on an unknown/used/expired confirm token —
// never fabricated success.
// Non-purpose: no backend logic, no admin consent surfaces; the mail
// itself comes from core-mail outbox.
"use client";

import { Button, Card, FieldHint, Input, Label, Spinner } from "@meridian/ui";
import { applyTheme } from "@meridian/ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";

import { AuthBrand } from "../../components/auth/auth-brand";
import { friendlyMutationText } from "../../lib/error-copy";
import { api } from "../../lib/api";

/** Show/hide affordance shared by the confirm-mode password fields. */
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
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        aria-hidden="true"
      >
        {on ? (
          <>
            <path
              d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6z"
              strokeLinejoin="round"
            />
            <circle cx="12" cy="12" r="2.5" />
            <path d="M4 4l16 16" strokeLinecap="round" />
          </>
        ) : (
          <>
            <path
              d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6z"
              strokeLinejoin="round"
            />
            <circle cx="12" cy="12" r="2.5" />
          </>
        )}
      </svg>
    </button>
  );
}

function PasswordResetInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";
  const [mode, setMode] = useState<"idle" | "sent" | "done">(
    token === "" ? "idle" : "idle",
  );
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [showConfirm, setShowConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Public page: always render in the customer theme (light), never
    // inherit a staff member's persisted preference on a shared device.
    applyTheme("light");
  }, []);

  const requestReset = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const { error: apiError } = await api.api.POST(
        "/api/v1/auth/password-reset/request",
        { body: { email: email.trim() } },
      );
      if (apiError !== undefined) {
        // 429 auth_rate_limited + generic envelopes both land here.
        setError(friendlyMutationText(apiError));
        return;
      }
      setMode("sent");
    } catch {
      setError("You're offline — check your connection and try again.");
    } finally {
      setBusy(false);
    }
  };

  const confirmReset = async (e: React.FormEvent) => {
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
      const { error: apiError } = await api.api.POST(
        "/api/v1/auth/password-reset/confirm",
        { body: { token, new_password: password } },
      );
      if (apiError !== undefined) {
        const body = apiError as { error?: { code?: number } } | undefined;
        if (body?.error?.code === 401) {
          setError(
            "This reset link is invalid, used, or expired — request a fresh reset email.",
          );
          return;
        }
        setError(friendlyMutationText(apiError));
        return;
      }
      setMode("done");
    } catch {
      setError("You're offline — check your connection and try again.");
    } finally {
      setBusy(false);
    }
  };

  if (token === "") {
    // --- REQUEST MODE -----------------------------------------------
    return (
      <main
        className="flex min-h-full items-center justify-center p-4 md:p-6"
        style={{ background: "var(--m-canvas)" }}
      >
        <Card className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7" data-testid="password-reset-card">
          <AuthBrand />
          <h1 className="mt-4 text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
            Reset your password
          </h1>
          {mode === "sent" ? (
            <div className="m-rise mt-2 grid gap-4" style={{ animationDelay: "40ms" }}>
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }} role="status">
                If an account exists for <strong>{email.trim()}</strong>, a
                single-use reset link is on its way (valid 30 minutes).
                Every previous link is invalidated when you ask again.
              </p>
              <Button variant="secondary" onClick={() => router.replace("/login")} className="w-full">
                Back to sign-in
              </Button>
            </div>
          ) : (
            <form onSubmit={requestReset} className="mt-2 grid gap-3">
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Enter the email you sign in with. We&apos;ll send a single-use
                reset link.
              </p>
              <label className="grid gap-1">
                <Label htmlFor="reset-email">Email</Label>
                <Input
                  id="reset-email"
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
                  data-testid="password-reset-error"
                >
                  {error}
                </p>
              )}
              <Button type="submit" variant="primary" disabled={busy}>
                {busy ? <Spinner /> : null} Send reset link
              </Button>
              <button
                type="button"
                onClick={() => router.replace("/login")}
                className="text-xs underline underline-offset-2 transition-opacity duration-100 hover:opacity-70"
                style={{ color: "var(--m-text-muted)" }}
              >
                Back to sign-in
              </button>
            </form>
          )}
        </Card>
      </main>
    );
  }

  if (mode === "done") {
    // Confirmation: safety net is the server-side session revocation.
    return (
      <main
        className="flex min-h-full items-center justify-center p-4 md:p-6"
        style={{ background: "var(--m-canvas)" }}
      >
        <Card className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7">
          <AuthBrand />
          <h1 className="mt-4 text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
            Password updated
          </h1>
          <p className="mt-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }} role="status">
            Every existing session was revoked as a safeguard.
          </p>
          <Button
            variant="primary"
            className="m-rise mt-4 w-full"
            onClick={() => router.replace("/login")}
          >
            Sign in
          </Button>
        </Card>
      </main>
    );
  }

  // --- CONFIRM MODE -------------------------------------------------
  return (
    <main
      className="flex min-h-full items-center justify-center p-4 md:p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7" data-testid="password-reset-confirm-card">
        <AuthBrand />
        <h1 className="mt-4 text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
          Set a new password
        </h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Pick a new password for your Meridian account.
        </p>
        <form onSubmit={confirmReset} className="m-rise mt-4 grid gap-3" style={{ animationDelay: "40ms" }}>
          <label className="grid gap-1">
            <Label htmlFor="reset-password">New password</Label>
            <div className="relative">
              <Input
                id="reset-password"
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
            <FieldHint>10+ characters</FieldHint>
          </label>
          <label className="grid gap-1">
            <Label htmlFor="reset-password-confirm">Confirm password</Label>
            <div className="relative">
              <Input
                id="reset-password-confirm"
                type={showConfirm ? "text" : "password"}
                autoComplete="new-password"
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                className="pr-12"
                required
              />
              <ToggleShowPassword on={showConfirm} onClick={() => setShowConfirm((v) => !v)} />
            </div>
          </label>
          {error !== null && (
            <p
              role="alert"
              className="text-[13px]"
              style={{ color: "var(--m-status-danger-fg)" }}
              data-testid="password-reset-confirm-error"
            >
              {error}
            </p>
          )}
          <Button type="submit" variant="primary" disabled={busy}>
            {busy ? <Spinner /> : null} Set password
          </Button>
        </form>
      </Card>
    </main>
  );
}

export default function PasswordResetPage(): ReactNode {
  return (
    <Suspense
      fallback={
        <main className="flex min-h-full items-center justify-center p-6" style={{ background: "var(--m-canvas)" }}>
          <Spinner />
        </main>
      }
    >
      <PasswordResetInner />
    </Suspense>
  );
}
