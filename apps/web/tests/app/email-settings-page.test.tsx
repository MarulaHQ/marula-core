// Integration test: /settings/email (Wave 34 lane 8) — the honest
// not-configured state, the sealed-credential presence signal (never
// material), the delivery ledger note, and friendly labels for every
// notification kind in the workspace-defaults table.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import EmailSettingsPage from "../../app/settings/email/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/email",
}));

function mailSettings(overrides: Record<string, unknown> = {}) {
  return {
    inbound: {
      catchall_address: null,
      catchall_board_id: null,
      enabled: false,
    },
    outbound: {
      transport: "none",
      from_address: "",
      from_name: "",
      reply_to: "",
      signature: "",
      brand_name: "",
      logo_url: "",
      smtp_host: "",
      smtp_port: 587,
      smtp_username: "",
      postmark_base_url: "https://api.postmarkapp.com",
      ready: false,
      not_ready_reason:
        "no outbound transport is selected — mail stays queued until one is",
      credentials: {
        postmark: { configured: false, updated_at: null },
        smtp: { configured: false, updated_at: null },
      },
    },
    boards: [
      {
        board_id: "00000000-0000-4000-8000-000000000010",
        name: "Service Desk",
        inbound_address: null,
        identity: {
          from_address: null,
          from_name: null,
          reply_to: null,
          signature: null,
        },
      },
    ],
    reply_by_email: { enabled: false, catchall_address: null, board_addresses: 0 },
    digest_policy: "allowed",
    ...overrides,
  };
}

function ledger(overrides: Record<string, unknown> = {}) {
  return {
    rows: [],
    counts: { queued: 0, sending: 0, sent: 0, dead: 0 },
    next_cursor: null,
    transport_configured: false,
    ...overrides,
  };
}

function setupApi(settings: unknown, ledgerView: unknown, defaults: unknown): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/email/settings", () => ({ json: settings }));
  mock.on("GET", "/api/v1/email/delivery-ledger", () => ({ json: ledgerView }));
  mock.on("GET", "/api/v1/notification-org-defaults", () => ({ json: defaults }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/email integration", () => {
  it("renders the typed not-configured state and never fakes a send", async () => {
    setupApi(
      mailSettings(),
      ledger({
        counts: { queued: 2, sending: 0, sent: 0, dead: 0 },
        rows: [
          {
            message_id: "00000000-0000-4000-8000-0000000000aa",
            subject: "New ticket assigned",
            to_addresses: ["tech@acme.test"],
            from_address: "support@acme.test",
            status: "queued",
            attempts: 0,
            max_attempts: 8,
            last_error: null,
            created_at: "2026-09-26T09:00:00Z",
            rfc_message_id: "m1@acme.test",
          },
        ],
      }),
      { defaults: [] },
    );
    render(
      <AuthProvider>
        <EmailSettingsPage />
      </AuthProvider>,
    );
    // The reason line the backend supplied renders verbatim.
    expect(
      await screen.findByText(/no outbound transport is selected/i),
    ).toBeInTheDocument();
    // The ledger explains queued mail instead of implying zero activity.
    expect(
      screen.getByText(/No transport is configured, so nothing has sent yet/i),
    ).toBeInTheDocument();
    // The row shows its state, and the probe is refused before it is tried.
    expect(screen.getByText("New ticket assigned")).toBeInTheDocument();
    expect(screen.getAllByText("queued").length).toBeGreaterThan(0);
    const probe = screen.getByRole("button", { name: "Send test email" });
    expect(probe).toBeDisabled();
    cleanup();
  });

  it("shows sealed credential presence and friendly labels for every kind", async () => {
    setupApi(
      mailSettings({
        outbound: {
          ...mailSettings().outbound,
          transport: "smtp",
          smtp_host: "smtp.acme.test",
          ready: true,
          not_ready_reason: null,
          credentials: {
            postmark: { configured: false, updated_at: null },
            smtp: { configured: true, updated_at: "2026-09-26T09:10:00Z" },
          },
        },
      }),
      ledger({ transport_configured: true }),
      {
        defaults: [
          {
            kind: "report_delivery",
            in_app: true,
            email: true,
            quiet_from: null,
            quiet_until: null,
            timezone: "UTC",
          },
          {
            kind: "stock_low",
            in_app: true,
            email: false,
            quiet_from: null,
            quiet_until: null,
            timezone: "UTC",
          },
        ],
      },
    );
    render(
      <AuthProvider>
        <EmailSettingsPage />
      </AuthProvider>,
    );
    // Presence-only: sealed state shows, the credential value never does.
    expect(await screen.findByText("Sealed")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Replace sealed password")).toBeInTheDocument();
    // Friendly labels for the kinds the audit flagged as raw slugs.
    expect(screen.getByText("Report delivery")).toBeInTheDocument();
    expect(screen.getByText("Low stock")).toBeInTheDocument();
    expect(screen.queryByText("report_delivery")).not.toBeInTheDocument();
    const probe = screen.getByRole("button", { name: "Send test email" });
    expect(probe).toBeEnabled();
    cleanup();
  });
});
