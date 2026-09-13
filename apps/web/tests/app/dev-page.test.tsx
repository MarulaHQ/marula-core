// Integration test: /dev ops page render-gating (TASKLIST DX3).
//
// Purpose: the page must mirror the API's env gate — a 404/missing
// /api/dev/status renders ONLY the disabled notice (no action buttons),
// while an enabled probe unlocks the QA controls and its buttons hit the
// documented dev paths.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import DevPage from "../../app/dev/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/dev",
}));

function setupApi(opts: { enabled: boolean }): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/dev/status", () =>
    opts.enabled
      ? {
          json: {
            enabled: true,
            git_sha: "abc1234",
            migration_head: 45,
            demo_seeded: false,
            demo_org_id: "a0000000-0000-4000-8000-00000000d001",
            demo_slug: "meridian-demo",
          },
        }
      : { status: 404, json: { error: { message: "not found" } } },
  );
  mock.on("POST", "/api/dev/seed", () => ({
    json: { org_id: "a0000000-0000-4000-8000-00000000d001", tickets: 15 },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/dev ops page render-gating", () => {
  it("renders only the disabled notice when the API gate is off", async () => {
    setupApi({ enabled: false });
    render(
      <AuthProvider>
        <DevPage />
      </AuthProvider>,
    );

    await screen.findByText(/Dev tools are not enabled/);
    expect(screen.queryByRole("button", { name: "Seed demo org" })).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reset org (wipe + reseed)" }),
    ).not.toBeInTheDocument();
    cleanup();
  });

  it("unlocks the controls when the probe answers enabled and Seed hits the dev path", async () => {
    const mock = setupApi({ enabled: true });
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DevPage />
      </AuthProvider>,
    );

    // Build info from the status probe.
    await screen.findByText(/"migration_head": 45/);
    const seedButton = screen.getByRole("button", { name: "Seed demo org" });
    await user.click(seedButton);

    await waitFor(() => {
      const calls = mock.callsTo("POST", "/api/dev/seed");
      expect(calls).toHaveLength(1);
    });
    await screen.findByText(/"tickets": 15/);
    cleanup();
  });

  it("renders the typed already_seeded reason and unlocks the force-reseed second click", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.on("GET", "/api/dev/status", () => ({
      json: {
        enabled: true,
        git_sha: "abc1234",
        migration_head: 45,
        demo_seeded: true,
        demo_org_id: "a0000000-0000-4000-8000-00000000d001",
        demo_slug: "meridian-demo",
      },
    }));
    mock.on("POST", "/api/dev/seed", (ctx) =>
      ctx.query.force === "1"
        ? { json: { org_id: "a0000000-0000-4000-8000-00000000d001", tickets: 15 } }
        : {
            status: 409,
            json: {
              error: {
                code: 409,
                reason: "already_seeded",
                message: "demo org already seeded; POST /api/dev/seed?force=1 to wipe and reseed",
              },
            },
          },
    );
    mock.install();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DevPage />
      </AuthProvider>,
    );

    await screen.findByText(/"migration_head": 45/);
    await user.click(screen.getByRole("button", { name: "Seed demo org" }));

    // Friendly copy derived from the typed reason — not the raw constraint
    // prose, and no dead end: the force-reseed control appears.
    await screen.findByText(/Force-reseed wipes it and starts over/i);
    const forceButton = screen.getByRole("button", { name: /Wipe and reseed/i });
    await user.click(forceButton);

    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/dev/seed")).toHaveLength(2);
    });
    // The second call is the typed force replay (`?force=1`).
    expect(mock.callsTo("POST", "/api/dev/seed")[1]?.query.force).toBe("1");
    cleanup();
  });
});
