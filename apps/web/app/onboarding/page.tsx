// /onboarding — first run, four steps, one card at a time.
//
// Purpose: get a new person to a working Marula, and get them out of the way.
// Four steps: your profile, a second factor, a sixty-second orientation, and
// personal defaults. The pack's state 15 is the shape — the same quiet header
// band and card stack as the organization wizard, with four rail steps instead
// of six.
//
// MFA IS SKIPPABLE, AND THE SKIP IS EXPIRY-BOUND. This is the one step where
// "later" means something specific: the person can go seven days without a
// second factor, and the card says so in words, with the date. After that the
// step asks again. A skip with no end date is how accounts end up unprotected
// for years, so the sentence and the bound go together.
//
// THE ORIENTATION IS A THING YOU SKIP, NOT A THING YOU WAIT FOR. The pack's
// line: you can skip this and come back to it. It is not a carousel, and there
// is no "next slide" that has to be pressed before the wizard lets go.
//
// THE AI STEP SAYS NOTHING ABOUT AI. It is deliberately absent: a person's
// first four minutes are about their name, their account's safety, learning
// where work lives, and how they like the product. Surfacing a feature
// decision in those four minutes is a nudge, and a nudge is a cost. The AI
// switch is in the organization wizard's sixth step and in Settings, where
// somebody is already making decisions.
//
// Non-purpose: no org setup (that is /welcome), and no step can trap a person
// here — every one can be skipped and every one can be reopened.
"use client";

import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { Spinner } from "@marula-core/ui";
import { api, useRequireAuth } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import {
  USER_STEP_LABELS,
  USER_STEP_ORDER,
  completeUserStep,
  dismissUserStep,
  fetchOnboardingSummary,
  skipUserStep,
  type OnboardingStepView,
  type OnboardingSummary,
} from "../../lib/onboarding";
import {
  EntryFooter,
  EntryHeader,
  EntryNote,
  GuideMenu,
  NeverShowDialog,
  SkipButton,
  SkipExplain,
  WizardRail,
  WizardStack,
  type StepState,
} from "../../components/auth/entry";

/** How long a person may go without a second factor when they skip this step. */
const MFA_GRACE_DAYS = 7;

/** The last step, named, so the "nothing left to focus" case is not an index
 *  into a tuple that TypeScript has to treat as possibly empty. */
const USER_LAST_STEP = USER_STEP_ORDER[USER_STEP_ORDER.length - 1] as string;

/** The label for a step, with a total fallback: an unknown key must not
 *  crash the wizard or leave a heading blank. */
function labelFor(key: string): { title: string; blurb: string } {
  const found: { title: string; blurb: string } | undefined = USER_STEP_LABELS[key];
  return found ?? { title: key, blurb: "This step is not set up yet." };
}

function railState(view: OnboardingStepView | undefined, skipped: boolean): StepState {
  if (view === undefined) return "todo";
  if (view.dismissed) return "skipped";
  if (skipped) return "skipped";
  if (view.resolved) return "done";
  return "todo";
}

export default function OnboardingPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const [summary, setSummary] = useState<OnboardingSummary | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [skipped, setSkipped] = useState<readonly string[]>([]);
  const [neverShow, setNeverShow] = useState(false);
  const [name, setName] = useState("");
  const [tz, setTz] = useState("UTC");
  const [theme, setTheme] = useState("org_default");
  const [mode, setMode] = useState("system");
  /** When the MFA grace period ends, so the card can name the date. */
  const [mfaAskAgain, setMfaAskAgain] = useState<string | null>(null);

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
      setFocus(first ?? USER_LAST_STEP);
      const { data: mine } = await api.api.GET("/api/v1/settings/user");
      if (mine) {
        const t = mine.find((v) => v.key === "appearance.theme_override");
        if (t && typeof t.value === "string") setTheme(t.value);
        const m = mine.find((v) => v.key === "appearance.mode");
        if (m && typeof m.value === "string") setMode(m.value);
      }
    })();
  }, [auth.status, auth.me, loadSummary]);

  const selfSteps = summary?.members.find((m) => m.user_id === auth.me?.user_id)?.steps ?? [];
  const step = (k: string) => selfSteps.find((s) => s.step === k);
  const resolvedCount = selfSteps.filter((s) => s.resolved).length;
  const allResolved = selfSteps.length > 0 && resolvedCount === selfSteps.length;

  const nextUserStep = (target: string): string => {
    const idx = USER_STEP_ORDER.indexOf(target as (typeof USER_STEP_ORDER)[number]);
    const next = USER_STEP_ORDER[Math.min(idx + 1, USER_STEP_ORDER.length - 1)];
    return typeof next === "string" ? next : USER_STEP_ORDER[0];
  };

  const markComplete = async (target: string) => {
    setError(null);
    setBusy(true);
    try {
      const res = await completeUserStep(target);
      if (res.error !== undefined) {
        setError(friendlyMutationText(res.error));
        return;
      }
      await loadSummary();
      setFocus(nextUserStep(target));
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
      if (!never) {
        setSkipped((current) => (current.includes(target) ? current : [...current, target]));
        // The MFA grace period is the one skip with an end date, so the card
        // can say when it asks again.
        if (target === "mfa") {
          setMfaAskAgain(
            new Date(Date.now() + MFA_GRACE_DAYS * 86_400_000).toLocaleDateString(undefined, {
              day: "numeric",
              month: "short",
              year: "numeric",
            }),
          );
        }
      }
      await loadSummary();
      setFocus(nextUserStep(target));
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
      setSaved("Profile saved.");
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
      setSaved("Defaults applied.");
      await markComplete("defaults");
    } finally {
      setBusy(false);
    }
  };

  if (auth.status !== "authed" || focus === null) {
    return (
      <div className="entry-root">
        <EntryHeader background="band" />
        <main className="entry-main">
          <Spinner label="Setting things up" />
        </main>
        <EntryFooter />
      </div>
    );
  }

  const meta = labelFor(focus);
  const stepIndex = USER_STEP_ORDER.indexOf(focus as (typeof USER_STEP_ORDER)[number]);
  const states = USER_STEP_ORDER.map((key) => railState(step(key), skipped.includes(key)));
  const skipCount = skipped.length;
  const firstName = (auth.me?.display_name ?? "").split(" ")[0] || "there";

  return (
    <div className="entry-root">
      <EntryHeader
        background="band"
        right={<GuideMenu label="Setup guide" onNever={() => setNeverShow(true)} />}
      />

      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        <WizardRail
          heading={`Step ${stepIndex + 1} of ${USER_STEP_ORDER.length}`}
          steps={USER_STEP_ORDER.map((key) => labelFor(key).title)}
          states={states}
          current={meta.title}
          onSelect={(label) => {
            const key = USER_STEP_ORDER[USER_STEP_ORDER.findIndex((k) => labelFor(k).title === label)];
            if (key !== undefined) setFocus(key);
          }}
          doneCount={resolvedCount}
          total={USER_STEP_ORDER.length}
          footnote={
            skipCount > 0 ? `${skipCount} skipped. Skipped steps never nag.` : "Every step can be skipped."
          }
        />

        <WizardStack>
          <div>
            <div className="entry-kicker">
              {allResolved ? "Setup resolved" : `Step ${stepIndex + 1} of ${USER_STEP_ORDER.length}`}
            </div>
            <h1>{meta.title}</h1>
            <p className="entry-sub">
              {allResolved
                ? `${firstName}, you are set. Everything below is still here if you want to change it.`
                : meta.blurb}
            </p>
          </div>

          {saved !== null && (
            <p className="entry-hint" role="status" style={{ margin: 0 }} data-testid="onboarding-saved">
              {saved}
            </p>
          )}
          {error !== null && (
            <p className="entry-error" role="alert" data-testid="onboarding-error">
              {error}
            </p>
          )}

          {focus === "profile" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              <div className="entry-field">
                <label className="entry-label" htmlFor="onboarding-name">
                  Your name
                </label>
                <input
                  id="onboarding-name"
                  className="entry-input"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  data-testid="onboarding-profile-name"
                />
              </div>
              <div className="entry-field">
                <label className="entry-label" htmlFor="onboarding-tz">
                  Timezone
                </label>
                <input
                  id="onboarding-tz"
                  className="entry-input"
                  value={tz}
                  onChange={(e) => setTz(e.target.value)}
                  placeholder="Europe/Athens"
                  data-testid="onboarding-profile-tz"
                />
                <p className="entry-hint" style={{ margin: 0 }}>
                  This is what your teammates see, and what the service hours are measured in.
                </p>
              </div>
              <div className="entry-actions">
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("profile", false)} busy={busy} />
                <button
                  type="button"
                  className="entry-btn"
                  onClick={() => void saveProfile()}
                  disabled={busy || name.trim() === ""}
                  data-testid="onboarding-save-profile"
                >
                  {busy ? "Saving…" : "Save and continue"}
                </button>
              </div>
            </div>
          )}

          {focus === "mfa" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              <EntryNote>
                An authenticator app or a passkey keeps this account yours even if your password
                leaks somewhere. It takes about a minute.
              </EntryNote>
              {mfaAskAgain !== null && skipped.includes("mfa") && (
                <p className="entry-hint" style={{ margin: 0 }}>
                  Skipped. We&apos;ll ask you again on {mfaAskAgain}.
                </p>
              )}
              <div className="entry-actions">
                <span className="entry-actions__spacer" />
                <SkipButton
                  onClick={() => void recordSkip("mfa", false)}
                  busy={busy}
                  label="Skip for 7 days"
                />
                <button
                  type="button"
                  className="entry-btn"
                  onClick={() => router.push("/settings/security")}
                  data-testid="onboarding-mfa-open"
                >
                  Set it up now
                </button>
              </div>
              <SkipExplain
                extra={`Setup guide, Never show this setup again (this step will still ask again in ${MFA_GRACE_DAYS} days)`}
              />
            </div>
          )}

          {focus === "orientation" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              <p className="entry-sub" style={{ margin: 0 }}>
                Marula is a service desk. Work lives in <b style={{ color: "var(--m-text)" }}>tickets</b>,
                tracked on a <b style={{ color: "var(--m-text)" }}>board</b>, against a{" "}
                <b style={{ color: "var(--m-text)" }}>client</b>. Time on a ticket is billable
                whether you type it or not, and the timer is running from the first reply.
              </p>
              <p className="entry-hint" style={{ margin: 0 }}>
                Sixty seconds is enough. You can skip this and come back to it whenever you like
                &mdash; nothing here is a prerequisite for working.
              </p>
              <div className="entry-actions">
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("orientation", false)} busy={busy} />
                <button
                  type="button"
                  className="entry-btn"
                  onClick={() => void markComplete("orientation")}
                  disabled={busy}
                  data-testid="onboarding-orientation-done"
                >
                  That&apos;s clear
                </button>
              </div>
            </div>
          )}

          {focus === "defaults" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              <div className="entry-field">
                <label className="entry-label" htmlFor="onboarding-theme">
                  Theme
                </label>
                <select
                  id="onboarding-theme"
                  className="entry-input"
                  style={{ padding: "0 12px" }}
                  value={theme}
                  onChange={(e) => setTheme(e.target.value)}
                  data-testid="onboarding-theme"
                >
                  <option value="org_default">Whatever the organization uses</option>
                  <option value="marula-neutral">Neutral</option>
                  <option value="marula-warm">Warm</option>
                </select>
              </div>
              <div className="entry-field">
                <label className="entry-label" htmlFor="onboarding-mode">
                  Light or dark
                </label>
                <select
                  id="onboarding-mode"
                  className="entry-input"
                  style={{ padding: "0 12px" }}
                  value={mode}
                  onChange={(e) => setMode(e.target.value)}
                  data-testid="onboarding-mode"
                >
                  <option value="system">Match my device</option>
                  <option value="light">Light</option>
                  <option value="dark">Dark</option>
                </select>
                <p className="entry-hint" style={{ margin: 0 }}>
                  This is your own preference. It does not change what anybody else sees, and an
                  admin can set a floor for the organization.
                </p>
              </div>
              <div className="entry-actions">
                <span className="entry-actions__spacer" />
                <SkipButton onClick={() => void recordSkip("defaults", false)} busy={busy} />
                <button
                  type="button"
                  className="entry-btn"
                  onClick={() => void saveDefaults()}
                  disabled={busy}
                  data-testid="onboarding-save-defaults"
                >
                  {busy ? "Applying…" : "Apply and continue"}
                </button>
              </div>
            </div>
          )}

          {!skipped.includes("mfa") && focus !== "mfa" && (
            <SkipExplain />
          )}

          {allResolved && (
            <div className="entry-note" data-testid="onboarding-complete">
              <span>
                All four steps are done. Nothing is waiting, and anything you skipped is still here.
              </span>
              <button
                type="button"
                className="entry-btn entry-btn--small"
                onClick={() => router.replace("/today")}
                data-testid="onboarding-finish"
                style={{ flexShrink: 0 }}
              >
                Enter Marula
              </button>
            </div>
          )}
        </WizardStack>
      </div>

      <EntryFooter />
      {neverShow && (
        <NeverShowDialog
          skippedCount={skipCount}
          busy={busy}
          onKeep={() => setNeverShow(false)}
          onConfirm={() => {
            setNeverShow(false);
            for (const key of USER_STEP_ORDER) {
              if (!step(key)?.resolved) void recordSkip(key, true);
            }
            router.replace("/today");
          }}
        />
      )}
    </div>
  );
}
