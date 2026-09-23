// Wave 26 onboarding client (org wizard + per-user first-run).
//
// Purpose: one typed wrapper over /api/v1/onboarding plus the shared step
// metadata both wizard and user flow render from. Skipping semantics are
// the product: only recorded choices exist server-side — "skip for now"
// intentionally persists NOTHING (the step stays pending/resumable) and
// "never show this again" persists the dismissed flag. Dismissals therefore
// NEVER reset on reload.
//
// Non-purpose: wizard step content forms (the pages own them), MFA
// enrollment (auth seam owns /settings/security), and role checks beyond
// the `admin:edit` permission probe exported here.

import { api } from "./api";

export type OnboardingStepView = {
  step: string;
  done: boolean;
  dismissed: boolean;
  satisfied: boolean;
  resolved: boolean;
};

export type OnboardingScopeView = {
  disabled: boolean;
  legacy_wizard_completed: boolean;
  steps: OnboardingStepView[];
};

export type OnboardingMemberView = {
  user_id: string;
  display_name: string;
  email: string;
  steps: OnboardingStepView[];
  unresolved: boolean;
};

export type OnboardingSummary = {
  org: OnboardingScopeView;
  members: OnboardingMemberView[];
};

export const ORG_STEP_ORDER = ["clients", "board", "tickets", "invites", "feeder", "ai"] as const;
export const USER_STEP_ORDER = ["profile", "mfa", "orientation", "defaults"] as const;

export const ORG_STEP_LABELS: Record<string, { title: string; blurb: string; cta: string }> = {
  clients: {
    title: "Add your first clients",
    blurb: "Create or import the clients you serve. Skipping is safe — you can add clients any time from Clients.",
    cta: "Create a client",
  },
  board: {
    title: "Set up your first board",
    blurb: "A board is your service queue — the four default statuses are ready to use.",
    cta: "Create a board",
  },
  tickets: {
    title: "Raise first tickets",
    blurb: "Work lives in tickets. Add one against a client, or skip and come back.",
    cta: "Create a ticket",
  },
  invites: {
    title: "Invite your team",
    blurb: "Each invite link carries a one-time setup token you send out-of-band.",
    cta: "Invite a member",
  },
  feeder: {
    title: "Connect a feeder connector (optional)",
    blurb: "NinjaOne, IT Glue or Auvix can feed configurations and devices into Meridian automatically.",
    cta: "Go to Imports",
  },
  ai: {
    title: "Enable AI surfaces (optional)",
    blurb: "Booking, drafting and merge-assist surfaces stay OFF until you turn them on.",
    cta: "Enable AI",
  },
};

export const USER_STEP_LABELS: Record<string, { title: string; blurb: string }> = {
  profile: {
    title: "Check your profile",
    blurb: "Your name and timezone are what teammates and SLA calendars use.",
  },
  mfa: {
    title: "Set a second factor",
    blurb: "An authenticator or passkey keeps this account yours even if the password leaks.",
  },
  orientation: {
    title: "60-second orientation",
    blurb: "What this product is and where the work lives.",
  },
  defaults: {
    title: "Personal defaults",
    blurb: "Pick your theme and how you want to be notified.",
  },
};

export function orgStepsSummary(steps: OnboardingStepView[]): { unresolved: number; first: string | null } {
  const unresolved = steps.filter((s) => !s.resolved).length;
  const first = steps.find((s) => !s.resolved)?.step ?? null;
  return { unresolved, first };
}

/** True when the signed-in member can manage org settings (admin:edit). */
export function canManageOnboarding(me: { permissions: readonly string[] } | null): boolean {
  return me?.permissions.includes("admin:edit") ?? false;
}

export async function fetchOnboardingSummary(): Promise<OnboardingSummary | null> {
  const { data } = await api.api.GET("/api/v1/onboarding");
  return data ?? null;
}

/** Records "never show this again" for one org step. */
export function dismissOrgStep(step: string) {
  return api.api.POST("/api/v1/onboarding/org/steps/{step}/skip", {
    params: { path: { step } },
    body: { never: true },
  });
}

/**
 * "Skip for now" — deliberately a no-op against the registry: nothing is
 * persisted, so the step simply stays pending (and the wizard lists it
 * as skipped). Skipped steps are recorded implicitly by their absence.
 */
export function skipOrgStep(_step: string) {
  return api.api.POST("/api/v1/onboarding/org/steps/{step}/skip", {
    params: { path: { step: _step } },
    body: { never: false },
  });
}

export function completeOrgStep(step: string) {
  return api.api.POST("/api/v1/onboarding/org/steps/{step}/complete", {
    params: { path: { step } },
  });
}

export function dismissUserStep(step: string) {
  return api.api.POST("/api/v1/onboarding/user/steps/{step}/skip", {
    params: { path: { step } },
    body: { never: true },
  });
}

export function skipUserStep(step: string) {
  return api.api.POST("/api/v1/onboarding/user/steps/{step}/skip", {
    params: { path: { step } },
    body: { never: false },
  });
}

export function completeUserStep(step: string) {
  return api.api.POST("/api/v1/onboarding/user/steps/{step}/complete", {
    params: { path: { step } },
  });
}

/** Org-wide master switch (Settings → Onboarding). */
export function setOnboardingDisabled(disabled: boolean) {
  return api.api.POST("/api/v1/onboarding/disable", { body: { disabled } });
}

/** Re-runs the org wizard: clears per-step flags, keeps the disable switch. */
export function resetOrgOnboarding() {
  return api.api.POST("/api/v1/onboarding/reset/org");
}

/** Re-triggers one member's first-run flow (admin) or clears own. */
export function resetUserOnboarding(userId?: string) {
  return api.api.POST("/api/v1/onboarding/reset/user", {
    body: userId ? { user_id: userId } : {},
  });
}

/** Owner dismissed the banner: recorded, never resets on reload. */
export function dismissOrgBanner() {
  return api.api.PUT("/api/v1/settings/org", {
    body: { key: "onboarding.org_banner_dismissed", value: true },
  });
}

/** Member dismissed their own first-run prompt (same stickiness). */
export function dismissUserBanner() {
  return api.api.PUT("/api/v1/settings/user", {
    body: { key: "onboarding.user_banner_dismissed", value: true },
  });
}
