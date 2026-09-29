// /mfa — second-factor challenge completing an mfa_required login.
//
// Purpose: accepts the pending bridge token from /login, verifies a TOTP or
// recovery code, or runs the WebAuthn passkey assertion flow; stores tokens
// and continues to /today.
// Non-purpose: enrollment and management live in /settings/security.

//
// Wave 17 lane 9 UX pass: Tangerine brand header, mono code field with
// letter-count affordance, method pills (typed from the server methods
// list), icon'd error row, back-to-sign-in affordance, staggered entrance
// motion (≤120ms transform/opacity, prefers-reduced-motion collapses).
// Security copy and every wired call unchanged.

"use client";

import { FieldError } from "@marula-core/ui";
import { friendlyMutationText } from "../../lib/error-copy";
import { Button, Card, Input, Label, Spinner, Tag } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { AuthBrand } from "../../components/auth/auth-brand";
import { api, resolveApiBaseUrl, saveTokens } from "../../lib/api";

const API_BASE = resolveApiBaseUrl();

function b64urlToBuffer(value: string): ArrayBuffer {
  const pad = value.length % 4 === 0 ? "" : "=".repeat(4 - (value.length % 4));
  const base64 = value.replace(/-/g, "+").replace(/_/g, "/") + pad;
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

function bufferToB64url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Server method strings → legible pill labels (UI labels only; the
 * verification itself always goes to the server with the raw code). */
function methodLabel(method: string): string | null {
  if (method === "totp") return "Authenticator";
  if (method === "recovery") return "Recovery code";
  if (method === "webauthn") return "Passkey";
  return null;
}

export default function MfaChallengePage() {
  const router = useRouter();
  const [mfaToken, setMfaToken] = useState<string | null>(null);
  const [methods, setMethods] = useState<string[]>([]);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const pending = window.sessionStorage.getItem("marula-mfa-pending");
    if (!pending) {
      router.replace("/login");
      return;
    }
    const parsed = JSON.parse(pending) as { mfa_token: string; methods: string[] };
    setMfaToken(parsed.mfa_token);
    setMethods(parsed.methods ?? []);
  }, [router]);

  async function complete(tokens: {
    access_token: string;
    refresh_token: string;
    session_id: string;
    expires_in: number;
  }) {
    window.sessionStorage.removeItem("marula-mfa-pending");
    saveTokens(tokens);
    window.location.assign("/today");
  }

  async function submitCode(e: React.FormEvent) {
    e.preventDefault();
    if (!mfaToken || busy) return;
    setBusy(true);
    setError(null);
    try {
      await complete(await api.mfaVerify({ mfa_token: mfaToken, code }));
    } catch (err) {
      setError(friendlyMutationText(err, "Verification failed"));
    } finally {
      setBusy(false);
    }
  }

  async function signInWithPasskey() {
    if (!mfaToken || busy) return;
    setBusy(true);
    setError(null);
    try {
      const beginResponse = await fetch(`${API_BASE}/api/v1/auth/mfa/webauthn/begin`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ mfa_token: mfaToken }),
      });
      if (!beginResponse.ok) throw new Error(`passkey begin failed (${beginResponse.status})`);
      const begin = (await beginResponse.json()) as {
        public_key: { publicKey?: Record<string, unknown> } & Record<string, unknown>;
        challenge_token: string;
      };

      const envelope = begin.public_key as { publicKey: Record<string, unknown> };
      const raw = (envelope.publicKey ?? begin.public_key) as {
        challenge: string;
        allowCredentials?: Array<{ id: string } & Record<string, unknown>>;
      } & Record<string, unknown>;
      const requestOptions = {
        ...raw,
        challenge: b64urlToBuffer(raw.challenge),
        allowCredentials: (raw.allowCredentials ?? []).map((descriptor) => ({
          ...descriptor,
          id: b64urlToBuffer(descriptor.id),
        })),
      } as unknown as PublicKeyCredentialRequestOptions;
      const credential = (await navigator.credentials.get({
        publicKey: requestOptions,
      })) as PublicKeyCredential | null;
      if (!credential) throw new Error("no credential returned by authenticator");

      const serialized = JSON.parse(JSON.stringify(credential)) as Record<string, unknown>;
      const response = serialized.response as Record<string, string>;
      const assertion = credential.response as AuthenticatorAssertionResponse;
      response.clientDataJSON = bufferToB64url(assertion.clientDataJSON);
      response.authenticatorData = bufferToB64url(assertion.authenticatorData);
      response.signature = bufferToB64url(assertion.signature);
      response.userHandle = bufferToB64url((assertion.userHandle ?? new ArrayBuffer(0)) as ArrayBuffer);

      const finishResponse = await fetch(`${API_BASE}/api/v1/auth/mfa/webauthn/finish`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          challenge_token: begin.challenge_token,
          credential: serialized,
        }),
      });
      if (!finishResponse.ok) throw new Error(`passkey rejected (${finishResponse.status})`);
      await complete((await finishResponse.json()) as Parameters<typeof saveTokens>[0]);
    } catch (err) {
      setError(friendlyMutationText(err, "Passkey sign-in failed"));
    } finally {
      setBusy(false);
    }
  }

  if (!mfaToken) return null;

  const pills = methods.map(methodLabel).filter((m): m is string => m !== null);

  return (
    <main
      className="flex min-h-screen items-center justify-center p-4 md:p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card className="m-rise w-full max-w-sm rounded-xl p-6 md:p-7" data-testid="mfa-card">
        <AuthBrand subtitle="Meridian account security." />
        <h1 className="mt-4 text-base font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
          Two-factor verification
        </h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Your account is protected with a second factor. Enter a code from
          your authenticator app, a recovery code, or use a passkey.
        </p>
        {pills.length > 0 && (
          <div className="mt-3 flex flex-wrap gap-1.5" aria-label="Available second factors">
            {pills.map((label) => (
              <Tag key={label} tone="outline" style={{ border: "1px solid var(--m-border)" }}>
                {label}
              </Tag>
            ))}
          </div>
        )}
        <form onSubmit={submitCode} className="m-rise mt-5 grid gap-3" style={{ animationDelay: "40ms" }}>
          <div className="grid gap-1.5">
            <Label htmlFor="code">Authenticator or recovery code</Label>
            <Input
              id="code"
              value={code}
              onChange={(e) => setCode(e.target.value)}
              placeholder="123456"
              className="font-mono tracking-widest"
              autoFocus
              autoComplete="one-time-code"
              inputMode="numeric"
            />
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Six digits from your authenticator, or a recovery code
              (XXXXX-XXXXX).
            </p>
          </div>
          {error && (
            <FieldError data-testid="mfa-error">{error}</FieldError>
          )}
          <Button type="submit" variant="primary" disabled={busy || code.trim().length === 0}>
            {busy && <Spinner />}
            {busy ? "Verifying…" : "Verify"}
          </Button>
        </form>
        {methods.includes("webauthn") && (
          <>
            <div
              className="my-4 flex items-center gap-2 text-xs"
              style={{ color: "var(--m-text-secondary)" }}
            >
              <span className="h-px flex-1" style={{ background: "var(--m-border)" }} />
              or
              <span className="h-px flex-1" style={{ background: "var(--m-border)" }} />
            </div>
            <Button variant="secondary" onClick={signInWithPasskey} disabled={busy} className="w-full">
              Use a passkey
            </Button>
          </>
        )}
        <button
          type="button"
          onClick={() => window.location.assign("/login")}
          className="mt-5 w-full text-xs underline underline-offset-2 transition-opacity duration-100 hover:opacity-70"
          style={{ color: "var(--m-text-muted)" }}
        >
          Back to sign-in
        </button>
      </Card>
    </main>
  );
}
