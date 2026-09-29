// /login — email+password sign-in, MFA hand-off, SSO entry, and first-run
// org registration.
//
// Purpose: segmented form (Sign in / Create org) against the public auth
// endpoints via @marula-core/sdk; routes MFA-gated logins to /mfa; renders
// config-driven SSO buttons from the enabled OIDC providers.
// Non-purpose: no enrollment/management UI here (see /settings/security).
//
// Wave 17 lane 9 UX pass: Tangerine brand header (§7 mark), morphing
// segmented control, show/hide password affordance, icon'd error row,
// staggered entrance motion (transform/opacity ≤120ms, reduced-motion
// collapses). Security copy and all wired APIs unchanged.
"use client";

import { FieldError, FieldHint } from "@marula-core/ui";
import { friendlyMutationText } from "../../lib/error-copy";
import { Button, Card, Input, Label, Spinner } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { AuthBrand } from "../../components/auth/auth-brand";
import { api, resolveApiBaseUrl, saveTokens, useAuth } from "../../lib/api";
import { broadcastSession } from "../../lib/popout/window-bus";

const API_BASE = resolveApiBaseUrl();

interface PublicProvider {
  name: string;
  display_name: string;
}

type Mode = "signin" | "register";

const MODES: readonly { id: Mode; label: string }[] = [
  { id: "signin", label: "Sign in" },
  { id: "register", label: "Create org" },
];

export default function LoginPage() {
  const router = useRouter();
  const { status } = useAuth();
  const [mode, setMode] = useState<Mode>("signin");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  if (status === "authed") {
    router.replace("/today");
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    await attempt();
  }

  async function attempt(slugOverride?: string) {
    setBusy(true);
    setError(null);
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
      setError(friendlyMutationText(err));
    } finally {
      setBusy(false);
    }
  }

  /** One-click accept of the org_slug_taken suggestion: swap the slug in
   * and resubmit immediately (Wave 13 lane 1 follow-up). */
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

  return (
    <main
      className="flex min-h-screen items-center justify-center p-4 md:p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card
        className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7"
        data-testid="login-card"
      >
        <AuthBrand subtitle="Sign in to your service desk, or create your organization." />
        <div
          className="relative mt-5 grid grid-cols-2 gap-1 rounded-xl p-1 transition-colors duration-100"
          style={{ background: "var(--m-status-neutral-bg)" }}
          role="tablist"
        >
          {MODES.map((m) => (
            <button
              key={m.id}
              role="tab"
              aria-selected={mode === m.id}
              onClick={() => setMode(m.id)}
              className="rounded-lg px-3 py-1.5 text-xs font-medium transition-[background-color,opacity] duration-100"
              style={
                mode === m.id
                  ? {
                      background: "var(--m-surface)",
                      color: "var(--m-text)",
                      boxShadow:
                        "0 1px 2px rgba(0, 0, 0, 0.1), inset 0 0 0 1px var(--m-border)",
                    }
                  : { color: "var(--m-text-secondary)" }
              }
            >
              {m.label}
            </button>
          ))}
        </div>
        <form key={mode} onSubmit={submit} className="m-rise mt-5 grid gap-3">
          {mode === "register" && (
            <>
              <label className="grid gap-1">
                <Label>Organization name</Label>
                <Input value={orgName} onChange={(e) => setOrgName(e.target.value)} placeholder="Acme IT" required />
              </label>
              <label className="grid gap-1">
                <Label>Org slug</Label>
                <Input
                  value={orgSlug}
                  onChange={(e) => setOrgSlug(e.target.value)}
                  placeholder="acme-it"
                  pattern="[a-z0-9\-]+"
                  required
                />
                <FieldHint>lowercase letters, digits, dashes</FieldHint>
              </label>
              <label className="grid gap-1">
                <Label>Your name</Label>
                <Input
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder="Dana Owner"
                  required
                />
              </label>
            </>
          )}
          <label className="grid gap-1">
            <Label>Email</Label>
            <Input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@company.com"
              autoComplete="email"
              required
            />
          </label>
          <label className="grid gap-1">
            <Label>Password</Label>
            <div className="relative">
              <Input
                type={showPassword ? "text" : "password"}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                minLength={10}
                autoComplete={mode === "signin" ? "current-password" : "new-password"}
                className="pr-12"
                required
              />
              <button
                type="button"
                aria-label={showPassword ? "Hide password" : "Show password"}
                aria-pressed={showPassword}
                onClick={() => setShowPassword((v) => !v)}
                className="absolute inset-y-0 right-0 flex w-9 items-center justify-center transition-opacity duration-100 hover:opacity-70"
                style={{ color: "var(--m-text-muted)" }}
              >
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
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
            {mode === "register" && <FieldHint>10+ characters</FieldHint>}
          </label>
          {error !== null && (
            <FieldError data-testid="login-error">{error}</FieldError>
          )}
          {slugSuggestion !== null && (
            <div
              className="flex items-center gap-2 rounded-lg px-2.5 py-2 text-xs"
              role="status"
              style={{ background: "var(--m-accent-soft)" }}
            >
              <span style={{ color: "var(--m-text-secondary)" }}>
                Available: <strong>{slugSuggestion}</strong>
              </span>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={acceptSuggestion}
              >
                Use {slugSuggestion}
              </Button>
            </div>
          )}
          <Button type="submit" variant="primary" disabled={busy}>
            {busy && <Spinner />}
            {busy ? "Working…" : mode === "signin" ? "Sign in" : "Create organization"}
          </Button>
        </form>
        {mode === "signin" && providers.length > 0 && (
          <>
            <div
              className="my-4 flex items-center gap-2 text-xs"
              style={{ color: "var(--m-text-secondary)" }}
            >
              <span className="h-px flex-1" style={{ background: "var(--m-border)" }} />
              or continue with
              <span className="h-px flex-1" style={{ background: "var(--m-border)" }} />
            </div>
            <div className="grid gap-2">
              {providers.map((p, i) => (
                <div key={p.name} className="m-rise" style={{ animationDelay: `${Math.min(i, 3) * 40}ms` }}>
                  <a href={ssoHref(p.name)} className="block">
                    <Button type="button" variant="secondary" className="w-full">
                      {p.display_name}
                    </Button>
                  </a>
                </div>
              ))}
            </div>
          </>
        )}
      </Card>
    </main>
  );
}
