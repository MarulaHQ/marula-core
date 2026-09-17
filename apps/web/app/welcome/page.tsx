// /welcome — Tier-1 org first-run wizard (docs/10-agent-platform.md §3),
// Wave 17 lane 1 polish: the three-tier onboarding presented honestly.
//
// Purpose: a skippable, idempotent flow. Tier 1 (org profile: name +
// ticket prefix) is fully wired — PATCH /org + the completion flag.
// Tiers 2 (module activation) and 3 (guided playbooks) are backend-scoped
// waves, so they render as typed posture panels — clearly labeled "Not
// available yet", with no fake controls (whole features ship end-to-end
// OR visibly absent). Skipping sets the completion flag and changes
// NOTHING else — defaults-first doctrine means the org already operates
// untouched. Re-running the wizard overwrites the same fields (idempotent).
//
// Non-purpose: business-hours calendar / default board / admin invite
// steps are later-tier scope; no module activation or playbook runtime here.
"use client";

import { friendlyMutationText } from "../../lib/error-copy";
import { Button, Card, FieldHint, Input, Label, Spinner, Tag } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

const WIZARD_KEY = "onboarding.wizard_completed";

const MODULE_TIERS = [
  {
    name: "Dispatch",
    blurb: "Skills-based routing and the dispatch queue.",
  },
  {
    name: "Client portal",
    blurb: "Clients raise, track, and reply to tickets.",
  },
  {
    name: "Billing & payments",
    blurb: "Invoices, statements, and online payment.",
  },
  {
    name: "Agent platform",
    blurb: "Skills, plugins, and the MCP tool surface.",
  },
] as const;

export default function WelcomeWizardPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [step, setStep] = useState<1 | 2 | 3>(1);
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

  if (auth.status !== "authed") {
    return (
      <div
        className="flex min-h-screen items-center justify-center"
        style={{ background: "var(--m-canvas)" }}
        aria-busy="true"
      >
        <Spinner label="Loading welcome" />
      </div>
    );
  }

  return (
    <div
      className="flex min-h-screen flex-col"
      style={{ background: "var(--m-canvas)" }}
    >
      <main className="mx-auto grid w-full max-w-xl content-start gap-6 px-6 py-14">
        <header className="m-rise grid gap-2">
          {/* Tier progress: three quiet segments, current one accented */}
          <div className="flex items-center gap-1.5" aria-hidden="true">
            {[1, 2, 3].map((s) => (
              <span
                key={s}
                className="h-1 flex-1 rounded-full transition-colors duration-100"
                style={{
                  background:
                    s < step
                      ? "var(--m-accent)"
                      : s === step
                        ? "var(--m-accent)"
                        : "var(--m-border)",
                  opacity: s === step ? 1 : s < step ? 0.55 : 1,
                }}
              />
            ))}
          </div>
          <p
            className="text-[11px] font-medium uppercase tracking-wide"
            style={{ color: "var(--m-text-muted)" }}
          >
            Welcome to Meridian · step {step} of 3
          </p>
          <h1 className="text-xl font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
            {step === 1
              ? "Set up your organization"
              : step === 2
                ? "Activate modules"
                : "Guided playbooks"}
          </h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {step === 1
              ? "Everything below already works with sensible defaults — this just personalizes it."
              : step === 2
                ? "The modules below arrive with their platform waves. Nothing to configure here yet."
                : "Day-to-day playbooks walk your team through common flows. They land with the agent platform."}
          </p>
        </header>

        {step === 1 && (
          <Card className="m-rise grid gap-3 p-5">
            <div className="grid gap-1">
              <Label htmlFor="org-name">Organization name</Label>
              <Input
                id="org-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="org-prefix">Ticket prefix</Label>
              <Input
                id="org-prefix"
                value={ticketPrefix}
                maxLength={8}
                onChange={(e) => setTicketPrefix(e.target.value.toUpperCase())}
                className="w-32 font-mono"
              />
              <FieldHint>Prepended to every ticket number.</FieldHint>
            </div>

            {(error || null) && (
              <span
                className="text-[12px]"
                role="alert"
                style={{ color: "var(--m-status-danger-fg)" }}
              >
                {error}
              </span>
            )}

            <div className="mt-1 flex items-center gap-2">
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
          <div className="grid gap-4">
            <Card className="m-rise grid gap-3 p-5" style={{ animationDelay: "30ms" }}>
              <ul className="grid gap-2.5">
                {MODULE_TIERS.map((m) => (
                  <li key={m.name} className="flex items-center gap-3">
                    <div className="grid min-w-0 gap-0.5">
                      <span className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                        {m.name}
                      </span>
                      <span className="truncate text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                        {m.blurb}
                      </span>
                    </div>
                    <span className="ml-auto shrink-0">
                      <Tag tone="outline" style={{ border: "1px solid var(--m-border)" }}>Not available yet</Tag>
                    </span>
                  </li>
                ))}
              </ul>
              {(error || null) && (
                <span
                  className="text-[12px]"
                  role="alert"
                  style={{ color: "var(--m-status-danger-fg)" }}
                >
                  {error}
                </span>
              )}
              <div className="flex items-center gap-2">
                <Button variant="primary" onClick={() => setStep(3)} disabled={busy}>
                  Continue
                </Button>
                <Button variant="ghost" onClick={() => void skip()} disabled={busy}>
                  Skip for now
                </Button>
              </div>
            </Card>
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Boards, statuses, roles, and SLA policies came pre-configured with
              industry-standard defaults — tune them anytime in Settings.
            </p>
          </div>
        )}

        {step === 3 && (
          <Card className="m-rise grid gap-3 justify-items-start p-5" style={{ animationDelay: "30ms" }}>
            <div className="flex w-full items-center gap-2">
              <span className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                Playbook runtime
              </span>
              <span className="ml-auto">
                <Tag tone="outline" style={{ border: "1px solid var(--m-border)" }}>Not available yet</Tag>
              </span>
            </div>
            <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              Walkthroughs for triage, dispatch, and time review once
              the agent platform lands. Your workbench is ready today.
            </p>
            {(error || null) && (
              <span
                className="text-[12px]"
                role="alert"
                style={{ color: "var(--m-status-danger-fg)" }}
              >
                {error}
              </span>
            )}
            <Button variant="primary" onClick={() => router.replace("/today")}>
              Enter Meridian
            </Button>
          </Card>
        )}
      </main>
    </div>
  );
}
