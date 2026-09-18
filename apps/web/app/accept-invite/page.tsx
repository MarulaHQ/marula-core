// /accept-invite?token=… — the public invite-redemption page.
//
// Purpose: the customer-facing half of the org-users invite flow (Wave 11
// lane 2 backend, migration 0047). The invited member opens the one-time
// setup link out-of-band; this page accepts the setup token, takes a new
// password, and redeems `POST /org/users/accept-invite` — which returns a
// standard token pair, so the member lands logged in. Unknown / used /
// expired tokens are ALL a bare 404 server-side and render one friendly
// state here. Lives OUTSIDE the authed shell (headerless route).
// Non-purpose: no staff surfaces, no branding beyond Workbench tokens.
"use client";

import type { components } from "@meridian/sdk";
import { Button, Card, Input, Label, Spinner } from "@meridian/ui";
import { applyTheme } from "@meridian/ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState, type ReactNode } from "react";

import { saveTokens } from "../../lib/api";
import { api } from "../../lib/api";
import { OFFLINE_RETRY_COPY } from "../../lib/error-copy";

function AcceptInviteInner(): ReactNode {
  const router = useRouter();
  const params = useSearchParams();
  const token = params.get("token") ?? "";

  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  useEffect(() => {
    // Public page: always render in the customer theme (light), never
    // inherit a staff member's persisted preference on a shared device.
    applyTheme("light");
  }, []);

  const redeem = async (e: React.FormEvent) => {
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
      const { data, error: apiError } = await api.api.POST("/api/v1/org/users/accept-invite", {
        body: { token, password },
      });
      if (apiError !== undefined || data === undefined) {
        const body = apiError as { error?: { code?: number; message?: string } } | undefined;
        if (body?.error?.code === 404) {
          setError(
            "This invite link is not valid — it may already have been used or has expired. Ask your administrator for a fresh invite.",
          );
          return;
        }
        setError(body?.error?.message ?? "The invite could not be redeemed.");
        return;
      }
      // Standard token pair: persist exactly like a login and do a full
      // navigation so AuthProvider re-bootstraps from the fresh tokens.
      saveTokens(data as components["schemas"]["TokenPair"]);
      setDone(true);
      window.location.assign("/today");
    } catch {
      setError(OFFLINE_RETRY_COPY);
    } finally {
      setBusy(false);
    }
  };

  if (token === "") {
    return (
      <main
        className="m-rise flex min-h-full items-center justify-center p-6"
        style={{ background: "var(--m-canvas)" }}
      >
        <Card className="w-full max-w-sm p-6">
          <div className="mb-3 text-center">
            <span className="font-heading text-lg font-semibold tracking-tight">Meridian</span>
          </div>
          <h1 className="text-base font-semibold">Accept invite</h1>
          <p className="mt-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            This page needs the setup link your administrator sent you (it should look like
            {" "}
            <code className="font-mono text-[12px]">…/accept-invite?token=…</code>).
          </p>
          <Button className="mt-4" variant="secondary" onClick={() => router.replace("/")}>
            Go home
          </Button>
        </Card>
      </main>
    );
  }

  return (
    <main
      className="m-rise flex min-h-full items-center justify-center p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card className="w-full max-w-sm p-6" data-testid="accept-invite-card">
        <div className="mb-3 text-center">
          <span className="font-heading text-lg font-semibold tracking-tight">Meridian</span>
        </div>
        <h1 className="text-base font-semibold">Set your password</h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Finish setting up your Meridian account. You will land signed in.
        </p>
        <form className="mt-4 grid gap-3" onSubmit={redeem}>
          <div>
            <Label htmlFor="invite-password">New password</Label>
            <Input
              id="invite-password"
              type="password"
              autoComplete="new-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
              minLength={10}
            />
          </div>
          <div>
            <Label htmlFor="invite-password-confirm">Confirm password</Label>
            <Input
              id="invite-password-confirm"
              type="password"
              autoComplete="new-password"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              required
            />
          </div>
          {error !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="accept-invite-error">
              {error}
            </p>
          ) : null}
          <Button type="submit" variant="primary" disabled={busy || done}>
            {busy ? <Spinner /> : null} Set password and sign in
          </Button>
        </form>
      </Card>
    </main>
  );
}

export default function AcceptInvitePage(): ReactNode {
  return (
    <Suspense
      fallback={
        <main className="flex min-h-full items-center justify-center p-6" style={{ background: "var(--m-canvas)" }}>
          <Spinner />
        </main>
      }
    >
      <AcceptInviteInner />
    </Suspense>
  );
}
