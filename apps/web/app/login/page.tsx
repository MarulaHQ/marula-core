// /login — email+password sign-in, MFA hand-off, SSO entry, and first-run
// org registration.
//
// Purpose: segmented form (Sign in / Create org) against the public auth
// endpoints via @meridian/sdk; routes MFA-gated logins to /mfa; renders
// config-driven SSO buttons from the enabled OIDC providers.
// Non-purpose: no enrollment/management UI here (see /settings/security).
"use client";

import { Button, Card, FieldHint, Input, Label } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api, saveTokens, useAuth } from "../../lib/api";

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

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
  const [orgName, setOrgName] = useState("");
  const [orgSlug, setOrgSlug] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [providers, setProviders] = useState<PublicProvider[]>([]);

  // Config-driven SSO: only providers an operator enabled appear here.
  useEffect(() => {
    let cancelled = false;
    fetch(`${API_BASE}/api/auth/oidc/providers/public`)
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
    setBusy(true);
    setError(null);
    try {
      if (mode === "signin") {
        const result = await api.login({ email, password });
        if ("mfa_required" in result) {
          window.sessionStorage.setItem(
            "meridian-mfa-pending",
            JSON.stringify({ mfa_token: result.mfa_token, methods: result.methods }),
          );
          window.location.assign("/mfa");
          return;
        }
        saveTokens(result);
      } else {
        saveTokens(
          await api.register({
            org_name: orgName,
            org_slug: orgSlug,
            email,
            password,
            display_name: displayName,
          }),
        );
      }
      // Full navigation so AuthProvider re-bootstraps from the fresh tokens.
      window.location.assign("/today");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Authentication failed");
    } finally {
      setBusy(false);
    }
  }

  function ssoHref(provider: string): string {
    return `${API_BASE}/api/auth/oidc/${provider}/login`;
  }

  return (
    <main
      className="flex min-h-screen items-center justify-center p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card className="w-full max-w-sm p-6">
        <div className="mb-4">
          <span className="text-base font-semibold">Meridian</span>
          <p className="mt-1 text-xs" style={{ color: "var(--m-text-secondary)" }}>
            Sign in to your service desk, or create your organization.
          </p>
        </div>
        <div
          className="mb-4 grid grid-cols-2 gap-1 rounded-[2px] p-1"
          style={{ background: "var(--m-status-neutral-bg)" }}
          role="tablist"
        >
          {MODES.map((m) => (
            <button
              key={m.id}
              role="tab"
              aria-selected={mode === m.id}
              onClick={() => setMode(m.id)}
              className="rounded-[2px] px-3 py-1.5 text-xs font-medium"
              style={
                mode === m.id
                  ? { background: "var(--m-surface)", color: "var(--m-text)" }
                  : { color: "var(--m-text-secondary)" }
              }
            >
              {m.label}
            </button>
          ))}
        </div>
        <form onSubmit={submit} className="grid gap-3">
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
              required
            />
          </label>
          <label className="grid gap-1">
            <Label>Password</Label>
            <Input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              minLength={10}
              required
            />
            <FieldHint>10+ characters</FieldHint>
          </label>
          {error !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
              {error}
            </p>
          )}
          <Button type="submit" variant="primary" disabled={busy}>
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
              {providers.map((p) => (
                <a key={p.name} href={ssoHref(p.name)} className="block">
                  <Button type="button" variant="secondary" className="w-full">
                    {p.display_name}
                  </Button>
                </a>
              ))}
            </div>
          </>
        )}
      </Card>
    </main>
  );
}
