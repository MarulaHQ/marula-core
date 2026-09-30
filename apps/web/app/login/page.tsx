// /login — the door. Indigo, because this screen protects something.
//
// Purpose: email + password, the two other ways in (an emailed link and the
// company's own sign-in), and first-run org creation — which is why this
// screen also carries "Create org". The card carries the brand's serif
// headline and the pattern sits behind it, with the fruit peeking out.
//
// THE WRONG-PASSWORD STATE IS BLUSH AND SAYS WHAT HAPPENS NEXT. State 02 in
// the pack: a 2px blush border on the password field and a line with an alert
// icon reading "That email and password don't match. You have 2 tries left
// before a short wait." That is not red and not a bare "invalid": it names the
// problem and the consequence, because a person signing in needs to know
// whether to try again or stop.
//
// Non-purpose: no enrollment, no MFA management (see /settings/security), and
// no "forgot your username" — the address is the identity.
"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { FieldError, FieldHint } from "@marula-core/ui";
import { friendlyMutationText } from "../../lib/error-copy";
import { Button, Input, Label, Spinner } from "@marula-core/ui";
import { api, resolveApiBaseUrl, saveTokens, useAuth } from "../../lib/api";
import { broadcastSession } from "../../lib/popout/window-bus";
import { portalOrgName } from "../../lib/portal-branding";
import {
  EntryCard,
  EntryHeadline,
  EntryIcon,
  EntryShell,
} from "../../components/auth/entry";

const API_BASE = resolveApiBaseUrl();

interface PublicProvider {
  name: string;
  display_name: string;
}

type Mode = "signin" | "register";

/** The envelope's own "how many tries are left", when the server sends one. */
function triesLeft(err: unknown): number | null {
  const body = (err as { body?: unknown } | null)?.body;
  if (typeof body !== "object" || body === null) return null;
  const record = body as Record<string, unknown>;
  for (const key of ["attempts_remaining", "tries_remaining", "remaining_attempts"]) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return null;
}

export default function LoginPage() {
  const router = useRouter();
  const { status } = useAuth();
  const [mode, setMode] = useState<Mode>("signin");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tries, setTries] = useState<number | null>(null);

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [orgName, setOrgName] = useState("");
  const [orgSlug, setOrgSlug] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [providers, setProviders] = useState<PublicProvider[]>([]);
  /** Server-suggested available slug behind the org_slug_taken 409. */
  const [slugSuggestion, setSlugSuggestion] = useState<string | null>(null);

  // Config-driven SSO: only providers an operator enabled appear here.
  useEffect(() => {
    let cancelled = false;
    fetch(`${API_BASE}/api/v1/auth/oidc/providers/public`)
      .then((r) => (r.ok ? r.json() : []))
      .then((list: PublicProvider[]) => {
        if (!cancelled) setProviders(list);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (status === "authed") router.replace("/today");
  }, [status, router]);

  async function attempt(slugOverride?: string) {
    setBusy(true);
    setError(null);
    setTries(null);
    setSlugSuggestion(null);
    try {
      if (mode === "signin") {
        const result = await api.login({ email, password });
        if ("mfa_required" in result) {
          window.sessionStorage.setItem(
            "marula-mfa-pending",
            JSON.stringify({ mfa_token: result.mfa_token, methods: result.methods }),
          );
          window.location.assign("/mfa");
          return;
        }
        saveTokens(result);
        // Wave 38 session honesty: restore every sibling window too.
        broadcastSession("restored");
      } else {
        saveTokens(
          await api.register({
            org_name: orgName,
            org_slug: slugOverride ?? orgSlug,
            email,
            password,
            display_name: displayName,
          }),
        );
      }
      // Full navigation so AuthProvider re-bootstraps from the fresh tokens.
      window.location.assign("/today");
    } catch (err) {
      const envelope = (err as { body?: { error?: { suggestion?: unknown } } } | null)?.body;
      const suggestion = envelope?.error?.suggestion;
      if (typeof suggestion === "string" && suggestion !== "") {
        setSlugSuggestion(suggestion);
      }
      const left = triesLeft(err);
      setTries(left);
      setError(
        friendlyMutationText(
          err,
          left === null
            ? "That email and password don't match."
            : `That email and password don't match. You have ${left} ${left === 1 ? "try" : "tries"} left before a short wait.`,
        ),
      );
    } finally {
      setBusy(false);
    }
  }

  function acceptSuggestion() {
    if (slugSuggestion === null) return;
    setOrgSlug(slugSuggestion);
    setSlugSuggestion(null);
    setError(null);
    void attempt(slugSuggestion);
  }

  function ssoHref(provider: string): string {
    return `${API_BASE}/api/v1/auth/oidc/${provider}/login`;
  }

  const signInError = mode === "signin" ? error : null;

  return (
    <EntryShell background="indigo">
      <EntryCard>
        <div>
          <EntryHeadline>{mode === "signin" ? "Good to see you." : "Let's set this up."}</EntryHeadline>
          <p className="entry-sub">
            {mode === "signin"
              ? `Sign in to ${portalOrgName() ?? "your service desk"} and pick up where you left off.`
              : "Name your organization and you can invite the rest of your team next."}
          </p>
        </div>

        <form key={mode} onSubmit={(e) => { e.preventDefault(); void attempt(); }} style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          {mode === "register" && (
            <>
              <div className="entry-field">
                <Label htmlFor="org-name">Organization name</Label>
                <Input
                  id="org-name"
                  value={orgName}
                  onChange={(e) => setOrgName(e.target.value)}
                  placeholder="Acme IT"
                  required
                />
              </div>
              <div className="entry-field">
                <Label htmlFor="org-slug">Org slug</Label>
                <Input
                  id="org-slug"
                  value={orgSlug}
                  onChange={(e) => setOrgSlug(e.target.value)}
                  placeholder="acme-it"
                  pattern="[a-z0-9\-]+"
                  required
                />
                <FieldHint>lowercase letters, digits, dashes</FieldHint>
              </div>
              <div className="entry-field">
                <Label htmlFor="org-user">Your name</Label>
                <Input
                  id="org-user"
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder="Dana Owner"
                  required
                />
              </div>
            </>
          )}

          <div className="entry-field">
            <Label htmlFor="login-email">Work email</Label>
            <Input
              id="login-email"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@company.com"
              autoComplete="email"
              required
            />
          </div>

          <div className={`entry-field${signInError !== null ? " entry-field--error" : ""}`}>
            <div className="entry-label">
              <Label htmlFor="login-password">Password</Label>
              <button
                type="button"
                className="entry-link"
                style={{ fontWeight: 500 }}
                onClick={() => router.push("/password-reset")}
              >
                Forgot password?
              </button>
            </div>
            <div style={{ position: "relative" }}>
              <Input
                id="login-password"
                type={showPassword ? "text" : "password"}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete={mode === "signin" ? "current-password" : "new-password"}
                required
                aria-invalid={signInError !== null}
                aria-describedby={signInError !== null ? "login-password-error" : undefined}
                style={{ paddingRight: 48 }}
              />
              <button
                type="button"
                aria-label={showPassword ? "Hide password" : "Show password"}
                aria-pressed={showPassword}
                onClick={() => setShowPassword((v) => !v)}
                style={{
                  position: "absolute",
                  top: 0,
                  right: 0,
                  display: "flex",
                  height: 48,
                  width: 44,
                  alignItems: "center",
                  justifyContent: "center",
                  border: "none",
                  background: "transparent",
                  color: "var(--m-text-secondary)",
                }}
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" aria-hidden>
                  {showPassword ? (
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
            {/* Blush, with an icon and words. Never red. */}
            {signInError !== null && (
              <p className="entry-error" role="alert" id="login-password-error" data-testid="login-error">
                <EntryIcon name="alert" size={14} />
                <span>
                  {signInError}
                  {tries !== null && tries > 0
                    ? ` You have ${tries} ${tries === 1 ? "try" : "tries"} left before a short wait.`
                    : ""}
                </span>
              </p>
            )}
            {mode === "register" && signInError === null && <FieldHint>12 or more characters</FieldHint>}
          </div>

          {error !== null && mode === "register" && (
            <FieldError data-testid="register-error">{error}</FieldError>
          )}

          {slugSuggestion !== null && (
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                borderRadius: 10,
                background: "var(--m-accent-soft)",
                padding: "8px 10px",
                fontSize: 13,
              }}
              role="status"
            >
              <span style={{ color: "var(--m-text-secondary)", flex: 1 }}>
                Available: <strong>{slugSuggestion}</strong>
              </span>
              <Button type="button" variant="secondary" size="sm" onClick={acceptSuggestion}>
                Use {slugSuggestion}
              </Button>
            </div>
          )}

          <button type="submit" className="entry-btn" disabled={busy} data-testid="login-submit">
            {busy && <Spinner />}
            {busy
              ? "Working…"
              : mode === "signin"
                ? "Sign in"
                : "Create organization"}
          </button>
        </form>

        {mode === "signin" && (
          <>
            <p className="entry-or">or</p>
            <button
              type="button"
              className="entry-btn entry-btn--secondary entry-btn--full"
              onClick={() => router.push("/magic-link")}
              data-testid="login-magic-link"
            >
              <EntryIcon name="mail" />
              Email me a sign-in link
            </button>
            {providers.length > 0 && (
              <a
                href={ssoHref(providers[0]?.name ?? "")}
                style={{ display: "block", textDecoration: "none" }}
                data-testid="login-sso"
              >
                <button type="button" className="entry-btn entry-btn--secondary entry-btn--full">
                  <EntryIcon name="shield" />
                  Sign in with your company account
                </button>
              </a>
            )}
            {/* The pack closes the card with "Need an account? Ask your admin
                for an invite." This product also has to let a FIRST
                organization exist, and `POST /api/v1/auth/register` is the only
                way in — so the invite route stays the sentence it is, and
                organization creation sits one quiet link below it rather than
                as a peer tab on the sign-in card. */}
            <p className="entry-hint" style={{ textAlign: "center", margin: 0 }}>
              Need an account? Ask your admin for an invite.
            </p>
            {mode === "signin" ? (
              <p className="entry-hint" style={{ textAlign: "center", margin: 0 }}>
                <button
                  type="button"
                  className="entry-link"
                  style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                  onClick={() => {
                    setMode("register");
                    setError(null);
                    setTries(null);
                  }}
                  data-testid="login-create-org"
                >
                  Setting up the first organization?
                </button>
              </p>
            ) : (
              <p className="entry-hint" style={{ textAlign: "center", margin: 0 }}>
                <button
                  type="button"
                  className="entry-link"
                  style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                  onClick={() => {
                    setMode("signin");
                    setError(null);
                    setTries(null);
                  }}
                  data-testid="login-back-to-signin"
                >
                  Back to sign in
                </button>
              </p>
            )}
          </>
        )}
      </EntryCard>
    </EntryShell>
  );
}
