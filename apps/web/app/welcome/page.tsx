// /welcome — the Wave 26 org onboarding wizard (owner/admin first-run).
//
// Purpose: one resumable, one-decision-per-step setup flow REPLACING the
// old Tier-1 static wizard: clients → board → tickets → invites →
// (optional) feeder connector → (optional) AI surfaces. EVERY step is
// independently skippable with "Skip for now" (records nothing — the step
// stays pending and resumable) AND "Never show this again" (persists the
// dismissed flag server-side; never resets on reload, never silently
// re-nags). Step completion is SERVER-GATED: the artifact must exist
// before the done flag lands (POST .../complete 409s until satisfied).
// Wizard state rides the typed settings registry (core-domain Wave 26
// keys); the flat legacy `onboarding.wizard_completed` key is honored as
// a fallback on read (U6-safe supersede).
//
// Non-purpose: member-level first-run lives at /onboarding; the
// re-run/disable controls live in Settings → Onboarding; the dismissible
// resume banner lives on /today.

"use client";

import type { components } from "@marula-core/sdk";
import { Button, Card, Input, Tag } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import {
  ORG_STEP_LABELS,
  ORG_STEP_ORDER,
  completeOrgStep,
  dismissOrgStep,
  fetchOnboardingSummary,
  skipOrgStep,
  type OnboardingSummary,
} from "../../lib/onboarding";
import { StepChecklist } from "../../components/onboarding/step-checklist";
import { StepActions } from "../../components/onboarding/step-actions";

type Client = components["schemas"]["Client"];
type Board = components["schemas"]["Board"];

const ROLES = [
  ["admin", "Admin"],
  ["dispatcher", "Dispatcher"],
  ["senior_tech", "Senior tech"],
  ["technician", "Technician"],
  ["bookkeeper", "Bookkeeper"],
  ["auditor", "Auditor"],
  ["read_only", "Read only"],
] as const;

export default function WelcomeWizardPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [summary, setSummary] = useState<OnboardingSummary | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // create-client draft
  const [clientName, setClientName] = useState("");
  // create-board draft
  const [boardName, setBoardName] = useState("");
  // create-ticket drafts
  const [ticketTitle, setTicketTitle] = useState("");
  // invite drafts
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteName, setInviteName] = useState("");
  const [inviteRole, setInviteRole] = useState<string>("technician");
  const [inviteLink, setInviteLink] = useState<string | null>(null);

  const loadSummary = useCallback(async (): Promise<OnboardingSummary | null> => {
    const s = await fetchOnboardingSummary();
    if (s) setSummary(s);
    return s;
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void (async () => {
      const s = await loadSummary();
      const first = s?.org.steps.find((st) => !st.resolved)?.step ?? null;
      setFocus(first ?? ORG_STEP_ORDER[5]);
    })();
  }, [auth.status, loadSummary]);

  if (auth.status !== "authed") {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <p className="text-sm" style={{ color: "var(--m-text-muted)" }}>
          Loading welcome…
        </p>
      </div>
    );
  }

  const steps = summary?.org.steps ?? [];
  const stepView = (key: string) => steps.find((s) => s.step === key);
  const resolvedCount = steps.filter((s) => s.resolved).length;
  const allResolved = steps.length > 0 && resolvedCount === steps.length;

  /** Any successful step mutation → refresh gates + advance focus. */
  const refresh = async (nextSafe?: string) => {
    await loadSummary();
    if (nextSafe) setFocus(nextSafe);
  };

  /** Server-gated done: POST complete; 409s are surfaced honestly. */
  const markComplete = async (step: string) => {
    setError(null);
    setBusy(true);
    try {
      const { error: apiError } = await completeOrgStep(step);
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const recordSkip = async (step: string, never: boolean) => {
    setError(null);
    setBusy(true);
    try {
      const res = never ? await dismissOrgStep(step) : await skipOrgStep(step);
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      const idx = ORG_STEP_ORDER.indexOf(step as (typeof ORG_STEP_ORDER)[number]);
      await refresh(never ? advance(idx) : advance(idx));
    } finally {
      setBusy(false);
    }
  };

  const advance = (idx: number): string => {
    const next = ORG_STEP_ORDER[Math.min(idx + 1, ORG_STEP_ORDER.length - 1)];
    return typeof next === "string" ? next : ORG_STEP_ORDER[0];
  };

  // --- step bodies ------------------------------------------------------------

  const createClient = async () => {
    setError(null);
    setBusy(true);
    try {
      const name = clientName.trim();
      const { error: apiError } = await api.api.POST("/api/v1/clients", {
        body: { name } as never,
      });
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      setClientName("");
      // The gate is server-side: satisfied flips only after the artifact
      // exists; completing records it right away.
      const res = await completeOrgStep("clients");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh("board");
    } finally {
      setBusy(false);
    }
  };

  const createBoard = async () => {
    setError(null);
    setBusy(true);
    try {
      const name = boardName.trim();
      const { error: apiError } = await api.api.POST("/api/v1/boards", {
        body: { name, is_default: true },
      });
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      setBoardName("");
      const res = await completeOrgStep("board");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh("tickets");
    } finally {
      setBusy(false);
    }
  };

  const createTicket = async () => {
    setError(null);
    setBusy(true);
    try {
      const title = ticketTitle.trim();
      const { error: apiError } = await api.api.POST("/api/v1/tickets", {
        body: { title, priority: "p3" } as never,
      });
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      setTicketTitle("");
      const res = await completeOrgStep("tickets");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh("invites");
    } finally {
      setBusy(false);
    }
  };

  const sendInvite = async () => {
    setError(null);
    setBusy(true);
    try {
      const { data: inviteData, error: apiError } = await api.api.POST(
        "/api/v1/org/users",
        {
          body: {
            email: inviteEmail.trim(),
            display_name: inviteName.trim(),
            role: inviteRole,
          } as never,
        },
      );
      if (apiError !== undefined || inviteData === undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      const setup = inviteSetupToken(inviteData);
      if (setup) {
        setInviteLink(`${window.location.origin}/accept-invite?token=${setup}`);
      }
      setInviteEmail("");
      setInviteName("");
      const res = await completeOrgStep("invites");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh("feeder");
    } finally {
      setBusy(false);
    }
  };

  const enableAi = async () => {
    setError(null);
    setBusy(true);
    try {
      const { error: apiError } = await api.api.PUT("/api/v1/settings/org", {
        body: { key: "ai.features_enabled", value: true },
      });
      if (apiError !== undefined) {
        setError(friendlyMutationText(apiError ?? null));
        return;
      }
      const res = await completeOrgStep("ai");
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  return (
    <main
      className="mx-auto grid min-h-screen max-w-4xl content-start gap-5 p-4 md:p-8"
      style={{ background: "var(--m-canvas)" }}
    >
      <header className="m-rise grid gap-1">
        <p
          className="text-[10px] font-semibold uppercase tracking-wide"
          style={{ color: "var(--m-text-muted)" }}
        >
          Setup wizard
        </p>
        <h1 className="text-lg font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
          Finish setting up your organization
        </h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {resolvedCount} of {ORG_STEP_ORDER.length} steps resolved. Every step is skippable —
          skipping never resets, and defaults already run your org.
        </p>
      </header>

      <div className="grid gap-5 md:grid-cols-[240px_1fr] md:items-start">
        <Card className="p-3">
          <StepChecklist
            steps={steps}
            labels={ORG_STEP_LABELS}
            current={focus}
            onSelect={(s) => setFocus(s)}
          />
        </Card>

        <div className="grid gap-4">
          {summary?.org.disabled && (
            <Card className="p-3 text-[13px]" data-testid="onboarding-disabled-note">
              Org-wide onboarding is currently disabled in Settings → Onboarding. Re-enable it to
              surface these prompts for new members — your progress is kept either way.
            </Card>
          )}

          {error && (
            <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          )}

          {focus === "clients" && (
            <StepCard step="clients" summaryReady={summary !== null} onSkip={(n) => void recordSkip("clients", n)} busy={busy}>
              <div className="grid gap-2">
                <Input
                  value={clientName}
                  onChange={(e) => setClientName(e.target.value)}
                  placeholder="Client name"
                  aria-label="Client name"
                  data-testid="wizard-client-name"
                />
                <div className="flex items-center gap-2">
                  <Button variant="primary" onClick={() => void createClient()} disabled={busy || !clientName.trim()}>
                    Create client
                  </Button>
                  {stepView("clients")?.satisfied && (
                    <Button onClick={() => void markComplete("clients")} disabled={busy}>
                      Mark done
                    </Button>
                  )}
                </div>
                {stepView("clients")?.satisfied && !stepView("clients")?.done && (
                  <p className="text-[12px]" style={{ color: "var(--m-status-warning-fg)" }}>
                    This org already has clients — you can mark the step done.
                  </p>
                )}
              </div>
            </StepCard>
          )}

          {focus === "board" && (
            <StepCard step="board" summaryReady={summary !== null} onSkip={(n) => void recordSkip("board", n)} busy={busy}>
              <div className="grid gap-2">
                <Input
                  value={boardName}
                  onChange={(e) => setBoardName(e.target.value)}
                  placeholder="Board name (e.g. Service desk)"
                  aria-label="Board name"
                  data-testid="wizard-board-name"
                />
                <Button variant="primary" onClick={() => void createBoard()} disabled={busy || !boardName.trim()}>
                  Create board with default statuses
                </Button>
                {stepView("board")?.satisfied && !stepView("board")?.done && (
                  <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    Boards already exist in this org — you can mark the step done instead.
                  </p>
                )}
              </div>
            </StepCard>
          )}

          {focus === "tickets" && (
            <StepCard step="tickets" summaryReady={summary !== null} onSkip={(n) => void recordSkip("tickets", n)} busy={busy}>
              <div className="grid gap-2">
                <Input
                  value={ticketTitle}
                  onChange={(e) => setTicketTitle(e.target.value)}
                  placeholder="Ticket title"
                  aria-label="Ticket title"
                  data-testid="wizard-ticket-title"
                />
                <Button variant="primary" onClick={() => void createTicket()} disabled={busy || !ticketTitle.trim()}>
                  Create ticket
                </Button>
                {stepView("tickets")?.satisfied && !stepView("tickets")?.done && (
                  <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    Tickets already exist — mark the step done when you like.
                  </p>
                )}
              </div>
            </StepCard>
          )}

          {focus === "invites" && (
            <StepCard step="invites" summaryReady={summary !== null} onSkip={(n) => void recordSkip("invites", n)} busy={busy}>
              <div className="grid gap-2">
                <div className="grid gap-2 sm:grid-cols-2">
                  <Input
                    value={inviteName}
                    onChange={(e) => setInviteName(e.target.value)}
                    placeholder="Full name"
                    aria-label="Invitee full name"
                    data-testid="wizard-invite-name"
                  />
                  <Input
                    value={inviteEmail}
                    onChange={(e) => setInviteEmail(e.target.value)}
                    placeholder="Email"
                    aria-label="Invitee email"
                    type="email"
                    data-testid="wizard-invite-email"
                  />
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <select
                    aria-label="Role"
                    value={inviteRole}
                    onChange={(e) => setInviteRole(e.target.value)}
                    className="rounded px-2 py-1.5 text-[13px]"
                    style={{ background: "var(--m-canvas-raised)", color: "var(--m-text)", border: "1px solid var(--m-border)" }}
                    data-testid="wizard-invite-role"
                  >
                    {ROLES.map(([id, label]) => (
                      <option key={id} value={id}>
                        {label}
                      </option>
                    ))}
                  </select>
                  <Button variant="primary" onClick={() => void sendInvite()} disabled={busy || !inviteEmail.trim() || !inviteName.trim()}>
                    Send invite
                  </Button>
                </div>
                {inviteLink && (
                  <Card className="grid gap-1 p-3" data-testid="wizard-invite-link">
                    <span className="text-[12px] font-medium" style={{ color: "var(--m-text)" }}>
                      One-time setup link (shown once, copy now):
                    </span>
                    <code className="break-all text-[12px]" style={{ color: "var(--m-text-secondary)", fontFamily: "var(--m-font-mono)" }}>
                      {inviteLink}
                    </code>
                  </Card>
                )}
                {stepView("invites")?.satisfied && !stepView("invites")?.done && (
                  <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    Invites already issued — mark the step done when you like.
                  </p>
                )}
              </div>
            </StepCard>
          )}

          {focus === "feeder" && (
            <StepCard step="feeder" summaryReady={summary !== null} onSkip={(n) => void recordSkip("feeder", n)} busy={busy}>
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Feeder connectors (NinjaOne / IT Glue / Auvix) are configured from Imports with
                machine API keys; the step completes server-side once a feed has actually written
                records.
              </p>
              <div className="flex items-center gap-2">
                <Button onClick={() => router.push("/imports")}>Go to Imports</Button>
                {stepView("feeder")?.satisfied && !stepView("feeder")?.done && (
                  <Button variant="primary" onClick={() => void markComplete("feeder")} disabled={busy}>
                    Mark done
                  </Button>
                )}
              </div>
              {stepView("feeder")?.satisfied && !stepView("feeder")?.done && (
                <p className="text-[12px]" style={{ color: "var(--m-status-warning-fg)" }}>
                  A feeder wrote records into this org — mark done or let the banner note it.
                </p>
              )}
            </StepCard>
          )}

          {focus === "ai" && (
            <StepCard step="ai" summaryReady={summary !== null} onSkip={(n) => void recordSkip("ai", n)} busy={busy}>
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Turning this on enables drafting, booking and merge-assist surfaces org-wide. It
                stays honest: nothing calls a model until a provider is configured.
              </p>
              <div className="flex items-center gap-2">
                <Button variant="primary" onClick={() => void enableAi()} disabled={busy}>
                  Enable AI surfaces
                </Button>
                <Button variant="ghost" onClick={() => setFocus(advance(ORG_STEP_ORDER.indexOf("ai")))} disabled={busy}>
                  Decide later
                </Button>
              </div>
            </StepCard>
          )}

          {allResolved && (
            <Card className="m-rise grid gap-2 p-5" data-testid="wizard-complete">
              <div className="flex items-center gap-2">
                <span className="text-[13px] font-semibold" style={{ color: "var(--m-accent)" }}>
                  Setup resolved
                </span>
                <Tag tone="outline" style={{ border: "1px solid var(--m-border)" }}>
                  {resolvedCount}/{steps.length}
                </Tag>
              </div>
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Anything skipped stays available: the resume banner on Today, Settings → Onboarding,
                and this page itself all re-open it. Skipped steps never nag.
              </p>
              <Button variant="primary" onClick={() => router.replace("/today")}>
                Enter Meridian
              </Button>
            </Card>
          )}
        </div>
      </div>
    </main>
  );
}

function inviteSetupToken(
  data: components["schemas"]["OrgUserMutationResponse"] | undefined,
): string | null {
  const candidate = data as unknown as { user?: { setup_token?: string }; setup_token?: string } | undefined;
  return candidate?.user?.setup_token ?? candidate?.setup_token ?? null;
}

function StepCard({
  step,
  busy,
  onSkip,
  children,
}: {
  step: string;
  summaryReady: boolean;
  busy: boolean;
  onSkip: (never: boolean) => void;
  children: React.ReactNode;
}) {
  const meta = Object(ORG_STEP_LABELS[step]);
  return (
    <Card className="m-rise grid gap-3 p-5" style={{ animationDelay: "30ms" }} data-testid={`wizard-step-${step}`}>
      <div className="grid gap-0.5">
        <h2 className="text-[15px] font-semibold" style={{ color: "var(--m-text)" }}>
          {meta.title}
        </h2>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {meta.blurb}
        </p>
      </div>
      {children}
      <div className="mt-1 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
        <StepActions onSkip={() => onSkip(false)} onNever={() => onSkip(true)} busy={busy} />
      </div>
    </Card>
  );
}
