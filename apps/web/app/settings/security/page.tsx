// /settings/security — MFA enrollment and session management.
//
// Purpose: TOTP setup wizard (secret shown once → confirm → recovery codes
// shown once), disable flow (password + current code), passkey
// registration/removal, and live refresh-session list with revocation.
// Non-purpose: org-wide SSO provider administration.

"use client";

import { Button, Card, FieldHint, Input, Label } from "@meridian/ui";
import { useCallback, useEffect, useState } from "react";
import { useAuth, useRequireAuth } from "../../../lib/api";

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

interface CredentialInfo {
  id: string;
  label: string;
  created_at: string;
  last_used_at: string | null;
}

interface SessionRow {
  id: string;
  user_agent: string | null;
  created_at: string;
  expires_at: string;
  current: boolean;
}

function b64urlToBuffer(value: string): ArrayBuffer {
  const pad = value.length % 4 === 0 ? "" : "=".repeat(4 - (value.length % 4));
  const binary = atob(value.replace(/-/g, "+").replace(/_/g, "/") + pad);
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

async function apiCall<T>(
  path: string,
  method: "POST" | "GET",
  body?: unknown,
): Promise<T> {
  const tokens = JSON.parse(window.localStorage.getItem("meridian-tokens") ?? "{}");
  const response = await fetch(`${API_BASE}${path}`, {
    method,
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${tokens.access_token ?? ""}`,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(`${path} failed (${response.status}): ${detail}`);
  }
  return (response.status === 204 ? {} : await response.json()) as T;
}

export default function SecuritySettingsPage() {
  useRequireAuth();
  const { me } = useAuth();

  // status flags
  const [credentials, setCredentials] = useState<CredentialInfo[]>([]);
  const [sessions, setSessions] = useState<SessionRow[]>([]);

  // totp wizard state
  const [setupSecret, setSetupSecret] = useState<string | null>(null);
  const [setupUrl, setSetupUrl] = useState<string | null>(null);
  const [confirmCode, setConfirmCode] = useState("");
  const [recoveryCodes, setRecoveryCodes] = useState<string[] | null>(null);

  // disable / regenerate forms
  const [password, setPassword] = useState("");
  const [disableCode, setDisableCode] = useState("");

  // passkey registration
  const [passkeyLabel, setPasskeyLabel] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      const creds = await apiCall<CredentialInfo[]>("/api/auth/mfa/webauthn/credentials", "GET");
      setCredentials(creds);
      const list = await apiCall<SessionRow[]>("/api/auth/sessions", "GET");
      setSessions(list);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load security state");
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  function guard(action: () => Promise<void>) {
    return async () => {
      setBusy(true);
      setError(null);
      try {
        await action();
      } catch (err) {
        setError(err instanceof Error ? err.message : "Request failed");
      } finally {
        setBusy(false);
      }
    };
  }

  const beginTotp = guard(async () => {
    const begin = await apiCall<{ secret_base32: string; otpauth_url: string }>(
      "/api/auth/mfa/totp/enroll-begin",
      "POST",
    );
    setSetupSecret(begin.secret_base32);
    setSetupUrl(begin.otpauth_url);
    setNotice(null);
  });

  const confirmTotp = guard(async () => {
    const result = await apiCall<{ recovery_codes: string[] }>(
      "/api/auth/mfa/totp/enroll-verify",
      "POST",
      { code: confirmCode.trim() },
    );
    setRecoveryCodes(result.recovery_codes);
    setSetupSecret(null);
    setSetupUrl(null);
    setConfirmCode("");
    setNotice("Two-factor authentication enabled.");
  });

  const disableTotp = guard(async () => {
    await apiCall("/api/auth/mfa/totp/disable", "POST", {
      password,
      code: disableCode.trim(),
    });
    setPassword("");
    setDisableCode("");
    setNotice("Two-factor authentication disabled.");
  });

  const regenerateCodes = guard(async () => {
    const result = await apiCall<{ recovery_codes: string[] }>(
      "/api/auth/mfa/recovery-codes/regenerate",
      "POST",
      { password, code: disableCode.trim() },
    );
    setPassword("");
    setDisableCode("");
    setRecoveryCodes(result.recovery_codes);
    setNotice("New recovery codes issued. Old codes no longer work.");
  });

  const registerPasskey = guard(async () => {
    const begin = await apiCall<{
      public_key: { publicKey?: Record<string, unknown> } & Record<string, unknown>;
      challenge_token: string;
    }>("/api/auth/mfa/webauthn/register/begin", "POST");

    const envelope = begin.public_key as { publicKey?: Record<string, unknown> };
    const raw = (envelope.publicKey ?? begin.public_key) as {
      challenge: string;
      excludeCredentials?: Array<{ id: string } & Record<string, unknown>>;
    } & Record<string, unknown>;
    const creationOptions = {
      ...raw,
      challenge: b64urlToBuffer(raw.challenge),
      excludeCredentials: (raw.excludeCredentials ?? []).map((descriptor) => ({
        ...descriptor,
        id: b64urlToBuffer(descriptor.id),
      })),
    } as unknown as PublicKeyCredentialCreationOptions;
    const credential = (await navigator.credentials.create({
      publicKey: creationOptions,
    })) as PublicKeyCredential | null;
    if (!credential) throw new Error("no credential returned by authenticator");

    const serialized = JSON.parse(JSON.stringify(credential)) as Record<string, unknown>;
    const response = serialized.response as Record<string, unknown>;
    const attestation = credential.response as AuthenticatorAttestationResponse;
    response.clientDataJSON = bufferToB64url(attestation.clientDataJSON);
    response.attestationObject = bufferToB64url(attestation.attestationObject);

    await apiCall("/api/auth/mfa/webauthn/register/finish", "POST", {
      challenge_token: begin.challenge_token,
      label: passkeyLabel.trim() || undefined,
      credential: serialized,
    });
    setPasskeyLabel("");
    setNotice("Passkey registered.");
    await refreshStatus();
  });

  return (
    <main className="mx-auto grid max-w-2xl gap-6 p-6" style={{ background: "var(--m-canvas)" }}>
      <header>
        <h1 className="text-lg font-semibold">Security</h1>
        <p className="mt-1 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          Signed in as {me?.email}. Manage two-factor authentication and devices.
        </p>
      </header>

      {(notice || error) && (
        <p
          className="text-xs"
          role={error ? "alert" : "status"}
          style={{ color: error ? "var(--m-status-danger-fg)" : "var(--m-status-success-fg)" }}
        >
          {error ?? notice}
        </p>
      )}

      {/* --- TOTP -------------------------------------------------------- */}
      <Card className="p-5">
        <h2 className="text-sm font-semibold">Authenticator app (TOTP)</h2>
        <p className="mt-1 mb-3 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          Require a rotating six-digit code from an authenticator app at sign-in.
        </p>

        {setupSecret === null ? (
          <Button variant="secondary" onClick={beginTotp} disabled={busy}>
            Start setup
          </Button>
        ) : (
          <div className="grid gap-3">
            <div className="grid gap-1">
              <Label>Secret (enter manually in your app)</Label>
              <code
                className="rounded-[2px] px-2 py-1 text-xs"
                style={{ background: "var(--m-surface-raised)", wordBreak: "break-all" }}
              >
                {setupSecret}
              </code>
            </div>
            <div className="grid gap-1">
              <Label>otpauth URI</Label>
              <code
                className="rounded-[2px] px-2 py-1 text-xs"
                style={{ background: "var(--m-surface-raised)", wordBreak: "break-all" }}
              >
                {setupUrl}
              </code>
              <FieldHint>Shown once. Confirm below with the first generated code.</FieldHint>
            </div>
            <label className="grid gap-1">
              <Label>Confirmation code</Label>
              <Input
                value={confirmCode}
                onChange={(e) => setConfirmCode(e.target.value)}
                placeholder="123456"
                inputMode="numeric"
              />
            </label>
            <Button onClick={confirmTotp} disabled={busy || confirmCode.trim().length !== 6}>
              Confirm and enable
            </Button>
          </div>
        )}
      </Card>

      {/* --- recovery codes ---------------------------------------------- */}
      {recoveryCodes && (
        <Card className="p-5">
          <h2 className="text-sm font-semibold">Recovery codes</h2>
          <p className="mt-1 mb-3 text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
            Shown once — store them somewhere safe. Each code works exactly once.
          </p>
          <ul className="grid grid-cols-2 gap-1 font-mono text-xs">
            {recoveryCodes.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
          <Button variant="ghost" className="mt-3" onClick={() => setRecoveryCodes(null)}>
            I&apos;ve saved them
          </Button>
        </Card>
      )}

      <Card className="p-5">
        <h2 className="text-sm font-semibold">Sensitive changes</h2>
        <p className="mt-1 mb-3 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          Disabling TOTP or regenerating codes requires your password plus a
          current second factor.
        </p>
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="grid gap-1">
            <Label>Password</Label>
            <Input type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
          </label>
          <label className="grid gap-1">
            <Label>Current code</Label>
            <Input
              value={disableCode}
              onChange={(e) => setDisableCode(e.target.value)}
              placeholder="TOTP or recovery"
            />
          </label>
        </div>
        <div className="mt-3 flex gap-2">
          <Button variant="secondary" onClick={disableTotp} disabled={busy || !password || !disableCode}>
            Disable TOTP
          </Button>
          <Button variant="secondary" onClick={regenerateCodes} disabled={busy || !password || !disableCode}>
            Regenerate recovery codes
          </Button>
        </div>
      </Card>

      {/* --- passkeys ------------------------------------------------------ */}
      <Card className="p-5">
        <h2 className="text-sm font-semibold">Passkeys</h2>
        <p className="mt-1 mb-3 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          Platform authenticators (Touch ID, Windows Hello, security keys).
        </p>
        {credentials.length > 0 && (
          <ul className="mb-3 grid gap-1 text-xs">
            {credentials.map((c) => (
              <li key={c.id} className="flex items-center justify-between gap-2">
                <span>{c.label}</span>
                <Button
                  variant="ghost"
                  onClick={guard(async () => {
                    await apiCall(`/api/auth/mfa/webauthn/credentials/${c.id}/remove`, "POST", {
                      password,
                    });
                    await refreshStatus();
                  })}
                  disabled={busy || !password}
                >
                  Remove
                </Button>
              </li>
            ))}
          </ul>
        )}
        <div className="flex gap-2">
          <Input
            value={passkeyLabel}
            onChange={(e) => setPasskeyLabel(e.target.value)}
            placeholder="Label (e.g. MacBook Touch ID)"
            className="max-w-xs"
          />
          <Button variant="secondary" onClick={registerPasskey} disabled={busy}>
            Register passkey
          </Button>
        </div>
      </Card>

      {/* --- sessions ------------------------------------------------------ */}
      <Card className="p-5">
        <h2 className="text-sm font-semibold">Sessions</h2>
        <p className="mt-1 mb-3 text-xs" style={{ color: "var(--m-text-secondary)" }}>
          Live refresh-token chains bound to this account.
        </p>
        <ul className="grid gap-1 text-xs">
          {sessions.map((s) => (
            <li key={s.id} className="flex items-center justify-between gap-2">
              <span>
                {s.current ? "This device" : (s.user_agent ?? "Unknown client")}
                {" · "}
                {new Date(s.created_at).toLocaleString()}
              </span>
              {!s.current && (
                <Button
                  variant="ghost"
                  onClick={guard(async () => {
                    await apiCall(`/api/auth/sessions/${s.id}/revoke`, "POST");
                    await refreshStatus();
                  })}
                  disabled={busy}
                >
                  Revoke
                </Button>
              )}
            </li>
          ))}
        </ul>
      </Card>
    </main>
  );
}
