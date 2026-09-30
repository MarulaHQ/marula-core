// /mfa — the second step. Indigo, because it protects something.
//
// Purpose: the 6-digit code from an authenticator app, on the pack's state 03:
// a shield in a gold circle, the serif headline "One more step, then you're
// in", six boxes with the active one outlined, a Verify that stays disabled
// until there are six digits, and a backup code one click away.
//
// THE TRUST CHECKBOX IS REAL. The pack draws "Trust this computer for 30 days"
// checked by default, and `MfaVerifyRequest` now carries `trust_device` (it did
// not when this screen was first built, which is why the box was omitted rather
// than faked). The server honours it by lengthening the session's REFRESH window
// to 30 days — the access token keeps its short TTL and the session row stays
// revocable, so trusting a browser buys convenience and never an irrevocable
// grant.
//
// Non-purpose: no passkey management, no recovery-code list, no device naming.
// Those live in Settings, Security.
"use client";

import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { Spinner } from "@marula-core/ui";
import { friendlyMutationText } from "../../lib/error-copy";
import { EntryCard, EntryHeadline, EntryIcon, EntryShell, EntryShield, OtpBoxes } from "../../components/auth/entry";

/** What the pending sign-in left in session storage, if anything. */
interface Pending {
  mfa_token: string;
  methods: string[];
}

function readPending(): Pending | null {
  if (typeof window === "undefined") return null;
  const raw = window.sessionStorage.getItem("marula-mfa-pending");
  if (raw === null) return null;
  try {
    const parsed = JSON.parse(raw) as Pending;
    return typeof parsed.mfa_token === "string" ? parsed : null;
  } catch {
    return null;
  }
}

function methodLabel(method: string): string | null {
  if (method === "totp") return null;
  if (method === "webauthn" || method === "passkey") return "passkey";
  return method;
}

export default function MfaChallengePage() {
  const router = useRouter();
  const [pending, setPending] = useState<Pending | null>(null);
  const [code, setCode] = useState("");
  const [trustDevice, setTrustDevice] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [backupMode, setBackupMode] = useState(false);

  useEffect(() => {
    setPending(readPending());
  }, []);

  const verify = useCallback(
    async (value: string) => {
      if (pending === null || busy) return;
      setBusy(true);
      setError(null);
      try {
        const { api, saveTokens } = await import("../../lib/api");
        const result = await api.mfaVerify({
          mfa_token: pending.mfa_token,
          code: value,
          trust_device: trustDevice,
          // TEMPORARY, AND THE ONLY CAST IN THIS LANE.
          //
          // `trust_device` is live in the API (MfaVerifyRequest gained the
          // field in this lane) but NOT yet in the generated SDK type, because
          // the OpenAPI pins are stale — the first `regen-openapi-pins.sh` run
          // fetched a stray core-api that did not have these routes and
          // produced a pair byte-identical to HEAD, so the regen has to be
          // re-run on a clean stack. Delete this cast in the same commit that
          // regenerates the pins; it exists so the checkbox sends a field the
          // server honours instead of shipping a control that does nothing.
        } as Parameters<typeof api.mfaVerify>[0]);
        saveTokens(result);
        window.sessionStorage.removeItem("marula-mfa-pending");
        window.location.assign("/today");
      } catch (err) {
        setCode("");
        setError(
          friendlyMutationText(
            err,
            "That code didn't match. Check the six digits and try again.",
          ),
        );
      } finally {
        setBusy(false);
      }
    },
    [busy, pending, trustDevice],
  );

  const alternative = pending?.methods.map(methodLabel).find((m) => m !== null);

  return (
    <EntryShell background="indigo" fruit="none">
      <EntryCard>
        <EntryShield />
        <div>
          <EntryHeadline>One more step, then you&apos;re in.</EntryHeadline>
          <p className="entry-sub">
            {backupMode
              ? "Type one of the backup codes you saved when you turned this on."
              : "Open your authenticator app and type the 6-digit code it shows."}
          </p>
        </div>

        {pending === null ? (
          <div style={{ textAlign: "center" }}>
            <p className="entry-hint" style={{ marginBottom: 12 }}>
              This page needs a sign-in to finish. Start again and we&apos;ll bring you back here.
            </p>
            <button
              type="button"
              className="entry-btn entry-btn--full"
              onClick={() => router.replace("/login")}
            >
              Back to sign in
            </button>
          </div>
        ) : (
          <>
            {error !== null && (
              <p className="entry-error" role="alert" data-testid="mfa-error">
                <EntryIcon name="alert" size={14} />
                <span>{error}</span>
              </p>
            )}

            <div className="entry-field">
              <span className="entry-label" id="mfa-code-label">
                {backupMode ? "Backup code" : "6-digit code"}
              </span>
              <OtpBoxes
                id="mfa-code"
                label={backupMode ? "Backup code" : "6-digit code"}
                value={code}
                onChange={setCode}
                onComplete={(value) => void verify(value)}
              />
            </div>

            {!backupMode && (
              /* The pack draws this checked by default
                 (auth/neutral/states/03-mfa.png), and `trust_device` is now a
                 real field on MfaVerifyRequest — so the choice is recorded
                 rather than displayed. It lengthens the REFRESH window to 30
                 days and nothing else: the access token keeps its short TTL and
                 the session stays revocable. */
              <label className="entry-check">
                <input
                  type="checkbox"
                  checked={trustDevice}
                  onChange={(e) => setTrustDevice(e.target.checked)}
                />
                <span>Trust this computer for 30 days</span>
              </label>
            )}

            <button
              type="button"
              className="entry-btn"
              disabled={code.length !== 6 || busy}
              onClick={() => void verify(code)}
              data-testid="mfa-verify"
            >
              {busy ? (
                <>
                  <Spinner /> Checking…
                </>
              ) : backupMode ? (
                "Use this code"
              ) : (
                "Verify"
              )}
            </button>

            <div style={{ textAlign: "center" }}>
              {backupMode ? (
                <button
                  type="button"
                  className="entry-link"
                  style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                  onClick={() => {
                    setBackupMode(false);
                    setCode("");
                    setError(null);
                  }}
                >
                  Use the app instead
                </button>
              ) : alternative !== undefined ? (
                <button
                  type="button"
                  className="entry-link"
                  style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                  onClick={() => {
                    setBackupMode(true);
                    setCode("");
                    setError(null);
                  }}
                  data-testid="mfa-alternative"
                >
                  {alternative === "passkey" ? "Use your passkey instead" : "Use a backup code instead"}
                </button>
              ) : (
                <button
                  type="button"
                  className="entry-link"
                  style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
                  onClick={() => {
                    setBackupMode(true);
                    setCode("");
                    setError(null);
                  }}
                  data-testid="mfa-alternative"
                >
                  Use a backup code instead
                </button>
              )}
            </div>
          </>
        )}
      </EntryCard>
    </EntryShell>
  );
}
