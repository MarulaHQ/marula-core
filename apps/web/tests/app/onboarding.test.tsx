// Wave 26 onboarding integration tests — the SKIPPABILITY CONTRACT.
//
// Purpose: the skip path is the happy path, first class. These tests pin
// the recorded-state semantics of every surface:
//   • org wizard (/welcome): "Skip for now" records NOTHING on the step
//     (POST skip body never:false, no step mutation); "Never show this
//     again" records the dismissed flag; completion is server-gated
//     (POST complete only fires after the gate is satisfied).
//   • resume banner: a dismissal persists across a RELOAD (remount) —
//     the read comes back dismissed server-side, so the banner stays
//     silent. The record is the state.
//   • user first-run (/onboarding): orientation completes; personal
//     defaults persist through PUT /api/v1/settings/user.
//   • Settings → Onboarding: org re-run (reset/org), org-wide disable
//     toggle (disable), member re-trigger (reset/user).
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
    satisfied: step === "board", // board artifact exists in every fixture
    resolved:
      orgFlags[`onboarding.org_${step}_done`] === true ||
      orgFlags[`onboarding.org_${step}_dismissed`] === true ||
      step === "board",
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

/** Installs every real route the onboarding family reads/writes. */
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
  mock.on("GET", "/api/v1/settings/org", () => ({ json: allOrgFlags(orgFlags) }));
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

function allOrgFlags(orgFlags: Record<string, boolean>) {
  return Object.entries(orgFlags).map(([key, value]) => ({ key, value }));
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
  it("satisfied steps never-nag: nothing is recorded when 'never' is false, everything is recorded when true", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <WelcomePage />
      </AuthProvider>,
    );
    // clients is the first pending step
    await screen.findByTestId("wizard-step-clients", {}, { timeout: 5000 });

    // SKIP FOR NOW — deliberately persists nothing on the step
    fireEvent.click(screen.getByTestId("onboarding-skip-for-now"));
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
    // advance to the next pending step (board, whose gate is satisfied)
    await screen.findByTestId("wizard-step-board", {}, { timeout: 5000 });

    // NEVER SHOW THIS AGAIN — recorded, sticks
    fireEvent.click(screen.getByTestId("onboarding-never-show"));
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
    await screen.findByTestId("wizard-step-clients", {}, { timeout: 5000 });
    fireEvent.change(screen.getByPlaceholderText("Client name"), {
      target: { value: "Northwind IT" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create client" }));
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
    await screen.findByTestId("wizard-step-clients", {}, { timeout: 5000 });
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
    // the first pending step is "mfa" (profile is satisfied in the fixture)
    await screen.findByTestId("onboarding-step-mfa", {}, { timeout: 5000 });

    // Orientation completes through the posted click.
    fireEvent.click(screen.getByTestId("onboarding-rail-orientation"));
    fireEvent.click(screen.getByRole("button", { name: /got it/i }));
    await waitFor(() => {
      expect(
        mock.callsTo("POST", "/api/v1/onboarding/user/steps/orientation/complete"),
      ).toHaveLength(1);
    });

    // Personal defaults ride the typed user settings (theme override + mode).
    fireEvent.change(screen.getByTestId("onboarding-theme"), {
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
});


// --- welcome hero: the first-open animated theme picker (user directive) -----

describe("first-open welcome hero", () => {
  it("renders the theme pick and completes the defaults step through it", async () => {
    const mock = setupApi();
    // Install the theme-packs catalog route the hero reads.
    mock.on("GET", "/api/v1/plugins/distribution/theme-packs", () => ({
      json: { packs: [{ slug: "ember-dusk", manifest: { light: { "--m-accent": "#7C5CFF" } } }] },
    }));
    render(
      <AuthProvider>
        <UserOnboardingPage />
      </AuthProvider>,
    );
    // The hero is the first thing on the page while defaults are pending.
    await screen.findByTestId("onboarding-welcome", {}, { timeout: 5000 });
    expect(screen.getByTestId("welcome-mode-light")).toBeInTheDocument();
    expect(screen.getByTestId("welcome-mode-dark")).toBeInTheDocument();
    expect(screen.getByTestId("welcome-theme-marula")).toBeInTheDocument();
    expect(screen.getByTestId("welcome-theme-marula-warm")).toBeInTheDocument();
    // Admin-added theme packs render as extra preview cards.
    await waitFor(() => {
      expect(screen.getByTestId("welcome-pack-ember-dusk")).toBeInTheDocument();
    });

    // Choosing DARK persists both Wave 24 keys (mode + override default).
    fireEvent.click(screen.getByTestId("welcome-mode-dark"));
    await waitFor(() => {
      const modeCall = mock.callsTo("PUT", "/api/v1/settings/user").find(
        (c) => (c.body as { key?: string }).key === "appearance.mode",
      );
      expect((modeCall as { body?: { value?: string } }).body?.value).toBe("dark");
    });
    // Confirm completes the recorded defaults step (the settings PUTs already
    // happened through the hero itself).
    fireEvent.click(screen.getByTestId("welcome-confirm"));
    await waitFor(() => {
      expect(
        mock.callsTo("POST", "/api/v1/onboarding/user/steps/defaults/complete"),
      ).toHaveLength(1);
    });
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
