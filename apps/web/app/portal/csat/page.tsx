// /portal/csat?token=… — the in-app CSAT response page (Wave 34 lane 2).
//
// Purpose: render the tokenized survey from the resolution email as an
// in-app form: one rating (1-5) plus an optional comment. The token is
// single-use and client-scoped — the route verifies the survey belongs to
// the signed-in portal user's client before anything renders.
//
// Honest states: an invalid, used, or expired link renders one plain
// explanation, never a form that pretends it might work.
//
// Non-purpose: score history (staff scorecards own that) and anonymous
// responses (the portal sign-in IS the attribution).
"use client";

import { Button, Card, EmptyState, Spinner } from "@meridian/ui";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState, type ReactNode } from "react";
import { portalApi, useRequirePortalAuth } from "../../../lib/portal-api";
import type { PortalCsatView } from "../../../lib/portal-api";

const SCALE = [1, 2, 3, 4, 5] as const;
const SCALE_LABEL: Record<number, string> = {
  1: "Poor",
  2: "Not great",
  3: "Okay",
  4: "Good",
  5: "Excellent",
};

export default function PortalCsatPage(): ReactNode {
  return (
    <Suspense
      fallback={
        <main className="flex min-h-full items-center justify-center p-6" style={{ background: "var(--m-canvas)" }}>
          <Spinner />
        </main>
      }
    >
      <PortalCsatInner />
    </Suspense>
  );
}

function PortalCsatInner() {
  useRequirePortalAuth();
  const params = useSearchParams();
  const token = params.get("token") ?? "";
  const [view, setView] = useState<PortalCsatView | null>(null);
  const [score, setScore] = useState<number | null>(null);
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [state, setState] = useState<"loading" | "invalid" | "ready">("loading");

  const load = useCallback(async () => {
    if (token === "") {
      setState("invalid");
      return;
    }
    try {
      const data = await portalApi.csatView(token);
      setView(data);
      setState("ready");
    } catch {
      setState("invalid");
    }
  }, [token]);

  useEffect(() => {
    void load();
  }, [load]);

  async function submit() {
    if (score === null) return;
    setBusy(true);
    setError(null);
    try {
      await portalApi.csatSubmit(token, score, comment);
      // The view re-read confirms the consume: the same token now reports
      // `responded`, which flips the page to the thank-you state.
      setView(await portalApi.csatView(token));
    } catch {
      setError("Your response could not be saved. The link may have expired.");
    } finally {
      setBusy(false);
    }
  }

  if (state === "loading") {
    return (
      <div className="grid place-items-center min-h-[50vh]">
        <Spinner />
      </div>
    );
  }

  if (state === "invalid" || view === null) {
    return (
      <div className="grid place-items-center min-h-[50vh] p-4">
        <EmptyState
          message="This survey link is invalid, used, or expired. Each link works once and only within its validity window — open it again from the original email."
        />
      </div>
    );
  }

  if (view.status === "responded") {
    return (
      <div className="grid place-items-center min-h-[60vh] p-4">
        <Card className="p-6 grid gap-2 max-w-lg w-full" data-testid="csat-thanks">
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            Thank you — your rating is recorded.
          </h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            Ticket {view.ticket_number} · {view.ticket_subject}
          </p>
        </Card>
      </div>
    );
  }

  return (
    <div className="grid place-items-center min-h-[60vh] p-4">
      <Card className="p-6 grid gap-5 max-w-lg w-full">
        <div className="grid gap-1">
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            Ticket {view.ticket_number} · {view.ticket_subject}
          </p>
          <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
            How did we do?
          </h1>
        </div>

        <div role="radiogroup" aria-label="Satisfaction rating" className="flex gap-2">
          {SCALE.map((value) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={score === value}
              aria-label={`${value} — ${SCALE_LABEL[value]}`}
              onClick={() => setScore(value)}
              disabled={busy}
              data-testid={`csat-score-${value}`}
              className="h-11 w-11 rounded-[var(--m-radius-control)] border text-sm font-semibold transition-[transform,opacity,border-color] duration-[120ms] hover:-translate-y-px motion-reduce:transform-none motion-reduce:hover:translate-y-0 disabled:opacity-50"
              style={{
                borderColor: score === value ? "var(--m-accent)" : "var(--m-border)",
                color: score === value ? "var(--m-accent)" : "var(--m-text)",
              }}
            >
              {value}
            </button>
          ))}
        </div>
        <div className="flex justify-between text-xs" style={{ color: "var(--m-text-muted)" }}>
          <span>Poor</span>
          <span>Excellent</span>
        </div>
        {score !== null && (
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            {SCALE_LABEL[score]}
          </p>
        )}

        <label className="grid gap-1">
          <span className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
            Anything to add? (optional)
          </span>
          <textarea
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            rows={3}
            maxLength={2000}
            disabled={busy}
            data-testid="csat-comment"
            className="rounded-[var(--m-radius-control)] border p-2 text-[13px]"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          />
        </label>

        {error && (
          <p role="alert" className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            {error}
          </p>
        )}

        <Button onClick={submit} disabled={score === null || busy} data-testid="csat-submit">
          {busy ? "Sending…" : "Send response"}
        </Button>
      </Card>
    </div>
  );
}
