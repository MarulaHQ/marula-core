// /sso — the company door, and its landing. Indigo, because it protects.
//
// Purpose: two halves on one route. WITHOUT a code it is the pack's state 06:
// type a company domain, and the screen says which provider it found for that
// domain, so nobody discovers at the provider's login page that they guessed
// wrong. WITH a code it is the callback landing, which exchanges the
// single-use handoff for tokens (or confirms a link) and continues in.
//
// The provider is read from the public providers endpoint and matched on the
// domain, so the sentence "We found Microsoft 365 for this domain" is a real
// lookup against the operator's configuration and not a hard-coded provider.
// When nothing is configured for the domain, the screen says so plainly and
// offers the password instead — it never guesses.
//
// Non-purpose: no provider configuration (Settings, Security), no account
// linking UI, and no SSO for the portal, which is invite-only.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useMemo, useState } from "react";
import { Spinner } from "@marula-core/ui";
import { applyEntryTheme } from "../../lib/entry-theme";
import { resolveApiBaseUrl, saveTokens } from "../../lib/api";
import {
  EntryCard,
  EntryHeadline,
  EntryIcon,
  EntryShell,
} from "../../components/auth/entry";

const API_BASE = resolveApiBaseUrl();

interface PublicProvider {
  name: string;
  display_name: string;
}

function SsoInner() {
  const router = useRouter();
  const params = useSearchParams();
  const code = params.get("code");
  const callbackError = params.get("error");

  const [status, setStatus] = useState<"working" | "linked" | "error">("working");
  const [message, setMessage] = useState("Completing sign-in…");

  useEffect(() => {
    applyEntryTheme();
  }, []);

  useEffect(() => {
    if (callbackError !== null) {
      setStatus("error");
      setMessage(
        callbackError === "link_required"
          ? "Sign-in refused: this identity is not linked to a Marula account yet. An administrator has to link it first."
          : `Sign-in failed (${callbackError}). Nothing was changed.`,
      );
      return;
    }
    if (code === null) return;
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
            setMessage("Identity linked. You can close this window, or carry on.");
          }
          return;
        }
        saveTokens(body as unknown as Parameters<typeof saveTokens>[0]);
        window.location.assign("/today");
      } catch {
        if (!cancelled) {
          setStatus("error");
          setMessage(
            "This handoff code is invalid, used, or expired. Sign in again and your company account will pick up where it left off.",
          );
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [code, callbackError]);

  // The callback landing: status only, no form.
  if (code !== null || callbackError !== null) {
    return (
      <EntryShell background="indigo" fruit="none">
        <EntryCard testId="sso-card">
          <div className="entry-centred">
            {status === "working" && <Spinner label="Completing sign-in" />}
            {status === "linked" && (
              <span
                aria-hidden
                style={{
                  display: "flex",
                  width: 48,
                  height: 48,
                  alignItems: "center",
                  justifyContent: "center",
                  borderRadius: 999,
                  background: "var(--m-status-success-bg)",
                  color: "var(--m-status-success-fg)",
                }}
              >
                <EntryIcon name="check" size={22} />
              </span>
            )}
            {status === "error" && (
              <span
                aria-hidden
                style={{
                  display: "flex",
                  width: 48,
                  height: 48,
                  alignItems: "center",
                  justifyContent: "center",
                  borderRadius: 999,
                  // Blush, not red: a refused sign-in is not a breach, and the
                  // page that says so also says what to do next.
                  background: "var(--m-status-due-bg)",
                  color: "var(--m-status-due-fg)",
                }}
              >
                <EntryIcon name="alert" size={22} />
              </span>
            )}
            <h1 style={{ margin: 0, fontSize: 20, fontWeight: 600 }}>
              {status === "working"
                ? "Straight through the front door."
                : status === "linked"
                  ? "Identity linked."
                  : "That didn't go through."}
            </h1>
            <p
              role={status === "error" ? "alert" : "status"}
              className="entry-sub"
              style={status === "error" ? { color: "var(--m-status-due-fg)", fontWeight: 600 } : undefined}
            >
              {message}
            </p>
          </div>
          {status !== "working" && (
            <button
              type="button"
              className="entry-btn entry-btn--full"
              onClick={() => router.replace(status === "error" ? "/login" : "/today")}
            >
              {status === "error" ? "Back to sign in" : "Continue"}
            </button>
          )}
        </EntryCard>
      </EntryShell>
    );
  }

  return <SsoPicker />;
}

/** The pack's state 06: a domain in, the provider it belongs to out. */
function SsoPicker() {
  const router = useRouter();
  const [domain, setDomain] = useState("");
  const [providers, setProviders] = useState<PublicProvider[]>([]);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    applyEntryTheme();
    let cancelled = false;
    fetch(`${API_BASE}/api/v1/auth/oidc/providers/public`)
      .then((r) => (r.ok ? r.json() : []))
      .then((list: PublicProvider[]) => {
        if (!cancelled) {
          setProviders(list);
          setReady(true);
        }
      })
      .catch(() => {
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // A provider matches when its display name carries the domain, which is how
  // an operator names a preset for the company that owns it. A domain with no
  // match is reported as no match — never guessed at.
  const match = useMemo(() => {
    const clean = domain.trim().toLowerCase();
    if (clean === "" || providers.length === 0) return null;
    return (
      providers.find((p) => {
        const name = p.display_name.toLowerCase();
        const bare = clean.replace(/^https?:\/\//, "").split("/")[0] ?? clean;
        return name.includes(bare) || bare.includes(name);
      }) ?? null
    );
  }, [domain, providers]);

  const answered = domain.trim() !== "" && ready;

  return (
    <EntryShell background="indigo">
      <EntryCard testId="sso-card">
        <div>
          <EntryHeadline>Straight through the front door.</EntryHeadline>
          <p className="entry-sub">
          Sign in with your company account. Your admin set this up. We&apos;ll send you to your
          company&apos;s sign-in, then bring you back.
          </p>
        </div>

        <div className="entry-field">
          <label className="entry-label" htmlFor="sso-domain">
            Work email or company domain
          </label>
          <input
            id="sso-domain"
            className="entry-input"
            value={domain}
            onChange={(e) => setDomain(e.target.value)}
            placeholder="brightline.example"
            autoComplete="email"
            data-testid="sso-domain"
          />
          {answered && match !== null && (
            <p className="entry-hint" style={{ margin: 0 }} data-testid="sso-detected">
              We found <b style={{ color: "var(--m-text)" }}>{match.display_name}</b> for this
              domain.
            </p>
          )}
          {answered && match === null && (
            <p className="entry-hint" style={{ margin: 0 }} data-testid="sso-not-found">
              No company sign-in is set up for that domain. Use a password, or ask your admin to
              set one up.
            </p>
          )}
        </div>

        <a
          href={
            match === null
              ? "#"
              : `${API_BASE}/api/v1/auth/oidc/${match.name}/login`
          }
          aria-disabled={match === null}
          onClick={(e) => {
            if (match === null) e.preventDefault();
          }}
          style={{ display: "block", textDecoration: "none" }}
        >
          <button
            type="button"
            className="entry-btn entry-btn--full"
            disabled={match === null}
            data-testid="sso-continue"
          >
            <EntryIcon name="shield" />
            {match === null
              ? "Enter your company domain"
              : `Continue with ${match.display_name}`}
          </button>
        </a>

        <div style={{ textAlign: "center" }}>
          <button
            type="button"
            className="entry-link"
            style={{ fontWeight: 500, color: "var(--m-text-secondary)" }}
            onClick={() => router.push("/login")}
          >
            Use a password instead
          </button>
        </div>
      </EntryCard>
    </EntryShell>
  );
}

export default function SsoPage() {
  return (
    <Suspense fallback={<EntryShell background="indigo"><EntryCard /></EntryShell>}>
      <SsoInner />
    </Suspense>
  );
}
