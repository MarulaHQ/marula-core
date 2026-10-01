// /password-reset — a fresh start. Indigo, because this screen protects something.
//
// Purpose: two halves. WITHOUT `?token` it asks for the address and sends a
// single-use link. WITH `?token` it chooses the new password, which is the
// pack's state 05: the serif headline "A fresh start", the two rules ticking
// off as they are met, and a mismatch as a BLUSH field with words.
//
// THE RULES TICK OFF AS THEY ARE MET, and they only report a failure once the
// field has been touched. An untouched form showing two red rules teaches
// nothing and looks like a rejection; a person needs to see which rule they
// have not met yet, which is what a tick list is for.
//
// MINIMUM LENGTH: this screen enforces the server's own floor of 10
// characters, and says 10, because that is the number the API answers with.
// The pack draws 12. Saying 12 here and having the server accept 10 would be
// a promise the product does not keep, so the copy matches the contract and the
// difference from the pack is recorded rather than papered over.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";
import { applyEntryTheme } from "../../lib/entry-theme";
import { friendlyMutationText } from "../../lib/error-copy";
import { api } from "../../lib/api";
import {
  EntryCard,
  EntryHeadline,
  EntryNote,
  EntryShell,
  PasswordRules,
} from "../../components/auth/entry";

/** The server's floor. See the header note: the copy matches the contract. */
const MIN_LENGTH = 10;

function PasswordInput({
  id,
  value,
  onChange,
  shown,
  onToggle,
  autoComplete,
  invalid,
}: {
  id: string;
  value: string;
  onChange: (next: string) => void;
  shown: boolean;
  onToggle: () => void;
  autoComplete: string;
  invalid: boolean;
}) {
  return (
    <div style={{ position: "relative" }}>
      <input
        id={id}
        className="entry-input"
        type={shown ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        autoComplete={autoComplete}
        aria-invalid={invalid}
        style={{ paddingRight: 48 }}
      />
      <button
        type="button"
        aria-label={shown ? "Hide password" : "Show password"}
        aria-pressed={shown}
        onClick={onToggle}
        style={{
          position: "absolute",
          top: 0,
          right: 0,
          display: "flex",
          width: 44,
          height: 48,
          alignItems: "center",
          justifyContent: "center",
          border: "none",
          background: "transparent",
          color: "var(--m-text-secondary)",
        }}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" aria-hidden>
          {shown ? (
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
    </div>
  );
}

function PasswordResetInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";
  const [mode, setMode] = useState<"idle" | "sent" | "done">("idle");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [showConfirm, setShowConfirm] = useState(false);
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mismatch, setMismatch] = useState(false);

  useEffect(() => {
    // Public page: always render in the light theme, never inherit a staff
    // member's persisted preference on a shared device.
    applyEntryTheme();
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
        // 429 auth_rate_limited and generic envelopes both land here.
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
    setTouched(true);
    setError(null);
    if (password.length < MIN_LENGTH) {
      setMismatch(false);
      setError(`Your password needs at least ${MIN_LENGTH} characters.`);
      return;
    }
    if (password !== confirm) {
      // Blush on the second field, with the words — never red.
      setMismatch(true);
      return;
    }
    setMismatch(false);
    setBusy(true);
    try {
      const { error: apiError } = await api.api.POST(
        "/api/v1/auth/password-reset/confirm",
        { body: { token, new_password: password } },
      );
      if (apiError !== undefined) {
        const body = apiError as { error?: { code?: number } } | undefined;
        if (body?.error?.code === 401) {
          setError("This reset link is invalid, used, or expired. Ask for a fresh one and try again.");
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
    return (
      <EntryShell background="indigo">
        <EntryCard testId="password-reset-card">
          {mode === "sent" ? (
            <>
              <div>
                <EntryHeadline>Check your email.</EntryHeadline>
                <p className="entry-sub">
                  If an account matches <b style={{ color: "var(--m-text)" }}>{email.trim()}</b>, a
                  reset link is on its way. It works once and lasts 30 minutes.
                </p>
              </div>
              <EntryNote>
                Asking again invalidates every earlier link, so only the newest one works.
              </EntryNote>
              <button
                type="button"
                className="entry-btn entry-btn--secondary entry-btn--full"
                onClick={() => router.replace("/login")}
              >
                Back to sign in
              </button>
            </>
          ) : (
            <>
              <div>
                <EntryHeadline>A fresh start.</EntryHeadline>
                <p className="entry-sub">
                  Enter the address you sign in with and we&apos;ll send a link to choose a new
                  password.
                </p>
              </div>
              <form onSubmit={requestReset} className="entry-field">
                <label className="entry-label" htmlFor="reset-email">
                  Work email
                </label>
                <input
                  id="reset-email"
                  className="entry-input"
                  type="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  placeholder="you@company.com"
                  autoComplete="email"
                  required
                />
              </form>
              {error !== null && (
                <p className="entry-error" role="alert" data-testid="password-reset-error">
                  {error}
                </p>
              )}
              <button type="submit" className="entry-btn" disabled={busy} data-testid="password-reset-send">
                {busy ? "Sending…" : "Email me a reset link"}
              </button>
            </>
          )}
        </EntryCard>
      </EntryShell>
    );
  }

  if (mode === "done") {
    return (
      <EntryShell background="indigo" fruit="none">
        <EntryCard testId="password-reset-card">
          <div className="entry-centred">
            <EntryHeadline>Password changed.</EntryHeadline>
            <p className="entry-sub">
              You&apos;re signed out everywhere else, and this browser is signed in. Take the new
              password to your password manager now.
            </p>
          </div>
          <button
            type="button"
            className="entry-btn"
            onClick={() => router.replace("/login")}
            data-testid="password-reset-continue"
          >
            Sign in
          </button>
        </EntryCard>
      </EntryShell>
    );
  }

  return (
    <EntryShell background="indigo">
      <EntryCard testId="password-reset-card">
        <div>
          <EntryHeadline>A fresh start.</EntryHeadline>
          <p className="entry-sub">
            Choose a new password. You&apos;ll be signed out everywhere else once you save it.
          </p>
        </div>

        <form onSubmit={confirmReset} style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <div className="entry-field">
            <label className="entry-label" htmlFor="reset-password">
              New password
            </label>
            <PasswordInput
              id="reset-password"
              value={password}
              onChange={(next) => {
                setPassword(next);
                setTouched(true);
              }}
              shown={showPassword}
              onToggle={() => setShowPassword((v) => !v)}
              autoComplete="new-password"
              invalid={error !== null}
            />
            <PasswordRules password={password} touched={touched} />
          </div>

          <div className={`entry-field${mismatch ? " entry-field--error" : ""}`}>
            <label className="entry-label" htmlFor="reset-confirm">
              Type it again
            </label>
            <PasswordInput
              id="reset-confirm"
              value={confirm}
              onChange={(next) => {
                setConfirm(next);
                if (mismatch) setMismatch(false);
              }}
              shown={showConfirm}
              onToggle={() => setShowConfirm((v) => !v)}
              autoComplete="new-password"
              invalid={mismatch}
            />
            {mismatch && (
              <p className="entry-error" role="alert" data-testid="password-mismatch">
                The two passwords don&apos;t match.
              </p>
            )}
          </div>

          {error !== null && (
            <p className="entry-error" role="alert" data-testid="password-reset-error">
              {error}
            </p>
          )}

          <button type="submit" className="entry-btn" disabled={busy} data-testid="password-reset-save">
            {busy ? "Saving…" : "Save and sign in"}
          </button>
        </form>
      </EntryCard>
    </EntryShell>
  );
}

export default function PasswordResetPage(): ReactNode {
  return (
    <Suspense fallback={<EntryShell background="indigo"><EntryCard /></EntryShell>}>
      <PasswordResetInner />
    </Suspense>
  );
}
