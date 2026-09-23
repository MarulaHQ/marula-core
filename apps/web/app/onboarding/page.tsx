// /onboarding — per-user first-run flow (Wave 26 leg b).
//
// Purpose: the member-level onboarding lands right after invite
// acceptance (and re-opens from Settings → Onboarding or the /today
// prompt): check your profile, set a second factor, the 60-second
// product orientation (what a PSA is + where the work lives), and
// personal defaults (theme/mode/notifications). EVERY step is
// independently skippable — "Skip for now" records nothing (the step
// stays pending/resumable), "Never show this again" persists the
// dismissed flag via the typed settings registry and NEVER resets on
// reload. Profile and MFA completion is server-gated (the route
// verifies display_name/timezone and enrollment itself).
//
// Non-purpose: no password step here — passwords are set at invite
// acceptance on the public page; MFA deep-links to /settings/security.
// Admin re-trigger flows through POST /api/onboarding/reset/user.

"use client";

import { Button, Card, Input } from "@meridian/ui";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import {
  USER_STEP_LABELS,
  USER_STEP_ORDER,
  completeUserStep,
  dismissUserStep,
  fetchOnboardingSummary,
  skipUserStep,
  type OnboardingSummary,
} from "../../lib/onboarding";
import { StepChecklist } from "../../components/onboarding/step-checklist";
import { StepActions } from "../../components/onboarding/step-actions";
import { OnboardingWelcomeHero } from "../../components/onboarding/welcome-hero";

const TIMEZONES = [
  "UTC",
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "Europe/London",
  "Europe/Berlin",
  "Europe/Warsaw",
  "Europe/Istanbul",
  "Europe/Moscow",
  "Asia/Dubai",
  "Asia/Kolkata",
  "Asia/Shanghai",
  "Asia/Manila",
  "Asia/Tokyo",
  "Australia/Sydney",
  "Pacific/Auckland",
];

const THEMES = [
  ["org_default", "Follow the organization"],
  ["tangerine", "Tangerine"],
  ["slate", "Slate"],
  ["high_contrast", "High contrast"],
  ["copper", "Copper"],
] as const;

const MODES = [
  ["system", "System"],
  ["light", "Light"],
  ["dark", "Dark"],
] as const;

export default function UserOnboardingPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [summary, setSummary] = useState<OnboardingSummary | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // profile draft
  const [name, setName] = useState("");
  const [tz, setTz] = useState("UTC");
  // defaults draft
  const [theme, setTheme] = useState("org_default");
  const [mode, setMode] = useState("system");

  const loadSummary = useCallback(async (): Promise<OnboardingSummary | null> => {
    const s = await fetchOnboardingSummary();
    if (s) setSummary(s);
    return s;
  }, []);

  useEffect(() => {
    if (auth.status !== "authed" || auth.me === null) return;
    setName(auth.me.display_name);
    setTz(auth.me.timezone);
    void (async () => {
      const s = await loadSummary();
      const first =
        s?.members.find((m) => m.user_id === auth.me?.user_id)?.steps.find((st) => !st.resolved)
          ?.step ?? null;
      setFocus(first);
      const { data: mine } = await api.api.GET("/api/v1/settings/user");
      if (mine) {
        const t = mine.find((v) => v.key === "appearance.theme_override");
        if (t && typeof t.value === "string") setTheme(t.value);
        const m = mine.find((v) => v.key === "appearance.mode");
        if (m && typeof m.value === "string") setMode(m.value);
      }
    })();
  }, [auth.status, auth.me, loadSummary]);

  if (auth.status !== "authed") {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <p className="text-sm" style={{ color: "var(--m-text-muted)" }}>
          Loading…
        </p>
      </div>
    );
  }

  const steps = USER_STEP_ORDER
    .map((k) => summary?.members.find((m) => m.user_id === auth.me?.user_id)?.steps.find((st) => st.step === k))
    .filter((x): x is NonNullable<typeof x> => x !== undefined);
  if (summary === null) {
    // members[].steps only arrives for admins; self-view falls back to
    // org.steps-style read from members of size ≥ 1 — for a non-admin
    // caller the members list is empty, so drive off own state below.
  }
  const selfSteps =
    summary?.members.find((m) => m.user_id === auth.me?.user_id)?.steps ?? [];
  const viewSteps = summary !== null && selfSteps.length > 0 ? selfSteps : steps;
  const step = (k: string) => viewSteps.find((s) => s.step === k);
  const resolvedCount = viewSteps.filter((s) => s.resolved).length;
  const allResolved = viewSteps.length > 0 && resolvedCount === viewSteps.length;

  const refresh = async (next?: string) => {
    await loadSummary();
    if (next) setFocus(next);
  };

  function nextUserStep(idx: number): string {
    const next = USER_STEP_ORDER[Math.min(idx + 1, USER_STEP_ORDER.length - 1)];
    return typeof next === "string" ? next : USER_STEP_ORDER[0];
  }

  const markComplete = async (target: string) => {
    setError(null);
    setBusy(true);
    try {
      const res = await completeUserStep(target);
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      const idx = USER_STEP_ORDER.indexOf(target as (typeof USER_STEP_ORDER)[number]);
      await loadSummary();
      setFocus(nextUserStep(idx));
    } finally {
      setBusy(false);
    }
  };

  const recordSkip = async (target: string, never: boolean) => {
    setError(null);
    setBusy(true);
    try {
      const res = await (never ? dismissUserStep(target) : skipUserStep(target));
      const err = (res as { error?: unknown }).error;
      if (err !== undefined && err !== null) {
        setError(friendlyMutationText(err));
        return;
      }
      const idx = USER_STEP_ORDER.indexOf(target as (typeof USER_STEP_ORDER)[number]);
      await loadSummary();
      setFocus(nextUserStep(idx));
    } finally {
      setBusy(false);
    }
  };

  const saveProfile = async () => {
    setError(null);
    setSaved(null);
    setBusy(true);
    try {
      const res = await api.api.PATCH("/api/v1/users/me", {
        body: { display_name: name.trim(), timezone: tz },
      });
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      setSaved("Profile saved");
      await markComplete("profile");
    } finally {
      setBusy(false);
    }
  };

  const saveDefaults = async () => {
    setError(null);
    setSaved(null);
    setBusy(true);
    try {
      for (const body of [
        { key: "appearance.theme_override", value: theme },
        { key: "appearance.mode", value: mode },
      ]) {
        const res = await api.api.PUT("/api/v1/settings/user", { body });
        if (res.error !== undefined) {
          setError(friendlyMutationText(res.error));
          return;
        }
      }
      setSaved("Defaults applied");
      await markComplete("defaults");
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
          First-run
        </p>
        <h1 className="text-lg font-semibold tracking-tight" style={{ color: "var(--m-text)" }}>
          Welcome, {auth.me?.display_name ?? "…"}
        </h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Four quick steps — every one is skippable, and skips stick. You can replay this
          from Settings → Onboarding any time.
        </p>
      </header>

      <div className="grid gap-5 md:grid-cols-[240px_1fr] md:items-start">
        <Card className="p-3">
          <StepChecklist steps={viewSteps} labels={USER_STEP_LABELS} current={focus} onSelect={(s) => setFocus(s)} />
        </Card>
        <div className="grid gap-4">
          {/* Wave 26 first-open WELCOME: animated theme pick that applies
              instantly and completes the defaults step. Shows only while
              defaults are unresolved (first open / re-trigger). */}
          {!step("defaults")?.resolved && (
            <OnboardingWelcomeHero
              saving={busy}
              onChosen={() => void markComplete("defaults")}
            />
          )}

          {error && (
            <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          )}
          {saved && (
            <p className="text-xs" style={{ color: "var(--m-status-ok-fg, var(--m-accent))" }}>
              {saved}
            </p>
          )}

          {focus === "profile" && (
            <UserStepCard step="profile" busy={busy} onSkip={(n) => void recordSkip("profile", n)}>
              <div className="grid gap-2">
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  aria-label="Display name"
                  placeholder="Your full name"
                  data-testid="onboarding-profile-name"
                />
                <select
                  aria-label="Timezone"
                  value={tz}
                  onChange={(e) => setTz(e.target.value)}
                  className="rounded px-2 py-1.5 text-[13px]"
                  style={{ background: "var(--m-canvas-raised)", color: "var(--m-text)", border: "1px solid var(--m-border)" }}
                  data-testid="onboarding-profile-tz"
                >
                  {TIMEZONES.map((t) => (
                    <option key={t} value={t}>
                      {t}
                    </option>
                  ))}
                </select>
                <div className="flex items-center gap-2">
                  <Button variant="primary" onClick={() => void saveProfile()} disabled={busy || !name.trim()}>
                    Save profile
                  </Button>
                  {step("profile")?.satisfied && (step("profile")?.done ?? false) ? null : (
                    <Button onClick={() => void markComplete("profile")} disabled={busy}>
                      Mark done
                    </Button>
                  )}
                </div>
              </div>
            </UserStepCard>
          )}

          {focus === "mfa" && (
            <UserStepCard step="mfa" busy={busy} onSkip={(n) => void recordSkip("mfa", n)}>
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Enroll an authenticator app (TOTP) or a passkey. It takes about two minutes
                and stays optional — you decide the risk you accept.
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <Button onClick={() => router.push("/settings/security")} data-testid="onboarding-mfa-link">
                  Open Security
                </Button>
                {step("mfa")?.satisfied && !step("mfa")?.done && (
                  <Button variant="primary" onClick={() => void markComplete("mfa")} disabled={busy}>
                    Mark done
                  </Button>
                )}
              </div>
            </UserStepCard>
          )}

          {focus === "orientation" && (
            <UserStepCard step="orientation" busy={busy} onSkip={(n) => void recordSkip("orientation", n)}>
              <div className="grid gap-3 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                <p style={{ color: "var(--m-text)" }} className="font-medium">
                  What a PSA is — and where the work lives
                </p>
                <p>
                  This is a professional services automation (PSA): the system of record for an
                  MSP or IT-services team. Work arrives as{" "}
                  <strong style={{ color: "var(--m-text)" }}>tickets</strong> on{" "}
                  <strong style={{ color: "var(--m-text)" }}>boards</strong>; each ticket hangs off
                  a <strong style={{ color: "var(--m-text)" }}>client</strong> and its{" "}
                  <strong style={{ color: "var(--m-text)" }}>sites and contacts</strong>.
                </p>
                <ul className="grid gap-1.5 list-disc pl-4">
                  <li>
                    <strong style={{ color: "var(--m-text)" }}>Today</strong> is your work queue —
                    what needs attention and when.
                  </li>
                  <li>
                    <strong style={{ color: "var(--m-text)" }}>Boards</strong> are where tickets
                    live, move and get SLAs.
                  </li>
                  <li>
                    <strong style={{ color: "var(--m-text)" }}>Clients</strong> hold inventory,
                    contracts and knowledge for each account.
                  </li>
                  <li>
                    <strong style={{ color: "var(--m-text)" }}>Time</strong> rides the workbar
                    timer and rolls into approvals and invoices.
                  </li>
                </ul>
                <p>Start at Today; everything else is one click away.</p>
              </div>
              <div className="grid gap-2">
                <Button variant="primary" onClick={() => void markComplete("orientation")} disabled={busy}>
                  Got it — done
                </Button>
              </div>
            </UserStepCard>
          )}

          {focus === "defaults" && (
            <UserStepCard step="defaults" busy={busy} onSkip={(n) => void recordSkip("defaults", n)}>
              <div className="grid gap-2">
                <Labelled label="Theme">
                  <select
                    aria-label="Theme"
                    value={theme}
                    onChange={(e) => setTheme(e.target.value)}
                    className="rounded px-2 py-1.5 text-[13px]"
                    style={{ background: "var(--m-canvas-raised)", color: "var(--m-text)", border: "1px solid var(--m-border)" }}
                    data-testid="onboarding-theme"
                  >
                    {THEMES.map(([id, label]) => (
                      <option key={id} value={id}>
                        {label}
                      </option>
                    ))}
                  </select>
                </Labelled>
                <Labelled label="Appearance">
                  <select
                    aria-label="Mode"
                    value={mode}
                    onChange={(e) => setMode(e.target.value)}
                    className="rounded px-2 py-1.5 text-[13px]"
                    style={{ background: "var(--m-canvas-raised)", color: "var(--m-text)", border: "1px solid var(--m-border)" }}
                    data-testid="onboarding-mode"
                  >
                    {MODES.map(([id, label]) => (
                      <option key={id} value={id}>
                        {label}
                      </option>
                    ))}
                  </select>
                </Labelled>
                <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  Notification details (docks, digests, per-channel routing) live under{" "}
                  <a href="/settings/notifications" className="underline">
                    Settings → Notifications
                  </a>
                  . The theme rides the org's Wave-24 preset system.
                </p>
                <div className="flex items-center gap-2">
                  <Button variant="primary" onClick={() => void saveDefaults()} disabled={busy} data-testid="onboarding-save-defaults">
                    Apply defaults
                  </Button>
                  <Button onClick={() => router.push("/settings/preferences")} disabled={busy}>
                    More preferences
                  </Button>
                </div>
              </div>
            </UserStepCard>
          )}

          {allResolved && (
            <Card className="m-rise grid gap-2 p-5" data-testid="user-onboarding-complete">
              <p className="text-[13px] font-semibold" style={{ color: "var(--m-accent)" }}>
                You're set
              </p>
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Anything you skipped stays available and never nags: re-run from Settings →
                Onboarding whenever you change your mind.
              </p>
              <Button variant="primary" onClick={() => router.replace("/today")} data-testid="onboarding-finish">
                Go to Today
              </Button>
            </Card>
          )}

          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Tip: after invite acceptance your password step is already handled — this page
            covers everything your admins asked you to set yourself.
          </p>
        </div>
      </div>
    </main>
  );
}

function UserStepCard({
  step,
  busy,
  onSkip,
  children,
}: {
  step: string;
  busy: boolean;
  onSkip: (never: boolean) => void;
  children: React.ReactNode;
}) {
  const meta = Object(USER_STEP_LABELS[step]);
  return (
    <Card className="m-rise grid gap-3 p-5" style={{ animationDelay: "30ms" }} data-testid={`onboarding-step-${step}`}>
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

function Labelled({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="grid gap-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
      <span className="uppercase tracking-wide text-[10px] font-semibold">{label}</span>
      {children}
    </label>
  );
}
