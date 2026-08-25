// /login — email+password sign-in and first-run org registration.
//
// Purpose: segmented form (Sign in / Create org) against the public auth
// endpoints via @meridian/sdk; stores tokens and routes to /today.
// Non-purpose: no SSO, no MFA enrollment UI yet.
"use client";

import { Button, Card, FieldHint, Input, Label } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { api, saveTokens, useAuth } from "../../lib/api";

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

  if (status === "authed") {
    router.replace("/today");
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      if (mode === "signin") {
        saveTokens(await api.login({ email, password }));
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
      </Card>
    </main>
  );
}
