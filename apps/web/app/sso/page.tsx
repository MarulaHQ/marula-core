// /sso — SSO callback landing: exchanges the single-use handoff code for
// tokens (or confirms an account link), then continues into the app.
//
// Purpose: the only page that consumes ?code= from the core-api OIDC
// callback redirect; codes are single-use and expire in 60 seconds.
// Non-purpose: no provider logic here — everything happened server-side.

"use client";

import { Button, Card, Spinner } from "@meridian/ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";
import { AuthBrand } from "../../components/auth/auth-brand";
import { resolveApiBaseUrl, saveTokens } from "../../lib/api";

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
      className="flex min-h-screen items-center justify-center p-4 md:p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7" data-testid="sso-card">
        <AuthBrand subtitle="Meridian account security." />
        <div className="mt-5 grid justify-items-center gap-3 text-center">
          {status === "working" && (
            <span
              aria-hidden="true"
              className="flex h-10 w-10 items-center justify-center rounded-full"
              style={{ background: "var(--m-status-neutral-bg)" }}
            >
              <Spinner label="Completing sign-in" />
            </span>
          )}
          {status === "linked" && (
            <span
              aria-hidden="true"
              className="flex h-10 w-10 items-center justify-center rounded-full"
              style={{
                background: "var(--m-status-success-bg)",
                color: "var(--m-status-success-fg)",
              }}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M4 12.5 10 18 20 6" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
          )}
          {status === "error" && (
            <span
              aria-hidden="true"
              className="flex h-10 w-10 items-center justify-center rounded-full"
              style={{
                background: "var(--m-status-danger-bg)",
                color: "var(--m-status-danger-fg)",
              }}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M12 3 2 21h20L12 3z" strokeLinejoin="round" />
                <path d="M12 10v5" strokeLinecap="round" />
                <circle cx="12" cy="18" r="0.5" fill="currentColor" />
              </svg>
            </span>
          )}
          <h1 className="font-heading text-base font-semibold tracking-tight">Single sign-on</h1>
          <p
            role={status === "error" ? "alert" : "status"}
            className="text-[13px]"
            style={{ color: status === "error" ? "var(--m-status-danger-fg)" : "var(--m-text-secondary)" }}
          >
            {message}
          </p>
        </div>
        {status !== "working" && (
          <Button
            variant={status === "error" ? "secondary" : "primary"}
            className="m-rise mt-5 w-full"
            style={{ animationDelay: "40ms" }}
            onClick={() => router.replace(status === "error" ? "/login" : "/today")}
          >
            {status === "error" ? "Back to sign-in" : "Continue"}
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
