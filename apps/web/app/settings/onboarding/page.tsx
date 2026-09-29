"use client";
/**
 * /settings/onboarding — Wave 26 onboarding control panel.
 *
 * Purpose: the org-level home for the flow family. Org wizard status
 * (per-step recorded choices + live server-verified gate states), the
 * org-wide disable master switch (true silences both the org wizard and
 * member first-run ON THE RECORD — resuming keeps all state), the
 * re-run control (clears per-step flags, keeps the disable switch), and
 * the per-member re-trigger list (admin clears a member's personal
 * flags; the member's next login resumes the flow).
 *
 * Non-purpose: the wizard flows themselves (/welcome for the org
 * wizard, /onboarding for the personal one) and audit browsing.
 */
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { Button, Card, FieldHint, Tag } from "@marula-core/ui";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import {
  canManageOnboarding,
  fetchOnboardingSummary,
  orgStepsSummary,
  resetOrgOnboarding,
  resetUserOnboarding,
  setOnboardingDisabled,
  USER_STEP_LABELS,
  type OnboardingSummary,
} from "../../../lib/onboarding";

function WizardLink() {
  const router = useRouter();
  return (
    <Button data-testid="onboarding-open-wizard" onClick={() => router.push("/welcome")}>
      Open the wizard
    </Button>
  );
}

export default function OnboardingSettingsPage() {
  const auth = useRequireAuth();
  const [summary, setSummary] = useState<OnboardingSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [confirmReRun, setConfirmReRun] = useState(false);

  const load = useCallback(async () => {
    const s = await fetchOnboardingSummary();
    if (s === null) return;
    setSummary(s);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void load();
  }, [auth.status, load]);

  const isAdmin = canManageOnboarding(auth.me);

  const rerun = async () => {
    setConfirmReRun(false);
    setBusy(true);
    setSaved(null);
    try {
      await resetOrgOnboarding();
      await load();
      setSaved("Org onboarding re-armed — the wizard resumes pending.");
    } finally {
      setBusy(false);
    }
  };

  const toggleDisable = async () => {
    setBusy(true);
    setSaved(null);
    try {
      const next = !(summary?.org.disabled ?? false);
      const res = await setOnboardingDisabled(next);
      if ((res as { error?: unknown }).error) {
        const err = (res as { error?: unknown }).error;
        setSaved(typeof err === "object" && err !== null ? String(err) : "Disable failed");
        return;
      }
      await load();
      setSaved(next ? "Onboarding disabled org-wide (state kept)." : "Onboarding flows re-enabled org-wide.");
    } finally {
      setBusy(false);
    }
  };

  const retrigger = async (userId: string) => {
    setBusy(true);
    setSaved(null);
    try {
      await resetUserOnboarding(userId);
      await load();
      setSaved("Member re-triggered — their first-run flow resumes.");
    } finally {
      setBusy(false);
    }
  };

  if (summary === null) {
    return (
      <div className="grid max-w-3xl gap-3 p-4">
        <SettingsHeader title="Onboarding" description="Loading…" />
      </div>
    );
  }

  return (
    <div className="grid max-w-3xl gap-4">
      <SettingsHeader
        title="Onboarding"
        description="First-run flows for the org and its members. Every step carries skip controls ('skip for now' records nothing and stays resumable; 'never show this again' sticks)."
      >
        {saved !== null && (
          <span className="text-[12px]" style={{ color: "var(--m-accent)" }} data-testid="onboarding-saved-note">
            {saved}
          </span>
        )}
      </SettingsHeader>

      <Card className="grid gap-3 p-4" data-testid="onboarding-org-card">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-[14px] font-semibold" style={{ color: "var(--m-text)" }}>
            Org setup wizard
          </h2>
          <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            {summary.org.legacy_wizard_completed ? "legacy completion carried over · " : ""}
            {orgStepsSummary(summary.org.steps).unresolved} step(s) pending
          </span>
        </div>
        <ul className="grid gap-1.5">
          {summary.org.steps.map((s) => (
            <li key={s.step} className="flex items-center gap-2 text-[13px]">
              <span
                aria-hidden="true"
                className="grid h-4 w-4 place-items-center rounded-full text-[10px]"
                style={{
                  border: `1.5px solid ${s.done ? "var(--m-accent)" : s.dismissed ? "var(--m-border)" : "var(--m-border)"}`,
                  color: s.done ? "var(--m-accent)" : "var(--m-text-muted)",
                }}
              >
                {s.done || s.dismissed ? "✓" : ""}
              </span>
              <span style={{ color: "var(--m-text)" }}>{s.step}</span>
              {s.done && (
                <Tag tone="outline" style={{ border: "1px solid var(--m-border)" }}>
                  {s.satisfied ? "done · verified" : "done"}
                </Tag>
              )}
              {s.dismissed && !s.done && (
                <Tag tone="outline" style={{ border: "1px solid var(--m-border)" }}>
                  dismissed
                </Tag>
              )}
              {!s.resolved && s.satisfied && (
                <Tag tone="outline" style={{ border: "1px solid var(--m-accent)" }}>
                  ready to complete
                </Tag>
              )}
              {!s.resolved && !s.satisfied && !s.dismissed && (
                <span className="ml-auto text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  pending
                </span>
              )}
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-2">
          <WizardLink />
          <Button data-testid="onboarding-reset-org" disabled={busy || confirmReRun} variant="ghost" onClick={() => setConfirmReRun(true)}>
            Re-run setup…
          </Button>
          {confirmReRun && (
            <Card className="grid gap-2 p-3" data-testid="onboarding-reset-confirm">
              <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Clear every recorded step state? The wizard re-arms from the first step. The
                org-wide disable switch below is NOT touched.
              </p>
              <div className="flex items-center gap-2">
                <Button variant="ghost" size="sm" onClick={() => void rerun()} disabled={busy}>
                  Confirm re-run
                </Button>
                <Button variant="ghost" size="sm" onClick={() => setConfirmReRun(false)}>
                  Cancel
                </Button>
              </div>
            </Card>
          )}
        </div>
        <FieldHint>
          Re-running resumes guidance only — it never deletes clients, boards, tickets or
          invites.
        </FieldHint>
      </Card>

      <Card className="grid gap-3 p-4" data-testid="onboarding-disable-card">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="grid gap-0.5">
            <h2 className="text-[14px] font-semibold" style={{ color: "var(--m-text)" }}>
              Onboarding flows org-wide
            </h2>
            <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              {summary.org.disabled
                ? "Currently DISABLED org-wide — no wizard banner, no member first-run. State is kept, so flipping back resumes everything."
                : "Currently enabled. Flipping off silences the wizard banner AND every member's first-run (recorded, never silent)."}
            </p>
          </div>
          <Button data-testid="onboarding-toggle-disable" disabled={busy || !isAdmin} onClick={() => void toggleDisable()}>
            {summary.org.disabled ? "Re-enable onboarding" : "Disable org-wide"}
          </Button>
        </div>
      </Card>

      {isAdmin && (
        <Card className="grid gap-3 p-4" data-testid="onboarding-members-card">
          <h2 className="text-[14px] font-semibold" style={{ color: "var(--m-text)" }}>
            Member first-run
          </h2>
          <ul className="grid gap-2">
            {summary.members.map((m) => (
              <li key={m.user_id} className="flex flex-wrap items-center gap-2" data-testid={`onboarding-member-${m.user_id}`}>
                <span className="min-w-0 grid gap-0">
                  <span className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
                    {m.display_name}
                  </span>
                  <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    {m.email}
                  </span>
                </span>
                <span className="ml-auto flex items-center gap-2">
                  {!m.unresolved ? (
                    <Tag tone="outline" style={{ border: "1px solid var(--m-border)" }}>
                      resolved
                    </Tag>
                  ) : (
                    <>
                      <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                        {m.steps.filter((s) => !s.resolved).map((s) => USER_STEP_LABELS[s.step]?.title ?? s.step).join(" · ")}
                      </span>
                      <Button size="sm" variant="ghost" disabled={busy} onClick={() => void retrigger(m.user_id)} data-testid={`onboarding-retrigger-${m.user_id}`}>
                        Re-trigger
                      </Button>
                    </>
                  )}
                </span>
              </li>
            ))}
          </ul>
          <FieldHint>
            Re-trigger clears the member's personal progress; their recorded skips return to
            pending without ever emailing them again mid-flow.
          </FieldHint>
        </Card>
      )}
    </div>
  );
}
