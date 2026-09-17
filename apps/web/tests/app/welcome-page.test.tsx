// Integration test: /welcome — the three-tier onboarding surface.
// Tier 1 (org profile) is wired through PATCH /org + the completion flag;
// tiers 2 (module activation) and 3 (guided playbooks) render honest
// "Not available yet" posture with no fake controls.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import WelcomePage from "../../app/welcome/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const push = vi.fn();
const replace = vi.fn();

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace, push }),
  usePathname: () => "/welcome",
}));

const ORG_FIXTURE = {
  id: ME_FIXTURE.org_id,
  name: "Meridian Demo",
  ticket_prefix: "MER",
};

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/org", () => ({ json: ORG_FIXTURE }));
  mock.on("PATCH", "/api/v1/org", (ctx) => ({
    json: { ...ORG_FIXTURE, ...(ctx.body as object) },
  }));
  mock.on("PUT", "/api/v1/settings/org", (ctx) => ({
    json: { key: (ctx.body as { key: string }).key, value: (ctx.body as { value: string }).value },
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

function mount() {
  return render(
    <AuthProvider>
      <WelcomePage />
    </AuthProvider>,
  );
}

/** Mounts and waits for the org-profile prefill so tests never race it. */
async function mountPrefilled() {
  const utils = mount();
  await screen.findByLabelText("Organization name", {}, { timeout: 5000 });
  await waitFor(() => {
    expect(screen.getByLabelText("Organization name")).toHaveValue("Meridian Demo");
    expect(screen.getByLabelText("Ticket prefix")).toHaveValue("MER");
  });
  return utils;
}

describe("/welcome integration", () => {
  it("prefills the org profile and saves through PATCH /org + the wizard flag", async () => {
    const mock = setupApi();
    await mountPrefilled();

    const name = screen.getByLabelText("Organization name");
    const prefix = screen.getByLabelText("Ticket prefix");

    fireEvent.change(name, { target: { value: "Northwind IT" } });
    fireEvent.click(screen.getByRole("button", { name: "Save and continue" }));

    await waitFor(() => {
      expect(mock.callsTo("PATCH", "/api/v1/org")).toHaveLength(1);
    });
    expect(mock.lastBody("PATCH", "/api/v1/org")).toEqual({
      name: "Northwind IT",
      ticket_prefix: "MER",
    });
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/org")).toHaveLength(1);
    });
    expect(mock.lastBody<{ key: string }>("PUT", "/api/v1/settings/org").key).toBe(
      "onboarding.wizard_completed",
    );

    // Tier 2 renders as honest posture, not a fake activation flow.
    expect(await screen.findByText("Activate modules")).toBeInTheDocument();
    expect(screen.getAllByText("Not available yet")).toHaveLength(4);
    expect(screen.queryByRole("button", { name: "Activate" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Enable" })).not.toBeInTheDocument();
    cleanup();
  });

  it("walks tier 3 and lands on /today via Enter Meridian", async () => {
    setupApi();
    await mountPrefilled();

    fireEvent.click(screen.getByRole("button", { name: "Save and continue" }));
    fireEvent.click(await screen.findByRole("button", { name: "Continue" }));
    expect(await screen.findByText("Guided playbooks")).toBeInTheDocument();
    expect(screen.getByText("Not available yet")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Enter Meridian" }));
    expect(replace).toHaveBeenCalledWith("/today");
    cleanup();
  });

  it("skips without touching the org profile but still sets the completion flag", async () => {
    const mock = setupApi();
    await mountPrefilled();

    fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));
    await waitFor(() => {
      expect(mock.callsTo("PUT", "/api/v1/settings/org")).toHaveLength(1);
    });
    expect(mock.callsTo("PATCH", "/api/v1/org")).toHaveLength(0);
    expect(replace).toHaveBeenCalledWith("/today");
    cleanup();
  });
});
