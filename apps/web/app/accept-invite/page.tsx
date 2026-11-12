// /accept-invite — come on in. Rust, because this moment welcomes.
//
// Purpose: the pack's state 08. Somebody has been invited; this is where the
// account becomes real. The page is a short guided setup, three steps in one
// card, with a small meter above each step:
//
//   1. Identity — your name (optional: a blank leaves whatever the inviter
//      recorded untouched), and the invited address, named in words because
//      the token does not carry it.
//   2. Password — chosen and confirmed, with the rules ticking off as they
//      are met. This step holds the one API call; every error path lives here.
//   3. You're in — timezone confirmation (saved through the existing
//      PATCH /api/v1/users/me), an optional desktop-app card on plain
//      browsers, and the path forward.
//
// After the account exists, first run starts at /onboarding — where every
// step is skippable — and lands on /today once the steps are resolved or
// skipped, where the per-surface tours greet a first-time visitor.
//
// ONE DOCUMENTED DIVERGENCE. The pack draws two extra fields: "Your full name"
// and a disabled "Email" with the note "Set by the invitation". The pinned
// `AcceptInviteRequest` carries exactly `token`, `password`, an optional
// `timezone` and an optional `display_name`. The name box here records the
// name with the account; the address the invitation was sent to is named in
// words instead, which is the fact the pack's disabled field was carrying.
//
// TWO HONEST GAPS, both recorded rather than papered over:
//  - The `timezone` field of `AcceptInviteRequest` is deserialized but left
//    alone server-side today (the accept handler's update_profile call passes
//    None), so the confirmed timezone is saved through PATCH /api/v1/users/me
//    in step 3 — an existing endpoint the first-run wizard already uses. The
//    field is still sent at acceptance so a server that starts honoring it
//    covers the case of somebody who closes the tab before step 3.
//  - The accept response returns the token pair WITHOUT a session cookie (the
//    login handler sets one; this one does not), so the full navigation at the
//    end re-bootstraps anonymously and /onboarding hands off to /login. The
//    person signs in once more with the password they just chose. Fixing it
//    needs a Set-Cookie on the accept response, which is server work outside
//    this page's reach.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useRef, useState, type ReactNode } from "react";
import { Spinner } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import { applyEntryTheme } from "../../lib/entry-theme";
import { friendlyMutationText } from "../../lib/error-copy";
import { api, saveTokens } from "../../lib/api";
import { isDesktopShell } from "../../lib/desktop/shell";
import {
  EntryCard,
  EntryHeadline,
  EntryNote,
  EntryShell,
  PasswordRules,
} from "../../components/auth/entry";

const MIN_LENGTH = 12;
const TOTAL_STEPS = 3;

/** Where the desktop-app card remembers it was dismissed. */
const DESKTOP_CARD_DISMISSED_KEY = "marula-accept-invite-desktop-dismissed";

// TODO(downloads): the app has no downloads page of its own yet, and the
// release pipeline publishes installers as GitHub Releases on desktop-v* tags
// (docs/desktop-release.md §7). Until a real downloads path exists, this
// points at the docs page that tells a person which installer they want and
// where the download lives. Swap it for the real path when one exists.
const DESKTOP_DOWNLOAD_URL =
  "https://getmarula.com/docs/psa/v0.3/how-to/install-the-desktop-app";

type Step = "identity" | "password" | "welcome";

/** The browser's timezone, or UTC when the environment won't say. */
function detectTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone ?? "UTC";
  } catch {
    return "UTC";
  }
}

/** The download label names the platform the person is on. */
function platformDownloadLabel(): string {
  if (typeof navigator === "undefined") return "Download the desktop app";
  const ua = navigator.userAgent;
  if (/Mac/i.test(ua)) return "Download for macOS";
  if (/Win/i.test(ua)) return "Download for Windows";
  if (/Linux/i.test(ua)) return "Download for Linux";
  return "Download the desktop app";
}

/** The compact meter above each step: a line of text and a thin bar. */
function StepMeter({ step, label }: { step: number; label: string }) {
  return (
    <div>
      <p className="entry-kicker" style={{ margin: 0 }}>
        {label}
      </p>
      <div
        className="entry-progress entry-progress--card"
        role="progressbar"
        aria-label="Setup progress"
        aria-valuemin={1}
        aria-valuemax={TOTAL_STEPS}
        aria-valuenow={step}
      >
        <i style={{ width: `${Math.round((step / TOTAL_STEPS) * 100)}%` }} />
      </div>
    </div>
  );
}

/** The desktop-app card: only on plain browsers, dismissible for good. */
function DesktopCard({ onDismiss }: { onDismiss: () => void }) {
  return (
    <div className="entry-appcard" data-testid="accept-invite-desktop-card">
      <div className="entry-appcard__hd">
        <Icon name="monitor" size={16} />
        <span>Marula on your desktop</span>
      </div>
      <p className="entry-hint">
        The desktop app keeps Marula in its own window, with the menu bar, the
        tray and printing. You can add it now or any time later.
      </p>
      <div style={{ display: "flex", gap: 10 }}>
        <a
          className="entry-btn entry-btn--small"
          href={DESKTOP_DOWNLOAD_URL}
          target="_blank"
          rel="noreferrer"
          data-testid="accept-invite-desktop-download"
        >
          <Icon name="download" size={16} />
          {platformDownloadLabel()}
        </a>
      </div>
      <button
        type="button"
        className="entry-appcard__x"
        aria-label="Dismiss. Don't show the desktop app card again."
        onClick={onDismiss}
        data-testid="accept-invite-desktop-dismiss"
      >
        <Icon name="x" size={14} />
      </button>
    </div>
  );
}

function AcceptInviteInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";

  const [step, setStep] = useState<Step>("identity");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [showConfirm, setShowConfirm] = useState(false);
  const [touched, setTouched] = useState(false);
  const [mismatch, setMismatch] = useState(false);
  /** Client-side length rule, tied to the password field. */
  const [passwordError, setPasswordError] = useState<string | null>(null);
  /** API and network failures, tied to the step as a whole. */
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // The welcome step. `tz` is null until the account exists.
  const [tz, setTz] = useState<string | null>(null);
  const [tzError, setTzError] = useState<string | null>(null);

  // The desktop card is a browser-only offer, so it mounts after hydration
  // (the server render never decides it) and only when it hasn't been
  // dismissed for good.
  const [desktopCard, setDesktopCard] = useState(false);
  useEffect(() => {
    // Public page: always render in the light theme, never inherit a staff
    // member's persisted preference on a shared device.
    applyEntryTheme();
  }, []);
  useEffect(() => {
    if (isDesktopShell()) return;
    let dismissed = false;
    try {
      dismissed = window.localStorage.getItem(DESKTOP_CARD_DISMISSED_KEY) === "1";
    } catch {
      // storage unavailable — the card simply comes back next visit
    }
    if (!dismissed) setDesktopCard(true);
  }, []);

  const headingRef = useRef<HTMLHeadingElement | null>(null);
  useEffect(() => {
    // Each step announces itself where a page turn would.
    headingRef.current?.focus();
  }, [step]);

  const dismissDesktopCard = () => {
    setDesktopCard(false);
    try {
      window.localStorage.setItem(DESKTOP_CARD_DISMISSED_KEY, "1");
    } catch {
      // storage unavailable — dismissal lasts this visit only
    }
  };

  const redeem = async (e: React.FormEvent) => {
    e.preventDefault();
    setTouched(true);
    setError(null);
    setPasswordError(null);
    if (password.length < MIN_LENGTH) {
      setMismatch(false);
      setPasswordError(`Your password needs at least ${MIN_LENGTH} characters.`);
      return;
    }
    if (password !== confirm) {
      setMismatch(true);
      return;
    }
    setMismatch(false);
    setBusy(true);
    const detected = detectTimezone();
    try {
      const { data, error: apiError } = await api.api.POST("/api/v1/org/users/accept-invite", {
        // A blank name is "no opinion" and leaves whatever the inviter recorded
        // untouched, which is why the field is optional rather than required.
        body: {
          token,
          password,
          // Sent for the record; the server leaves it alone today (see the
          // header note), so step 3 saves the confirmed value for real.
          timezone: detected,
          ...(name.trim() === "" ? {} : { display_name: name.trim() }),
        },
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
      // Persist exactly like a login does, and put the pair on the client
      // itself so step 3's profile PATCH is authenticated in this page's
      // session. The tokens never touch document storage.
      saveTokens(data as components["schemas"]["TokenPair"]);
      api.setTokens(data as components["schemas"]["TokenPair"]);
      // The account exists; the password has done its job.
      setPassword("");
      setConfirm("");
      setTouched(false);
      setTz(detected);
      setStep("welcome");
    } catch {
      setError("You're offline — check your connection and try again. Nothing was changed.");
    } finally {
      setBusy(false);
    }
  };

  /** Step 3's forward path: save the confirmed timezone, then leave for
   *  first run. A failure keeps the person here with the reason, and the
   *  secondary path continues without it — the setup wizard asks for the
   *  timezone again on its profile step, so nothing is lost. */
  const continueToSetup = async () => {
    if (tz === null) return;
    setTzError(null);
    if (tz.trim() === "") {
      setTzError("Type a timezone, like Europe/Athens, or continue without saving.");
      return;
    }
    setBusy(true);
    try {
      const { error: apiError } = await api.api.PATCH("/api/v1/users/me", {
        body: { timezone: tz.trim() },
      });
      if (apiError !== undefined) {
        setTzError(
          friendlyMutationText(
            apiError,
            "We couldn't save your timezone. You can set it during setup instead.",
          ),
        );
        setBusy(false);
        return;
      }
      window.location.assign("/onboarding");
    } catch {
      setTzError("You're offline — check your connection and try again.");
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
    describedBy?: string,
    invalid?: boolean,
  ) => (
    <div className={`entry-field${invalid ? " entry-field--error" : ""}`}>
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
          aria-describedby={describedBy}
          aria-invalid={invalid ? true : undefined}
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
          <Icon name="eye" size={16} />
        </button>
      </div>
    </div>
  );

  if (step === "welcome") {
    return (
      <EntryShell background="rust" fruit="none">
        <EntryCard testId="accept-invite-card">
          <div className="entry-mstep">
            <StepMeter step={TOTAL_STEPS} label={`Step ${TOTAL_STEPS} of ${TOTAL_STEPS}`} />
            <h2
              className="entry-mhead"
              ref={headingRef}
              tabIndex={-1}
              data-testid="accept-invite-welcome-heading"
            >
              You&apos;re in.
            </h2>
            <p className="entry-sub">
              Your account is live. One last thing, then we&apos;ll take you into setup.
            </p>
          </div>

          <div className="entry-field">
            <label className="entry-label" htmlFor="invite-tz">
              Your timezone
            </label>
            <input
              id="invite-tz"
              className="entry-input"
              value={tz ?? ""}
              onChange={(e) => {
                setTz(e.target.value);
                if (tzError !== null) setTzError(null);
              }}
              placeholder="Europe/Athens"
              data-testid="accept-invite-tz"
              aria-describedby={tzError !== null ? "invite-tz-error" : undefined}
              aria-invalid={tzError !== null ? true : undefined}
            />
            <p className="entry-hint" style={{ margin: 0 }}>
              This is the clock your times are shown in, and what your teammates see. It was
              filled in from this device.
            </p>
            {tzError !== null && (
              <p className="entry-error" role="alert" id="invite-tz-error" data-testid="accept-invite-tz-error">
                {tzError}
              </p>
            )}
          </div>

          {desktopCard && <DesktopCard onDismiss={dismissDesktopCard} />}

          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <button
              type="button"
              className="entry-btn"
              onClick={() => void continueToSetup()}
              disabled={busy}
              data-testid="accept-invite-continue"
            >
              {busy ? (
                <>
                  <Spinner /> Saving…
                </>
              ) : (
                "Continue to setup"
              )}
            </button>
            {tzError !== null && (
              <button
                type="button"
                className="entry-btn entry-btn--secondary entry-btn--small"
                onClick={() => window.location.assign("/onboarding")}
                data-testid="accept-invite-continue-without"
              >
                Continue without saving
              </button>
            )}
          </div>

          <p className="entry-hint" style={{ textAlign: "center", margin: 0 }}>
            Next you&apos;ll set up a second step, a quick way to keep your account safe.
          </p>
        </EntryCard>
      </EntryShell>
    );
  }

  return (
    <EntryShell background="rust">
      <EntryCard testId="accept-invite-card">
        <div className="entry-mstep">
          <StepMeter
            step={step === "identity" ? 1 : 2}
            label={`Step ${step === "identity" ? 1 : 2} of ${TOTAL_STEPS}`}
          />
          {step === "identity" ? (
            <>
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
              <h1
                className="entry-headline"
                ref={headingRef}
                tabIndex={-1}
                data-testid="accept-invite-heading"
              >
                Come on in.
              </h1>
              <p className="entry-sub">
                {/* The pack's subtitle names WHO invited you and WHERE you are going
                    ("Alex M. invited you to join Brightline IT"). Neither is on the
                    invite token this page receives, so it says what it can prove:
                    that an administrator did it, and that this is an account. Naming
                    a person or an org the response does not carry would be
                    inventing them. (auth/neutral/states/08-accept-invite.png) */}
                Your administrator invited you to join their service desk. Two quick steps and
                your account is ready.
              </p>
            </>
          ) : (
            <>
              <h2 className="entry-mhead" ref={headingRef} tabIndex={-1} data-testid="accept-invite-heading">
                Choose your password
              </h2>
              <p className="entry-sub">
                You&apos;ll sign in with this and the address you were invited on.
              </p>
            </>
          )}
        </div>

        {step === "identity" ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {/*
              The pack asks for a full name here, above the password, because the
              INVITATION carries an address rather than a person and this is the
              first moment the invitee can name themselves. The value is recorded
              with the account at the last step; a blank name leaves whatever the
              inviter recorded untouched.
            */}
            <div className="entry-field">
              <label className="entry-label" htmlFor="invite-name">
                Your full name
              </label>
              <input
                id="invite-name"
                className="entry-input"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Daniel Reid"
                autoComplete="name"
                data-testid="accept-invite-name"
              />
            </div>

            <div className="entry-field">
              <span className="entry-label">Email</span>
              {/*
                The pack shows a DISABLED input holding the invited address, with
                "Set by the invitation." beneath it. The invite token does not
                carry that address, so there is nothing truthful to put in the box,
                and a disabled input with no value is a field pretending to be data.
                The fact is kept as a sentence instead; carrying the address on the
                token would close this properly, and is recorded as a gap.
              */}
              <p
                className="entry-hint"
                style={{ margin: 0 }}
                data-testid="accept-invite-email"
              >
                Set by the invitation. Sign in with the address you were invited on.
              </p>
            </div>

            <div style={{ display: "flex", justifyContent: "flex-end" }}>
              <button
                type="button"
                className="entry-btn"
                onClick={() => setStep("password")}
                data-testid="accept-invite-next"
              >
                Continue
              </button>
            </div>
          </div>
        ) : (
          <form
            onSubmit={redeem}
            style={{ display: "flex", flexDirection: "column", gap: 16 }}
          >
            {field(
              "invite-password",
              password,
              (next) => {
                setPassword(next);
                setTouched(true);
                if (passwordError !== null) setPasswordError(null);
              },
              showPassword,
              () => setShowPassword((v) => !v),
              "Password",
              passwordError !== null ? "invite-password-error" : undefined,
              passwordError !== null,
            )}

            {passwordError !== null && (
              <p
                className="entry-error"
                role="alert"
                id="invite-password-error"
                data-testid="accept-invite-password-error"
                style={{ marginTop: -6 }}
              >
                {passwordError}
              </p>
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
              mismatch ? "invite-confirm-error" : undefined,
              mismatch,
            )}

            {mismatch && (
              <p
                className="entry-error"
                role="alert"
                id="invite-confirm-error"
                data-testid="accept-invite-mismatch"
                style={{ marginTop: -6 }}
              >
                The two passwords don&apos;t match.
              </p>
            )}

            {error !== null && (
              <p className="entry-error" role="alert" data-testid="accept-invite-error">
                {error}
              </p>
            )}

            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              {/* Back is navigation, not a decision: nothing has been sent yet. */}
              <button
                type="button"
                className="entry-btn entry-btn--secondary entry-btn--small"
                onClick={() => setStep("identity")}
                disabled={busy}
                data-testid="accept-invite-back"
              >
                Back
              </button>
              <span style={{ flex: 1 }} />
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
            </div>
          </form>
        )}

        {step === "identity" && (
          <p className="entry-hint" style={{ textAlign: "center", margin: 0 }}>
            After this you&apos;ll choose a password, and then a quick way to keep your account
            safe.
          </p>
        )}
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
