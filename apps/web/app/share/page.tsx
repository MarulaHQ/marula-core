// /share?token=… — the one-time share-link claim surface.
//
// Purpose (Wave 19 lane 3): the consumer half of a one-time share link. A team
// member opens a link composed by the sharer, the page claims it with their own
// JWT, and the material is shown exactly once with a copy-once affordance.
// Claiming consumes the link permanently, server-side; every lifecycle step is
// audited.
//
// THE DERIVATION (DERIVATION-GUIDE.md lines 90-91). This route is DERIVED from
// the KIT with no screen of its own, which leaves one instruction: no chrome, no
// portal header, no staff nav — and, on these two surfaces, no inverted surface
// and no red. So this is a plain card on the canvas with the material in it and
// nothing else around it. This is a read-only view with a one-time copy, which
// is the whole shape: nothing here can be edited, and nothing here stays.
//
// THE POSTURE IS THE POINT. A shared secret reveals only to members of the
// sharing organization who hold reveal rights, and that is enforced on the claim
// itself, not on this page. So the signed-out state says the truth about the
// gate instead of pretending the link is broken, and a refused claim gives the
// server's reason in plain words rather than a shrug.
"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState } from "react";
import { applyTheme, Spinner } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { api, useRequireAuth } from "../../lib/api";
import "../share.css";

type ShareClaimMaterial = components["schemas"]["ShareClaimMaterial"];

type Phase =
  | { kind: "loading" }
  | { kind: "claimed"; material: ShareClaimMaterial }
  | { kind: "rejected"; reason: string }
  | { kind: "signed-out" };

function ClaimInner() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  const [copied, setCopied] = useState<string | null>(null);
  const token = useSearchParams().get("token") ?? "";

  useEffect(() => {
    // Public page: always render in the light theme, never inherit a staff
    // member's persisted preference on a shared device.
    applyTheme("light");
  }, []);

  const claim = useCallback(async () => {
    if (token === "") {
      setPhase({ kind: "rejected", reason: "No claim token was in the link." });
      return;
    }
    setPhase({ kind: "loading" });
    const { data, error, response } = await api.api.POST("/api/v1/secrets/share-links/claim", {
      body: { token },
    });
    if (data !== undefined && error === undefined) {
      setPhase({ kind: "claimed", material: data });
      return;
    }
    const status = response.status;
    setPhase({
      kind: "rejected",
      reason:
        status === 409
          ? "This link was already claimed once, or the sender withdrew it, so the material is closed to claims."
          : status === 403
            ? "Your role cannot reveal this material. It needs category edit access in the sharing organization."
            : status === 404
              ? "This link is unknown to you, or it has expired."
              : "The claim was refused. Ask the person who shared it to send a new link.",
    });
  }, [token]);

  useEffect(() => {
    if (auth.status === "authed") void claim();
    else if (auth.status === "anonymous") setPhase({ kind: "signed-out" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auth.status]);

  if (auth.status === "anonymous" || phase.kind === "signed-out") {
    return (
      <div className="share-root">
        <div className="share-card" data-testid="share-signed-out">
          <h1 className="share-card__h">Sign in to claim this link.</h1>
          <p className="share-card__p">
            A shared secret reveals only to members of the organization that shared it, and only to
            people who hold reveal rights. That is checked when the link is claimed, not here.
          </p>
          <button
            type="button"
            className="entry-btn"
            onClick={() => router.replace("/login")}
          >
            Go to sign in
          </button>
        </div>
      </div>
    );
  }

  if (phase.kind === "loading") {
    return (
      <div className="share-root">
        <div className="share-card">
          <Spinner aria-label="Claiming the one-time link" />
          <p className="share-card__p">Claiming the one-time link…</p>
        </div>
      </div>
    );
  }

  if (phase.kind === "rejected") {
    return (
      <div className="share-root">
        <div className="share-card" data-testid="share-rejected">
          <h1 className="share-card__h">This link isn&apos;t claimable.</h1>
          <p className="share-card__p">{phase.reason}</p>
        </div>
      </div>
    );
  }

  const { material } = phase;
  return (
    <div className="share-root">
      <div className="share-card" data-testid="share-claimed">
        <div>
          <span className="share-desk">
            {material.subject_kind === "password" ? "Credential" : "Secret record"}
          </span>
          <h1 className="share-card__h">{material.title}</h1>
          <p className="share-card__p">
            Revealed by a one-time link. The share is consumed: this link cannot claim again, and
            the server has recorded that it was used.
          </p>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          {material.fields.map((f) => (
            <div key={f.field_key} className="entry-field">
              <span className="entry-label">{f.field_key.replaceAll("_", " ")}</span>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <code
                  style={{
                    minWidth: 0,
                    flex: 1,
                    overflowWrap: "anywhere",
                    border: "1px solid var(--m-border)",
                    borderRadius: 12,
                    padding: "10px 12px",
                    fontFamily: "var(--m-font-mono)",
                    fontSize: 13,
                  }}
                >
                  {f.value}
                </code>
                <button
                  type="button"
                  className="entry-btn entry-btn--secondary entry-btn--small"
                  onClick={() => {
                    void navigator.clipboard.writeText(f.value);
                    setCopied(f.field_key);
                  }}
                  data-testid={`share-copy-${f.field_key}`}
                >
                  {copied === f.field_key ? "Copied" : "Copy"}
                </button>
              </div>
            </div>
          ))}
        </div>

        <p className="share-card__p" style={{ color: "var(--m-text-secondary)" }}>
          Copy what you need now. When you close this page, it is gone &mdash; and asking for
          another copy means asking the person who shared it.
        </p>
      </div>
    </div>
  );
}

export default function ShareClaimPage() {
  return (
    <Suspense
      fallback={
        <div className="share-root">
          <div className="share-card">
            <Spinner />
          </div>
        </div>
      }
    >
      <ClaimInner />
    </Suspense>
  );
}
