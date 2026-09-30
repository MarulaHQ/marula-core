// /accept-invite — come on in. Rust, because this moment welcomes.
//
// Purpose: the pack's state 08. Somebody has been invited; this is where the
// account becomes real. The password is chosen, the rules tick off as they are
// met, and the line at the bottom says what happens next — a second step comes
// — so nobody finishes here wondering what is still to come.
//
// ONE DOCUMENTED DIVERGENCE. The pack draws two extra fields: "Your full name"
// and a disabled "Email" with the note "Set by the invitation". The pinned
// `AcceptInviteRequest` carries exactly three fields: `token`, `password` and an
// optional `timezone`. The name and the address were set by whoever sent the
// invitation, server-side, and this screen has nothing to send — so a name
// box here would collect an answer the server never receives, and a disabled
// address box would be a field pretending to be data. Neither is drawn. The
// address that the invitation was sent to is named in words instead, which is
// the fact the pack's disabled field was carrying.
//
// After the account exists, first run starts at /onboarding — where every step
// is skippable — and lands on /today once the steps are resolved or skipped.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";
import { applyTheme, Spinner } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { friendlyMutationText } from "../../lib/error-copy";
import { api, saveTokens } from "../../lib/api";
import {
  EntryCard,
  EntryHeadline,
  EntryNote,
  EntryShell,
  PasswordRules,
} from "../../components/auth/entry";

const MIN_LENGTH = 10;

function AcceptInviteInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";

  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [showConfirm, setShowConfirm] = useState(false);
  const [touched, setTouched] = useState(false);
  const [mismatch, setMismatch] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Public page: always render in the light theme, never inherit a staff
    // member's persisted preference on a shared device.
    applyTheme("light");
  }, []);

  const redeem = async (e: React.FormEvent) => {
    e.preventDefault();
    setTouched(true);
    setError(null);
    if (password.length < MIN_LENGTH) {
      setMismatch(false);
      setError(`Your password needs at least ${MIN_LENGTH} characters.`);
      return;
    }
    if (password !== confirm) {
      setMismatch(true);
      return;
    }
    setMismatch(false);
    setBusy(true);
    try {
      const { data, error: apiError } = await api.api.POST("/api/v1/org/users/accept-invite", {
        body: { token, password },
      });
      if (apiError !== undefined || data === undefined) {
        const body = apiError as { error?: { code?: number; message?: string } } | undefined;
        if (body?.error?.code === 404) {
          setError(
            "This invite link is not valid — it may already have been used, or it has expired. Ask your administrator for a fresh one.",
          );
          return;
        }
        setError(friendlyMutationText(apiError, "The invite could not be redeemed. Nothing was changed."));
        return;
      }
      // Persist exactly like a login, then a full navigation so AuthProvider
      // re-bootstraps from the fresh tokens.
      saveTokens(data as components["schemas"]["TokenPair"]);
      // First run starts here: profile, a second step, an orientation and
      // defaults, every step skippable.
      window.location.assign("/onboarding");
    } catch {
      setError("You're offline — check your connection and try again. Nothing was changed.");
    } finally {
      setBusy(false);
    }
  };

  if (token === "") {
    return (
      <EntryShell background="rust" fruit="none">
        <EntryCard testId="accept-invite-card">
          <div>
            <EntryHeadline>Come on in.</EntryHeadline>
            <p className="entry-sub">
              This page needs the setup link your administrator sent you. It looks like{" "}
              <code style={{ fontFamily: "var(--m-font-mono)", fontSize: 13 }}>
                …/accept-invite?token=…
              </code>
              .
            </p>
          </div>
          <EntryNote>
            Nothing is wrong with your account &mdash; the link just didn&apos;t arrive. Ask for a
            fresh one and it will work.
          </EntryNote>
          <button
            type="button"
            className="entry-btn entry-btn--secondary entry-btn--full"
            onClick={() => router.replace("/login")}
          >
            Back to sign in
          </button>
        </EntryCard>
      </EntryShell>
    );
  }

  const field = (
    id: string,
    value: string,
    setValue: (next: string) => void,
    shown: boolean,
    toggle: () => void,
    label: string,
  ) => (
    <div className="entry-field">
      <label className="entry-label" htmlFor={id}>
        {label}
      </label>
      <div style={{ position: "relative" }}>
        <input
          id={id}
          className="entry-input"
          type={shown ? "text" : "password"}
          autoComplete="new-password"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          style={{ paddingRight: 48 }}
          required
        />
        <button
          type="button"
          aria-label={shown ? "Hide password" : "Show password"}
          aria-pressed={shown}
          onClick={toggle}
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
    </div>
  );

  return (
    <EntryShell background="rust">
      <EntryCard testId="accept-invite-card">
        <div>
          <span
            style={{
              display: "inline-flex",
              alignItems: "center",
              borderRadius: 999,
              background: "var(--m-surface-2)",
              padding: "4px 12px",
              fontSize: 13,
              fontWeight: 600,
              marginBottom: 10,
            }}
          >
            Technician
          </span>
          <EntryHeadline>Come on in.</EntryHeadline>
          <p className="entry-sub">
            Your administrator invited you to join their service desk. Choose a password and
            you&apos;re in.
          </p>
        </div>

        <form onSubmit={redeem} style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          {field(
            "invite-password",
            password,
            (next) => {
              setPassword(next);
              setTouched(true);
            },
            showPassword,
            () => setShowPassword((v) => !v),
            "Choose a password",
          )}

          <div style={{ marginTop: -8 }}>
            <PasswordRules password={password} touched={touched} />
          </div>

          {field(
            "invite-password-confirm",
            confirm,
            (next) => {
              setConfirm(next);
              if (mismatch) setMismatch(false);
            },
            showConfirm,
            () => setShowConfirm((v) => !v),
            "Type it again",
          )}

          {mismatch && (
            <p
              className="entry-error"
              role="alert"
              data-testid="accept-invite-mismatch"
              style={{ marginTop: -6 }}
            >
              The two passwords don&apos;t match. Retype the second one.
            </p>
          )}

          {error !== null && (
            <p className="entry-error" role="alert" data-testid="accept-invite-error">
              {error}
            </p>
          )}

          <button
            type="submit"
            className="entry-btn"
            disabled={busy}
            data-testid="accept-invite-submit"
          >
            {busy ? (
              <>
                <Spinner /> Setting up…
              </>
            ) : (
              "Create account and join"
            )}
          </button>
        </form>

        <p className="entry-hint" style={{ textAlign: "center", margin: 0 }}>
          Next you&apos;ll set up a second step &mdash; a quick way to keep your account safe. Every
          step after this one can be skipped, and you can come back to it.
        </p>
      </EntryCard>
    </EntryShell>
  );
}

export default function AcceptInvitePage(): ReactNode {
  return (
    <Suspense fallback={<EntryShell background="rust"><EntryCard /></EntryShell>}>
      <AcceptInviteInner />
    </Suspense>
  );
}
