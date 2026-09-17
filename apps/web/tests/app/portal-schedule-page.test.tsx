// Integration test: /portal/schedule — day-grouped slot picking + intent
// semantics. Covers day grouping, the selected-time confirm card, the
// create call's payload shape, and request status labels (Wave 17 lane 8).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import PortalSchedulePage from "../../app/portal/schedule/page";
import { PortalAuthProvider } from "../../lib/portal-api";
import { MockApi } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/portal/schedule",
}));

function seedPortalTokens(): void {
  window.localStorage.setItem(
    "meridian-portal-tokens",
    JSON.stringify({
      access_token: "a",
      refresh_token: "r",
      session_id: "s",
      expires_in: 3600,
    }),
  );
}

const DAY1 = new Date(Date.now() + 86_400_000).toISOString().slice(0, 10);
const DAY2 = new Date(Date.now() + 2 * 86_400_000).toISOString().slice(0, 10);

function slots(hour: number): { starts_at: string; ends_at: string } {
  return {
    starts_at: `${DAY1}T0${hour % 10}:00:00.000Z`,
    ends_at: `${DAY1}T0${(hour % 10) + 1}:00:00.000Z`,
  };
}

const STAFF_SLOTS = [
  {
    user_id: "u9",
    display_name: "Sam Weaver",
    slots: [slots(13), slots(15)],
  },
];

const REQUESTS = [
  {
    id: "r1",
    user_id: "u9",
    title: "Quarterly review",
    starts_at: `${DAY2}T14:00:00.000Z`,
    ends_at: `${DAY2}T15:00:00.000Z`,
    kind: "remote",
    notes: null,
    status: "pending",
    decision_note: null,
    created_at: new Date().toISOString(),
  },
];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/portal/me", () => ({
    json: {
      id: "u1",
      org_id: "o1",
      client_id: "c1",
      email: "ops@acme.com",
      display_name: "Ops User",
      role: "contact",
    },
  }));
  mock.on("GET", "/api/v1/portal/schedule/slots", () => ({ json: STAFF_SLOTS }));
  mock.on("GET", "/api/v1/portal/appointment-requests", () => ({ json: REQUESTS }));
  mock.on("POST", "/api/v1/portal/appointment-requests", () => ({
    json: { id: "r2", status: "pending" },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedPortalTokens();
});

describe("/portal/schedule", () => {
  it("groups published slots by day and lists requests with labels", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalSchedulePage />
      </PortalAuthProvider>,
    );
    expect(await screen.findByText("Tomorrow")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /with Sam Weaver/ }).length).toBe(2);
    expect(screen.getByText("Your requests")).toBeInTheDocument();
    expect(screen.getByText("Quarterly review")).toBeInTheDocument();
    expect(screen.getAllByText("Pending").length).toBeGreaterThanOrEqual(1);
    expect(mock.calls.some((c) => c.path === "/api/v1/portal/schedule/slots")).toBe(true);
    cleanup();
  });

  it("carries the selected slot straight into the typed create call", async () => {
    const mock = setupApi();
    render(
      <PortalAuthProvider>
        <PortalSchedulePage />
      </PortalAuthProvider>,
    );
    await screen.findByText("Tomorrow");
    const pick = screen.getAllByRole("button", { name: /with Sam Weaver/ })[0];
    fireEvent.click(pick as HTMLElement);
    const summary = screen.getByTestId("picked-slot-summary");
    expect(summary.textContent).toContain("Sam Weaver");

    fireEvent.change(screen.getByLabelText("What is it about?"), {
      target: { value: "Config change" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send request" }));
    await waitFor(() => {
      expect(screen.getByRole("status").textContent).toContain("Request sent");
    });
    const body = mock.lastBody<Record<string, unknown>>("POST", "/api/v1/portal/appointment-requests");
    expect(body.title).toBe("Config change");
    expect(body.user_id).toBe("u9");
    expect((body.idempotency_key as string).startsWith("portal-req-")).toBe(true);
    cleanup();
  });
});
