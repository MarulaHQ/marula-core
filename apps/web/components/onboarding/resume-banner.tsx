// Wave 26 onboarding resume banner — the wizard's quiet entry point on /today.
//
// Purpose: replaces the old static setup banner. One dismissal
// stickiness contract:
//   • org flow (admin callers): unresolved org steps + not disabled +
//     banner not previously dismissed → "Continue setup". Dismissing
//     HERE is a recorded 'later' (onboarding.org_banner_dismissed)
//     that persists across reloads — the record IS the state, so this
//     is never a nag-loop, and a re-run from Settings clears it.
//   • user flow (every member): unresolved personal steps + own prompt
//     not dismissed (onboarding.user_banner_dismissed) → open /onboarding.
// Settings → Onboarding always re-opens any flow org-wide.
//
// Non-purpose: no step logic — deep-linked entry affordance only.

"use client";

import { Button, Card } from "@meridian/ui";
import { useEffect, useState } from "react";
import { api, useAuth } from "../../lib/api";
import {
  canManageOnboarding,
  dismissOrgBanner,
  dismissUserBanner,
  fetchOnboardingSummary,
  orgStepsSummary,
  type OnboardingSummary,
} from "../../lib/onboarding";

type Loaded = {
  summary: OnboardingSummary | null;
  orgBannerDismissed: boolean;
  userBannerDismissed: boolean;
};



export function OnboardingResumeBanner() {
  const auth = useAuth();
  const [state, setState] = useState<Loaded | null>(null);

  useEffect(() => {
    if (auth.status !== "authed" || auth.me === null) return;
    let cancelled = false;
    void (async () => {
      const [summary, orgVals, userVals] = await Promise.all([
        fetchOnboardingSummary(),
        api.api.GET("/api/v1/settings/org").then((r) => r.data ?? []),
        api.api.GET("/api/v1/settings/user").then((r) => r.data ?? []),
      ]);
      if (cancelled || summary === null) return;
      const pick = (vals: { key: string; value: unknown }[], key: string) =>
        (vals.find((v) => v.key === key)?.value as boolean) === true;
      setState({
        summary,
        orgBannerDismissed: pick(orgVals, "onboarding.org_banner_dismissed"),
        userBannerDismissed: pick(userVals, "onboarding.user_banner_dismissed"),
      });
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status, auth.me]);

  const summary = state?.summary ?? null;
  if (
    auth.status !== "authed" ||
    auth.me === null ||
    summary === null ||
    state === null
  )
    return null;

  const summaryDisabled = summary.org.disabled;
  const isAdmin = canManageOnboarding(auth.me);
  const orgResolved = summary.org.steps.filter((s) => s.resolved).length;
  const orgProgress = orgStepsSummary(summary.org.steps);
  const myRows = summary.members.find((m) => m.user_id === auth.me?.user_id);
  const orgApplies =
    isAdmin && !summaryDisabled && orgProgress.unresolved > 0 && !state.orgBannerDismissed;
  const userApplies =
    !summaryDisabled &&
    myRows !== undefined &&
    myRows.steps.some((s) => !s.resolved) &&
    !state.userBannerDismissed;

  if (!orgApplies && !userApplies) return null;

  return (
    <Card
      className="m-rise grid gap-2 p-4"
      role="region"
      aria-label="Onboarding"
      data-testid="onboarding-banner"
    >
      {orgApplies && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="grid gap-0.5">
            <p className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
              Finish org setup ({orgResolved}/{summary.org.steps.length} steps resolved)
            </p>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Clients, board, tickets, team — every step is skippable, and "skip for now" changes
              nothing: the org already runs on its defaults.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <a
              href="/welcome"
              className="rounded-[var(--m-radius-container)] px-3 py-1.5 text-[13px] font-medium"
              style={{ background: "var(--m-accent)", color: "var(--m-on-accent)" }}
              data-testid="onboarding-resume-org"
            >
              Continue setup
            </a>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                void dismissOrgBanner();
                setState({ ...state, orgBannerDismissed: true });
              }}
              data-testid="onboarding-banner-dismiss"
            >
              Later
            </Button>
          </div>
        </div>
      )}
      {userApplies && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="grid gap-0.5">
            <p className="text-[13px] font-medium" style={{ color: "var(--m-text)" }}>
              Set up your workspace
            </p>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Profile, second factor, orientation, defaults — every step is skippable.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <a
              href="/onboarding"
              className="rounded-[var(--m-radius-container)] px-3 py-1.5 text-[13px] font-medium"
              style={{ background: "var(--m-accent)", color: "var(--m-on-accent)" }}
              data-testid="onboarding-resume-user"
            >
              Open first-run
            </a>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                void dismissUserBanner();
                setState({ ...state, userBannerDismissed: true });
              }}
            >
              Dismiss
            </Button>
          </div>
        </div>
      )}
    </Card>
  );
}
