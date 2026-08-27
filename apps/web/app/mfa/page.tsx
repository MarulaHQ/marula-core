// /mfa — second-factor challenge completing an mfa_required login.
//
// Purpose: accepts the pending bridge token from /login, verifies a TOTP or
// recovery code, or runs the WebAuthn passkey assertion flow; stores tokens
// and continues to /today.
// Non-purpose: enrollment and management live in /settings/security.

"use client";

import { Button, Card, Input, Label } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api, saveTokens } from "../../lib/api";

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

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

export default function MfaChallengePage() {
  const router = useRouter();
  const [mfaToken, setMfaToken] = useState<string | null>(null);
  const [methods, setMethods] = useState<string[]>([]);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const pending = window.sessionStorage.getItem("meridian-mfa-pending");
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
    window.sessionStorage.removeItem("meridian-mfa-pending");
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
      setError(err instanceof Error ? err.message : "Verification failed");
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
      setError(err instanceof Error ? err.message : "Passkey sign-in failed");
    } finally {
      setBusy(false);
    }
  }

  if (!mfaToken) return null;

  return (
    <main
      className="flex min-h-screen items-center justify-center p-6"
      style={{ background: "var(--m-canvas)" }}
    >
      <Card className="w-full max-w-sm p-6">
        <h1 className="text-base font-semibold">Two-factor verification</h1>
        <p className="mt-1 mb-4 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          Your account is protected with a second factor. Enter a code from your
          authenticator app, a recovery code, or use a passkey.
        </p>
        <form onSubmit={submitCode} className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="code">Authenticator or recovery code</Label>
            <Input
              id="code"
              value={code}
              onChange={(e) => setCode(e.target.value)}
              placeholder="123456 or XXXXX-XXXXX"
              autoFocus
              autoComplete="one-time-code"
            />
          </div>
          {error && (
            <p className="text-xs" style={{ color: "var(--m-status-danger, #b3261e)" }}>
              {error}
            </p>
          )}
          <Button type="submit" disabled={busy || code.trim().length === 0}>
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
      </Card>
    </main>
  );
}
