// /sso — SSO callback landing: exchanges the single-use handoff code for
// tokens (or confirms an account link), then continues into the app.
//
// Purpose: the only page that consumes ?code= from the core-api OIDC
// callback redirect; codes are single-use and expire in 60 seconds.
// Non-purpose: no provider logic here — everything happened server-side.

"use client";

import { Button, Card } from "@meridian/ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";
import { api, resolveApiBaseUrl, saveTokens } from "../../lib/api";

const API_BASE = resolveApiBaseUrl();

function SsoLanding() {
  const router = useRouter();
  const params = useSearchParams();
  const [status, setStatus] = useState<"working" | "linked" | "error">("working");
  const [message, setMessage] = useState("Completing sign-in…");

  useEffect(() => {
    const error = params.get("error");
    if (error) {
      setStatus("error");
      setMessage(
        error === "link_required"
          ? "Sign-in refused: this identity is not linked to a Meridian account yet. An administrator must link it first."
          : `Sign-in failed (${error}).`,
      );
      return;
    }
    const code = params.get("code");
    if (!code) {
      setStatus("error");
      setMessage("Missing handoff code.");
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const response = await fetch(`${API_BASE}/api/v1/sso/exchange`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ code }),
        });
        const body = (await response.json()) as Record<string, unknown>;
        if (!response.ok) throw new Error("exchange failed");
        if (body.linked === true) {
          if (!cancelled) {
            setStatus("linked");
            setMessage("Identity linked. You can close this window or continue.");
          }
          return;
        }
        saveTokens(body as unknown as Parameters<typeof saveTokens>[0]);
        window.location.assign("/today");
      } catch {
        if (!cancelled) {
          setStatus("error");
          setMessage("This handoff code is invalid, used, or expired. Try signing in again.");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [params]);

  return (
    <main
      className="flex min-h-screen items-center justify-center p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card className="w-full max-w-sm p-6">
        <h1 className="text-base font-semibold">Single sign-on</h1>
        <p className="mt-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          {message}
        </p>
        {status === "error" && (
          <Button className="mt-4 w-full" onClick={() => router.replace("/login")}>
            Back to sign-in
          </Button>
        )}
        {status === "linked" && (
          <Button className="mt-4 w-full" onClick={() => router.replace("/today")}>
            Continue
          </Button>
        )}
      </Card>
    </main>
  );
}

export default function SsoPage() {
  return (
    <Suspense fallback={null}>
      <SsoLanding />
    </Suspense>
  );
}
