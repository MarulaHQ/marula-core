// /portal/login — client-portal sign-in.
//
// Purpose: email + password against /api/portal/login (the DEDICATED
// portal token family — a staff account cannot log in here and vice
// versa). No MFA in portal v1 (documented deferral).
// Non-purpose: no staff login, no registration, no SSO — the portal is an
// invite-only surface provisioned by the MSP.
"use client";

import { Button, Card, FieldHint, Input, Label } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { portalApi, usePortalAuth } from "../../../lib/portal-api";

export default function PortalLoginPage() {
  const router = useRouter();
  const { status } = usePortalAuth();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  useEffect(() => {
    if (status === "authed") router.replace("/portal/tickets");
  }, [status, router]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await portalApi.login(email.trim(), password);
      router.replace("/portal/tickets");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Sign-in failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-[60vh] items-center justify-center">
      <Card className="w-full max-w-sm p-6">
        <h1 className="text-base font-semibold">Client Portal sign-in</h1>
        <p className="mt-1 text-xs" style={{ color: "var(--m-text-muted)" }}>
          Support access provided by your IT service desk.
        </p>
        <form className="mt-5 space-y-4" onSubmit={submit}>
          <div className="space-y-1.5">
            <Label htmlFor="portal-email">Work email</Label>
            <Input
              id="portal-email"
              type="email"
              autoComplete="email"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@company.com"
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="portal-password">Password</Label>
            <Input
              id="portal-password"
              type="password"
              autoComplete="current-password"
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="••••••••••"
            />
            <FieldHint>
              Your service desk provisions accounts and can reset this password.
            </FieldHint>
          </div>
          {error !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger)" }} role="alert">
              {error}
            </p>
          )}
          <Button type="submit" className="w-full" disabled={busy}>
            {busy ? "Signing in…" : "Sign in"}
          </Button>
        </form>
      </Card>
    </div>
  );
}
