// /welcome — Tier-1 org first-run wizard (docs/10-agent-platform.md §3).
//
// Purpose: a skippable, idempotent two-step flow: org profile (name +
// ticket prefix) → done. Skipping sets the completion flag and changes
// NOTHING else — defaults-first doctrine means the org already operates
// untouched. Re-running the wizard overwrites the same fields (idempotent).
//
// Non-purpose: business-hours calendar / default board / admin invite
// steps are later-tier scope; this skeleton ships the flow mechanics.
"use client";

import { friendlyMutationText } from "../../lib/error-copy";
import { Button, Card, FieldHint, Input, Label } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

const WIZARD_KEY = "onboarding.wizard_completed";

export default function WelcomeWizardPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [step, setStep] = useState<1 | 2>(1);
  const [name, setName] = useState("");
  const [ticketPrefix, setTicketPrefix] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      // Prefill from the live profile so re-runs are true edits.
      const { data } = await api.api.GET("/api/v1/org");
      if (data) {
        setName(data.name);
        setTicketPrefix(data.ticket_prefix);
      }
    })();
  }, [auth.status]);

  /** Persists the completion flag; idempotent under replay. */
  const completeWizard = async () => {
    const res = await api.api.PUT("/api/v1/settings/org", {
      body: { key: WIZARD_KEY, value: true },
    });
    if (res.error) {
      const detail =
        (res.error as { error?: { message?: string } }).error?.message ??
        "rejected";
      throw new Error(detail);
    }
  };

  const saveProfile = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await api.api.PATCH("/api/v1/org", {
        body: {
          name: name.trim(),
          ticket_prefix: ticketPrefix.trim().toUpperCase(),
        },
      });
      if (res.error) {
        const detail =
          (res.error as { error?: { message?: string } }).error?.message ??
          "rejected";
        throw new Error(`Org profile ${detail}`);
      }
      await completeWizard();
      setStep(2);
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setBusy(false);
    }
  };

  const skip = async () => {
    setBusy(true);
    setError(null);
    try {
      await completeWizard();
      router.replace("/today");
    } catch (e) {
      setError(friendlyMutationText(e));
      setBusy(false);
    }
  };

  return (
    <div
      className="min-h-screen flex flex-col"
      style={{ background: "var(--m-canvas)" }}
    >
      <main className="flex-1 w-full max-w-xl mx-auto px-6 py-16 grid gap-6 content-start">
        <header className="grid gap-1">
          <p
            className="text-[12px] uppercase tracking-wide"
            style={{ color: "var(--m-text-muted)" }}
          >
            Welcome to Meridian · step {step} of 2
          </p>
          <h1
            className="text-lg font-semibold"
            style={{ color: "var(--m-text)" }}
          >
            {step === 1 ? "Set up your organization" : "You're all set"}
          </h1>
          <p
            className="text-[13px]"
            style={{ color: "var(--m-text-secondary)" }}
          >
            {step === 1
              ? "Everything below already works with sensible defaults — this just personalizes it."
              : "Your organization runs on Meridian defaults. Tune them anytime in Settings."}
          </p>
        </header>

        {step === 1 && (
          <Card className="p-5 grid gap-3">
            <label className="grid gap-1">
              <Label>Organization name</Label>
              <Input value={name} onChange={(e) => setName(e.target.value)} />
            </label>
            <label className="grid gap-1">
              <Label>Ticket prefix</Label>
              <Input
                value={ticketPrefix}
                maxLength={8}
                onChange={(e) => setTicketPrefix(e.target.value.toUpperCase())}
                className="w-32"
              />
              <FieldHint>Prepended to every ticket number.</FieldHint>
            </label>

            {(error || null) && (
              <span
                className="text-[12px]"
                role="alert"
                style={{ color: "var(--m-status-danger-fg)" }}
              >
                {error}
              </span>
            )}

            <div className="flex items-center gap-2 mt-1">
              <Button
                variant="primary"
                onClick={() => void saveProfile()}
                disabled={busy || !name.trim()}
              >
                Save and continue
              </Button>
              <Button
                variant="ghost"
                onClick={() => void skip()}
                disabled={busy}
              >
                Skip for now
              </Button>
            </div>
            <FieldHint>
              Skipping keeps every default exactly as shipped — nothing breaks.
            </FieldHint>
          </Card>
        )}

        {step === 2 && (
          <Card className="p-5 grid gap-3 justify-items-start">
            <p
              className="text-[13px]"
              style={{ color: "var(--m-text-secondary)" }}
            >
              Profile saved. Boards, statuses, roles, and SLA policies came
              pre-configured with industry-standard defaults.
            </p>
            <Button variant="primary" onClick={() => router.replace("/today")}>
              Enter Meridian
            </Button>
          </Card>
        )}
      </main>
    </div>
  );
}
