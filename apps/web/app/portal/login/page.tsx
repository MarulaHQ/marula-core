// /portal/login — client-portal sign-in.
//
// Purpose: email + password against /api/v1/portal/login (the DEDICATED
// portal token family — a staff account cannot log in here and vice
// versa). No MFA in portal v1 (documented deferral).
// Non-purpose: no staff login, no registration, no SSO — the portal is an
// invite-only surface provisioned by the MSP.
//
// Wave 17 lane 8 UX pass: brand mark + generous card rhythm, entrance
// motion riding the shared m-rise keyframe. Lane 10: the marula-ci logo
// mark replaces the placeholder globe; white-label-lite substitution (org
// name + accent as deployment config, honest Marula defaults).
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { Button, Card, FieldHint, Input, Label } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { portalApi, usePortalAuth } from "../../../lib/portal-api";
import { BrandMark } from "../../../components/brand";
import { portalAccentOverrides, portalOrgName } from "../../../lib/portal-branding";

const ORG_NAME = portalOrgName() ?? "Marula";

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
      setError(friendlyMutationText(err, "Sign-in failed"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="m-rise flex min-h-[64vh] items-center justify-center"
      style={portalAccentOverrides()}
    >
      <div className="w-full max-w-sm" data-testid="portal-login-card">
        <div className="mb-6 flex flex-col items-center gap-3 text-center">
          <BrandMark size={32} />
          <div>
            <h1 className="text-xl font-semibold tracking-tight">
              {ORG_NAME} — client portal sign-in
            </h1>
            <p className="mt-1 text-sm" style={{ color: "var(--m-text-muted)" }}>
              Support access provided by your service desk.
            </p>
          </div>
        </div>
        <Card className="p-6">
          <form className="space-y-4" onSubmit={submit}>
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
    </div>
  );
}
