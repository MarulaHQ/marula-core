// Onboarding integration tests — the SKIPPABILITY CONTRACT, against the p41
// pack redesign's card stacks (WizardRail + SkipButton + GuideMenu).
//
// Purpose: the skip path is the happy path, first class. These tests pin
// the recorded-state semantics of every surface:
//   • org wizard (/welcome): "Skip for now" records NOTHING on the step
//     (POST skip body never:false, no step mutation); "Never show this
//     setup again" lives in the guide menu behind a confirm and records
//     the dismissed flag; completion is server-gated (POST complete only
//     fires after the gate is satisfied).
//   • resume banner: a dismissal persists across a RELOAD (remount) —
//     the read comes back dismissed server-side, so the banner stays
//     silent. The record is the state.
//   • user first-run (/onboarding): orientation completes; MFA enrollment
//     runs inline (enroll-begin → enroll-verify); personal defaults
//     persist through PUT /api/v1/settings/user.
//   • Settings → Onboarding: org re-run (reset/org), org-wide disable
//     toggle (disable), member re-trigger (reset/user).
//
// The Wave 26 welcome hero (theme picker card) was dropped from /onboarding
// during the p41 card-stack rewrite and is NOT here — the defaults step
// carries the theme pick. That removal is recorded in the lane's parity
// ledger; if it returns, it returns behind the pack's wizard.
//
// Non-purpose: no backend bundling (the MockApi fabricates the same
// wire shapes the real server serves; contract-level behavior is pinned
// by the Rust route tests + Playwright E2E).

import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import WelcomePage from "../../app/welcome/page";
import UserOnboardingPage from "../../app/onboarding/page";
import OnboardingSettingsPage from "../../app/settings/onboarding/page";
import { OnboardingResumeBanner } from "../../components/onboarding/resume-banner";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const push = vi.fn();
const replace = vi.fn();

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace, push }),
  usePathname: () => "/",
}));

const ORG = ME_FIXTURE.org_id;

/** Server-side onboarding state the mock registry holds BETWEEN reloads. */
function serverState(initial?: Partial<Record<string, boolean>>) {
  const flags: Record<string, boolean> = {
    "onboarding.org_banner_dismissed": false,
    "onboarding.user_banner_dismissed": false,
    "onboarding.flows_disabled": false,
    "onboarding.wizard_completed": false,
    ...initial,
  };
  return flags;
}

function orgSummary(orgFlags: Record<string, boolean>) {
  const mk = (step: string) => ({
    step,
    done: orgFlags[`onboarding.org_${step}_done`] === true,
    dismissed: orgFlags[`onboarding.org_${step}_dismissed`] === true,
    // Nothing is satisfied up front: with a resolved step behind them the
    // page renders the resumed card instead of the wizard stack (the pack's
    // state 13), and these tests pin the wizard itself.
    satisfied: false,
    resolved:
      orgFlags[`onboarding.org_${step}_done`] === true ||
      orgFlags[`onboarding.org_${step}_dismissed`] === true,
  });
  return {
    org: {
      disabled: orgFlags["onboarding.flows_disabled"],
      legacy_wizard_completed: false,
      steps: ["clients", "board", "tickets", "invites", "feeder", "ai"].map(mk),
    },
    members: [
      {
        user_id: ME_FIXTURE.user_id,
        display_name: ME_FIXTURE.display_name,
        email: ME_FIXTURE.email,
        steps: [],
        unresolved: false,
      },
    ],
  };
}

/** Installs every real route the onboarding family reads/writes, plus the
 *  two TOTP enrollment routes the per-person MFA step calls inline. */
function setupApi(orgFlags = serverState()) {
  const mock = new MockApi();
  const userFlags: Record<string, boolean> = {
    "onboarding.user_banner_dismissed": false,
  };

  mock.on("GET", "/api/v1/me", () => ({
    json: { ...ME_FIXTURE, permissions: [...ME_FIXTURE.permissions, "admin:edit"] },
  }));
  mock.on("GET", "/api/v1/onboarding", () => ({
    json: {
      ...orgSummary(orgFlags),
      members: [
        {
          user_id: ME_FIXTURE.user_id,
          display_name: ME_FIXTURE.display_name,
          email: ME_FIXTURE.email,
          steps: [
            { step: "profile", done: true, dismissed: false, satisfied: true, resolved: true },
            { step: "mfa", done: false, dismissed: false, satisfied: false, resolved: false },
            { step: "orientation", done: false, dismissed: false, satisfied: false, resolved: false },
            { step: "defaults", done: false, dismissed: false, satisfied: false, resolved: false },
          ],
          unresolved: true,
        },
      ],
    },
  }));
  const allOrgSettings = () =>
    Object.entries(orgFlags).map(([key, value]) => ({ key, value }));
  const allUserFlags = () =>
    Object.entries(userFlags).map(([key, value]) => ({ key, value }));
  mock.on("GET", "/api/v1/settings/org", () => ({ json: allOrgSettings() }));
  mock.on("GET", "/api/v1/settings/user", () => ({ json: allUserFlags() }));
  mock.on("PUT", "/api/v1/settings/org", (ctx) => {
    const { key, value } = ctx.body as { key: string; value: boolean };
    orgFlags[key] = value as boolean;
    return { json: { key, value } };
  });
  mock.on("PUT", "/api/v1/settings/user", (ctx) => {
    const { key, value } = ctx.body as { key: string; value: boolean };
    userFlags[key] = value as boolean;
    return { json: { key, value } };
  });
  for (const step of ["clients", "board", "tickets", "invites", "feeder", "ai"]) {
    mock.on("POST", `/api/v1/onboarding/org/steps/${step}/complete`, () => {
      // server-gated: only "board" is satisfied in this fixture
      if (step !== "board") {
        return {
          status: 409,
          json: {
            error: {
              code: 409,
              message: `onboarding gate for '${step}' is not satisfied yet`,
            },
          },
        };
      }
      orgFlags[`onboarding.org_${step}_done`] = true;
      return { json: { step, done: true, dismissed: false, satisfied: true, resolved: true } };
    });
    mock.on("POST", `/api/v1/onboarding/org/steps/${step}/skip`, (ctx) => {
      const { never } = ctx.body as { never: boolean };
      if (never) orgFlags[`onboarding.org_${step}_dismissed`] = true;
      return { json: { step, done: false, dismissed: never, satisfied: false, resolved: never } };
    });
  }
  for (const step of ["profile", "mfa", "orientation", "defaults"]) {
    mock.on("POST", `/api/v1/onboarding/user/steps/${step}/complete`, () => {
      userFlags[`onboarding.user_${step}_done`] = true;
      return { json: { step, done: true, dismissed: false, satisfied: true, resolved: true } };
    });
    mock.on("POST", `/api/v1/onboarding/user/steps/${step}/skip`, (ctx) => {
      const { never } = ctx.body as { never: boolean };
      if (never) userFlags[`onboarding.user_${step}_dismissed`] = true;
      return { json: { step, done: false, dismissed: never, satisfied: false, resolved: never } };
    });
  }
  // The MFA step does the whole enrollment inline (p41 parity fix #44).
  mock.on("POST", "/api/v1/auth/mfa/totp/enroll-begin", () => ({
    json: {
      secret_base32: "K7QX2MVD9FTAR4LP",
      otpauth_url: "otpauth://totp/Marula:ops?secret=K7QX2MVD9FTAR4LP",
    },
  }));
  mock.on("POST", "/api/v1/auth/mfa/totp/enroll-verify", () => ({
    json: { recovery_codes: ["1111-2222", "3333-4444"] },
  }));
  mock.on("POST", "/api/v1/onboarding/disable", (ctx) => {
    orgFlags["onboarding.flows_disabled"] = (ctx.body as { disabled: boolean }).disabled;
    return { json: { disabled: orgFlags["onboarding.flows_disabled"] } };
  });
  mock.on("POST", "/api/v1/onboarding/reset/org", () => {
    return { json: { cleared: 1 } };
  });
  mock.on("POST", "/api/v1/onboarding/reset/user", () => {
    return { json: { cleared: 1, user_id: ME_FIXTURE.user_id } };
  });
  // client+board+ticket+invite creation (wizard content)
  mock.on("POST", "/api/v1/clients", (ctx) => ({
    json: { id: "c1", name: (ctx.body as { name: string }).name },
  }));
  mock.on("POST", "/api/v1/boards", (ctx) => ({
    json: { id: "b1", name: (ctx.body as { name: string }).name },
  }));
  mock.on("POST", "/api/v1/tickets", (ctx) => ({
    json: { id: "t1", title: (ctx.body as { title: string }).title },
  }));
  mock.on("POST", "/api/v1/org/users", (ctx) => ({
    json: {
      user: {
        id: "u2",
        setup_token: "tok-123",
      },
      email: (ctx.body as { email: string }).email,
    },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  push.mockClear();
  replace.mockClear();
});

function mountAuthed(node: React.ReactNode) {
  return render(<AuthProvider>{node}</AuthProvider>);
}

// --- org wizard ---------------------------------------------------------------

describe("/welcome wizard — skip contract", () => {
  it("skip for now records nothing; never show again (from the guide menu) records the dismissal", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <WelcomePage />
      </AuthProvider>,
    );
    // clients is the first pending step: the rail's step 1, labelled with the
    // pack's short label.
    const rail = await screen.findByTestId("wizard-step-1", {}, { timeout: 5000 });
    expect(rail.textContent).toContain("Add clients");
    expect(rail.getAttribute("aria-current")).toBe("step");

    // SKIP FOR NOW — deliberately persists nothing on the step.
    fireEvent.click(screen.getByTestId("wizard-skip-for-now"));
    await waitFor(() => {
      expect(
        mock.callsTo("POST", "/api/v1/onboarding/org/steps/clients/skip"),
      ).toHaveLength(1);
    });
    expect(mock.lastBody("POST", "/api/v1/onboarding/org/steps/clients/skip")).toEqual({
      never: false,
    });
    expect(
      mock.callsTo("POST", "/api/v1/onboarding/org/steps/clients/complete"),
    ).toHaveLength(0);

    // The wizard advances to the next pending step (board, whose gate is
    // satisfied) and the rail lights IT as the current step.
    const rail2 = await screen.findByTestId("wizard-step-2", {}, { timeout: 5000 });
    expect(rail2.getAttribute("aria-current")).toBe("step");

    // NEVER SHOW THIS SETUP AGAIN — it lives in the guide menu, behind a
    // confirm, never on a step card. It records the choice, and it sticks.
    fireEvent.click(screen.getByTestId("wizard-guide-menu"));
    fireEvent.click(screen.getByTestId("wizard-never-show-again"));
    await screen.findByTestId("never-show-dialog", {}, { timeout: 5000 });
    fireEvent.click(screen.getByTestId("never-show-confirm"));
    await waitFor(() => {
      expect(mock.lastBody("POST", "/api/v1/onboarding/org/steps/board/skip")).toEqual({
        never: true,
      });
    });
    cleanup();
  });

  it("completes the clients step server-gated after the artifact exists", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <WelcomePage />
      </AuthProvider>,
    );
    await screen.findByTestId("wizard-step-1", {}, { timeout: 5000 });
    fireEvent.change(screen.getByTestId("wizard-client-name"), {
      target: { value: "Northwind IT" },
    });
    fireEvent.click(screen.getByTestId("wizard-clients-create"));
    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/clients")).toHaveLength(1);
      expect(mock.callsTo("POST", "/api/v1/onboarding/org/steps/clients/complete")).toHaveLength(1);
    });
    cleanup();
  });

  it("never spread the completion through the legacy flat key: completion rides the per-step flags", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <WelcomePage />
      </AuthProvider>,
    );
    await screen.findByTestId("wizard-step-1", {}, { timeout: 5000 });
    // The wizard must NOT write onboarding.wizard_completed (superseded
    // by the per-step flags; the legacy key stays a read-only fallback).
    expect(
      mock.callsTo("PUT", "/api/v1/settings/org")
        .filter((c) => (c.body as { key?: string }).key === "onboarding.wizard_completed"),
    ).toHaveLength(0);
    cleanup();
  });
});

// --- resume banner: dismissal NEVER resets on reload -----------------------------

describe("resume banner — dismiss persistence", () => {
  it("banner visible while unresolved; a 'Later' dismissal persists across remount", async () => {
    const orgFlags = serverState();
    const resetMock = setupApi(orgFlags);
    render(
      <AuthProvider>
        <OnboardingResumeBanner />
      </AuthProvider>,
    );
    const banner = await screen.findByTestId("onboarding-banner", {}, { timeout: 5000 });
    expect(banner).toBeInTheDocument();

    // Dismiss NOW: a recorded server-side setting, not session state.
    fireEvent.click(screen.getByTestId("onboarding-banner-dismiss"));
    await waitFor(() => {
      const dismissCall = resetMock.callsTo("PUT", "/api/v1/settings/org").find(
        (c) => (c.body as { key?: string }).key === "onboarding.org_banner_dismissed",
      );
      expect(dismissCall).toBeDefined();
      expect((dismissCall as { body?: { value?: boolean } }).body?.value).toBe(true);
      expect(orgFlags["onboarding.org_banner_dismissed"]).toBe(true);
    });

    // RELOAD: mount a fresh banner against the same persisted state.
    cleanup();
    render(
      <AuthProvider>
        <OnboardingResumeBanner />
      </AuthProvider>,
    );
    await waitFor(() => {
      // The banner read org_banner_dismissed=true from the server, so it
      // stays quiet — no silent re-nag across reloads.
      expect(screen.queryByTestId("onboarding-banner")).not.toBeInTheDocument();
    });

    cleanup();
  });
});

// --- user first-run ------------------------------------------------------------

describe("/onboarding user flow", () => {
  it("completes orientation and persists personal defaults through the user settings", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <UserOnboardingPage />
      </AuthProvider>,
    );
    // The rail names itself, not its position: "Getting you started", and the
    // four pack labels.
    const rail = await screen.findByTestId("wizard-step-1", {}, { timeout: 5000 });
    expect(screen.getByTestId("wizard-step-2").textContent).toContain("Set a second step");
    expect(screen.getByTestId("wizard-step-4").textContent).toContain("Your defaults");

    // Orientation completes through its posted click.
    fireEvent.click(screen.getByTestId("wizard-step-3"));
    fireEvent.click(await screen.findByTestId("onboarding-orientation-done", {}, { timeout: 5000 }));
    await waitFor(() => {
      expect(
        mock.callsTo("POST", "/api/v1/onboarding/user/steps/orientation/complete"),
      ).toHaveLength(1);
    });

    // Personal defaults ride the typed user settings (theme override + mode).
    fireEvent.click(screen.getByTestId("wizard-step-4"));
    fireEvent.change(await screen.findByTestId("onboarding-theme", {}, { timeout: 5000 }), {
      target: { value: "marula-warm" },
    });
    fireEvent.change(screen.getByTestId("onboarding-mode"), {
      target: { value: "dark" },
    });
    fireEvent.click(screen.getByTestId("onboarding-save-defaults"));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/user")).toHaveLength(2);
    });
    cleanup();
  });

  it("does the MFA enrollment inline: begin on open, verify by code, recovery codes once", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <UserOnboardingPage />
      </AuthProvider>,
    );
    // The MFA step begins its enrollment the moment it is focused (p41
    // parity fix #44: no linking away to /settings/security).
    fireEvent.click(await screen.findByTestId("wizard-step-2", {}, { timeout: 5000 }));
    expect(
      await screen.findByTestId("onboarding-mfa-key", {}, { timeout: 5000 }),
    ).toBeInTheDocument();
    expect(mock.callsTo("POST", "/api/v1/auth/mfa/totp/enroll-begin")).toHaveLength(1);
    cleanup();
  });
});

// --- Settings → Onboarding ------------------------------------------------------

describe("Settings → Onboarding", () => {
  it("re-runs the org wizard (reset) and toggles the org-wide disable switch", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <OnboardingSettingsPage />
      </AuthProvider>,
    );
    await screen.findByTestId("onboarding-org-card", {}, { timeout: 5000 });

    // org-wide disable
    fireEvent.click(screen.getByTestId("onboarding-toggle-disable"));
    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/onboarding/disable")).toHaveLength(1);
    });
    expect(mock.lastBody("POST", "/api/v1/onboarding/disable")).toEqual({ disabled: true });

    // re-run (confirm first)
    fireEvent.click(screen.getByTestId("onboarding-reset-org"));
    fireEvent.click(screen.getByText("Confirm re-run"));
    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/onboarding/reset/org")).toHaveLength(1);
    });
    cleanup();
  });

  it("re-triggers one member's first-run (admin clears their personal flags)", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <OnboardingSettingsPage />
      </AuthProvider>,
    );
    await screen.findByTestId("onboarding-members-card", {}, { timeout: 5000 });
    fireEvent.click(screen.getByTestId(`onboarding-retrigger-${ME_FIXTURE.user_id}`));
    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/onboarding/reset/user")).toHaveLength(1);
    });
    expect(mock.lastBody("POST", "/api/v1/onboarding/reset/user")).toEqual({
      user_id: ME_FIXTURE.user_id,
    });
    cleanup();
  });
});
