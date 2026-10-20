// Integration test: /settings/notifications — the kind list is the server's
// list. A kind the worker notifier writes (stock_low) therefore gets a row and
// can be turned off, instead of disappearing because a hardcoded list or a
// Rust `ALL` omits it.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import NotificationPreferencesPage from "../../app/settings/notifications/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/notifications",
}));

// Mirrors `NotificationKind::ALL` (13 kinds, stable order).
const KINDS = [
  "mention",
  "assignment",
  "sla_risk",
  "sla_breach",
  "escalation",
  "missing_time",
  "ticket_update",
  "report_delivery",
  "ai_cap_warn",
  "secret_rotation",
  "stock_low",
  "dedupe_report",
  "scan_detection",
];

function pref(kind: string, overrides: Record<string, unknown> = {}) {
  return {
    kind,
    source: "builtin",
    in_app: true,
    email: true,
    quiet_from: null,
    quiet_until: null,
    timezone: "UTC",
    ...overrides,
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/settings/user", () => ({ json: [] }));
  mock.on("GET", "/api/v1/notification-preferences", () => ({
    json: KINDS.map((kind) => pref(kind)),
  }));
  mock.on("PUT", "/api/v1/notification-preferences", ({ body }) => ({
    json: pref((body as { kind: string }).kind, body as Record<string, unknown>),
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/notifications kind list", () => {
  it("renders one row per kind the server returns, stock_low included", async () => {
    setupApi();
    render(
      <AuthProvider>
        <NotificationPreferencesPage />
      </AuthProvider>,
    );

    // The header row plus one body row per kind; nothing is dropped or added
    // client-side.
    await screen.findByLabelText("Low stock email");
    expect(screen.getAllByRole("row")).toHaveLength(KINDS.length + 1);
    expect(screen.getByText("Low stock")).toBeInTheDocument();
  });

  it("sends stock_low when its email toggle is turned off", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <NotificationPreferencesPage />
      </AuthProvider>,
    );

    const email = await screen.findByLabelText("Low stock email");
    expect(email).toBeChecked();

    fireEvent.click(email);
    expect(email).not.toBeChecked();

    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(
        mock.callsTo("PUT", "/api/v1/notification-preferences").length,
      ).toBe(1),
    );
    expect(
      mock.lastBody("PUT", "/api/v1/notification-preferences"),
    ).toMatchObject({
      kind: "stock_low",
      email: false,
      in_app: true,
    });
  });
});
