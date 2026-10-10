// Integration test: /scorecards — the window IS the view control (tabs beside
// the title, per the Dispatch sub-nav pattern), team visibility mirrors the
// API's own enforcement, and the surface shows no AI affordance at all
// (P4: no sparkle, no hint, no assistant stub).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import ScorecardsPage from "../../app/scorecards/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/scorecards",
}));

const MANAGER_ME = { ...ME_FIXTURE, permissions: ["special:manage_users"] };
const MEMBER_ME = { ...ME_FIXTURE, permissions: ["tickets:view"] };

function scorecard(displayName: string, userId: string) {
  return {
    display_name: displayName,
    user_id: userId,
    window: { from: "2026-09-02T00:00:00Z", to: "2026-10-02T00:00:00Z" },
    sla: {
      response_attainment: 0.95,
      response_met: 19,
      response_total: 20,
      resolution_attainment: 0.9,
      resolution_met: 18,
      resolution_total: 20,
      breached: 0,
    },
    response: { count: 20, median_seconds: 600, p95_seconds: 3600 },
    resolution: { count: 20, median_seconds: 7200, p95_seconds: 86400 },
    workload: {
      approved_minutes: 450,
      approved_entries: 12,
      billable_ratio: 0.6,
      by_work_type: [{ work_type: "remote", minutes: 450 }],
    },
    csat: { available: true, responses: 4, avg_score: 4.5, low_scores: 0 },
  };
}

function setupApi(me: Record<string, unknown>, team: Record<string, unknown>[]): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: me }));
  mock.on("GET", "/api/v1/scorecards/me", () => ({
    json: scorecard(me.display_name as string, me.user_id as string),
  }));
  mock.on("GET", "/api/v1/scorecards", () => ({ json: { scorecards: team } }));
  mock.on("GET", "/api/v1/scorecards/me/settings", () => ({
    json: { gamification_opt_in: false, user_id: me.user_id },
  }));
  mock.on("GET", "/api/v1/scorecards/clients", () => ({ json: { available: false, rollups: [] } }));
  mock.install();
  return mock;
}

/** Everything the pack forbids on an AI-free surface, as one assertion. */
function expectNoAiAffordance(): void {
  const forbidden = /ask ai|assistant|sparkle|spark|try ai|let ai|ai suggestion|generate/i;
  for (const node of Array.from(document.querySelectorAll("body *"))) {
    const text = node.textContent?.trim() ?? "";
    if (text.length === 0) continue;
    expect(text).not.toMatch(forbidden);
  }
  expect(document.querySelector(".m-assistant-stub")).toBeNull();
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/scorecards integration", () => {
  it("uses the window tabs as the view control — switching re-queries the window", async () => {
    const mock = setupApi(MANAGER_ME, [
      scorecard("Riley Chen", "00000000-0000-4000-8000-000000000003"),
    ]);
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <ScorecardsPage />
      </AuthProvider>,
    );
    await screen.findByText("Owner");
    await waitFor(() => {
      expect(mock.callsTo("GET", "/api/v1/scorecards/me").length).toBeGreaterThan(0);
    });
    const first = mock.callsTo("GET", "/api/v1/scorecards/me");
    const firstQuery = first[0]!.query as { from?: string; to?: string };
    expect(firstQuery.from).toBeDefined();

    // The window tabs sit beside the title; switching one re-queries with a
    // new from date — the window IS the view control, never a select.
    await user.click(screen.getByRole("tab", { name: "Last 7 days" }));
    await waitFor(() => {
      expect(mock.callsTo("GET", "/api/v1/scorecards/me").length).toBeGreaterThan(first.length);
    });
    const second = mock.callsTo("GET", "/api/v1/scorecards/me");
    const secondQuery = second[second.length - 1]!.query as { from?: string; to?: string };
    expect(secondQuery.from).not.toBe(firstQuery.from);
    cleanup();
  });

  it("shows the team to a manager and the own card only to a member", async () => {
    setupApi(MANAGER_ME, [scorecard("Riley Chen", "00000000-0000-4000-8000-000000000003")]);
    render(
      <AuthProvider>
        <ScorecardsPage />
      </AuthProvider>,
    );
    await screen.findByText("Riley Chen");
    expect(screen.getByText("Team visible")).toBeInTheDocument();
    cleanup();

    setupApi(MEMBER_ME, []);
    render(
      <AuthProvider>
        <ScorecardsPage />
      </AuthProvider>,
    );
    await screen.findByText("Owner");
    expect(screen.queryByText("Team visible")).not.toBeInTheDocument();
    expect(screen.queryByText("Riley Chen")).not.toBeInTheDocument();
    cleanup();
  });

  it("shows no AI affordance anywhere on the surface", async () => {
    setupApi(MANAGER_ME, [scorecard("Riley Chen", "00000000-0000-4000-8000-000000000003")]);
    render(
      <AuthProvider>
        <ScorecardsPage />
      </AuthProvider>,
    );
    await screen.findByText("Riley Chen");
    expectNoAiAffordance();
    cleanup();
  });
});
