// /share?token=… — the one-time share-link claim surface.
//
// Purpose (Wave 19 lane 3): the consumer half of the one-time share
// links. A team member opens the link (composed by the sharer with
// `link_path`), the page claims `POST /api/v1/secrets/share-links/claim`
// with their own JWT — the same reveal ladder a normal reveal rides
// (org membership, secrets/passwords edit permission, category-grant
// gate) — and shows the material exactly once with a copy-once affordance.
// Every lifecycle step is audited server-side; claiming consumes the
// link permanently (409 thereafter).
//
// Posture: light plain language, Workbench tokens only, honest states —
// expired / revoked / already claimed / foreign org / unknown all read
// as one friendly "not claimable" row plus the typed reason the server
// returned. Not inside the authed shell; it renders its own authed
// posture via the shared guard.

"use client";

import { Button, Card, Spinner } from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

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
  const token =
    new URLSearchParams(typeof window === "undefined" ? "" : window.location.search)
      .get("token") ?? "";

  const claim = useCallback(async () => {
    if (token === "") {
      setPhase({ kind: "rejected", reason: "No claim token was provided." });
      return;
    }
    setPhase({ kind: "loading" });
    const path = "/api/v1/secrets/share-links/claim" as const;
    const { data, error, response } = await api.api.POST(path, {
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
          ? "This link was already claimed once — or it was revoked — so the material is closed to claims."
          : status === 403
            ? "Your role cannot reveal this material — category edit access is required."
            : status === 404
              ? "This link is unknown to you, expired, or gone."
              : "The claim was refused — ask the sharer to re-issue the link.",
    });
  }, [token]);

  useEffect(() => {
    if (auth.status === "authed") void claim();
    else if (auth.status === "anonymous") setPhase({ kind: "signed-out" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auth.status]);

  if (auth.status === "anonymous" || phase.kind === "signed-out") {
    return (
      <Card className="m-auto max-w-md">
        <div className="grid gap-3 p-6 text-center">
          <p className="text-[13px]">Sign in to claim this share link.</p>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            A shared secret reveals only to members of the sharing org who hold
            reveal rights — that is the honest posture, and it is enforced on
            the claim itself, not here.
          </p>
          <Button variant="secondary" onClick={() => router.replace("/login")}>
            Go to sign in
          </Button>
        </div>
      </Card>
    );
  }

  if (phase.kind === "loading") {
    return (
      <Card className="m-auto max-w-md">
        <div className="grid gap-3 p-6 text-center">
          <Spinner />
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Claiming the one-time link…
          </p>
        </div>
      </Card>
    );
  }

  if (phase.kind === "rejected") {
    return (
      <Card className="m-auto max-w-md">
        <div className="grid gap-3 p-6">
          <p className="text-sm font-semibold">This link is not claimable.</p>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {phase.reason}
          </p>
        </div>
      </Card>
    );
  }

  const { material } = phase;
  return (
    <Card className="m-auto max-w-md">
      <div className="grid gap-3 p-6">
        <div>
          <h1 className="text-sm font-semibold">{material.title}</h1>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            {material.subject_kind === "password" ? "Credential" : "Secret record"} revealed by a
            one-time link — the share is consumed; this link cannot claim again.
          </p>
        </div>
        <ul className="grid gap-2">
          {material.fields.map((f) => (
            <li key={f.field_key} className="grid gap-1">
              <span className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                {f.field_key.replaceAll("_", " ")}
              </span>
              <div className="flex items-center gap-2">
                <code className="min-w-0 flex-1 break-all rounded-[0.5rem] border px-3 py-2 font-mono text-[13px]" style={{ borderColor: "var(--m-border)" }}>
                  {f.value}
                </code>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => {
                    void navigator.clipboard.writeText(f.value);
                    setCopied(f.field_key);
                  }}
                >
                  {copied === f.field_key ? "Copied" : "Copy"}
                </Button>
              </div>
            </li>
          ))}
        </ul>
      </div>
    </Card>
  );
}

export default function ShareClaimPage() {
  return (
    <div className="grid min-h-screen place-items-center p-6" style={{ background: "var(--m-canvas)" }}>
      <Suspense
        fallback={
          <Card className="m-auto max-w-md">
            <div className="grid gap-3 p-6">
              <Spinner />
            </div>
          </Card>
        }
      >
        <ClaimInner />
      </Suspense>
    </div>
  );
}
