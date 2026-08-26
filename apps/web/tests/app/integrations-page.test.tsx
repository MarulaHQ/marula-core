// Integration test: /settings/integrations — webhook create with event
// multiselect (wildcard-empty payload), show-once secret, paged delivery
// log, rotate flow (create replacement → delete original), test-fire.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import IntegrationsSettingsPage from "../../app/settings/integrations/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/integrations",
}));

const WH_ID = "50000000-0000-4000-8000-000000000001";

function webhookView(overrides: Record<string, unknown> = {}) {
  return {
    id: WH_ID,
    url: "https://hooks.example.test/meridian",
    event_types: [] as string[],
    enabled: true,
    created_at: "2026-01-01T00:00:00Z",
    created_by: null,
    ...overrides,
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/webhooks", () => ({ json: { webhooks: [webhookView()] } }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/integrations integration", () => {
  it("creates a webhook from the multiselect and reveals the signing secret exactly once", async () => {
    const user = userEvent.setup();
    const mock = setupApi();
    mock.on("POST", "/api/webhooks", ({ body }) => ({
      status: 201,
      json: { secret: "whsec-live-value", webhook: webhookView({ id: WH_ID + "-new" }) },
    }));

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );

    await screen.findByRole("button", { name: "Delivery log" }); // list loaded

    await user.type(screen.getByLabelText("Endpoint URL"), "https://new.endpoint/hook");
    fireEvent.click(screen.getByLabelText("ticket.created"));
    await user.type(screen.getByLabelText("Custom event type"), "custom.merge");
    fireEvent.click(screen.getByRole("button", { name: "Add type" }));
    fireEvent.click(screen.getByRole("button", { name: "Create webhook" }));

    await waitFor(() => {
      const posts = mock.callsTo("POST", "/api/webhooks");
      expect(posts.length).toBe(1);
      expect(posts[0]?.body).toEqual({
        url: "https://new.endpoint/hook",
        event_types: ["custom.merge", "ticket.created"],
      });
    });
    // Show-once: raw secret visible now.
    expect(await screen.findByText("whsec-live-value")).toBeInTheDocument();
    // Dismiss → never re-fetched nor re-rendered.
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("whsec-live-value")).not.toBeInTheDocument();
    cleanup();
  });

  it("pages the delivery log by keyset cursor and appends the older page", async () => {
    const mock = setupApi();
    const attempts = (n: number, attempt: number) =>
      Array.from({ length: n }, (_, i) => ({
        id: `d-${attempt}-${i}`,
        event_id: `e-${attempt}`,
        event_type: "ticket.created",
        attempt,
        response_status: 200,
        next_retry_at: null,
        delivered_at: "2026-01-02T03:04:05Z",
        last_error: null,
        created_at: "2026-01-02T03:04:05Z",
      }));
    let deliveryCalls = 0;
    mock.on("GET", `/api/webhooks/${WH_ID}/deliveries`, () => {
      deliveryCalls += 1;
      if (deliveryCalls === 1) {
        return { json: { deliveries: attempts(20, 1), next_cursor: "cursor-page-2" } };
      }
      return { json: { deliveries: attempts(5, 2), next_cursor: null } };
    });

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Delivery log" }));
    await screen.findAllByText(/attempt #1/);
    fireEvent.click(screen.getByRole("button", { name: "Load older" }));
    await waitFor(() => {
      const rows = mock.callsTo("GET", `/api/webhooks/${WH_ID}/deliveries`);
      expect(rows.length).toBe(2);
      expect(rows[1]?.query["cursor"]).toBe("cursor-page-2");
    });
    expect(await screen.findByText("Beginning of log.")).toBeInTheDocument();
    cleanup();
  });

  it("rotates by creating a replacement with identical config then deleting the original", async () => {
    const mock = setupApi();
    // Empty selection on the seeded webhook means wildcard subscription;
    // rotation must carry that config over unchanged.
    mock.on("POST", "/api/webhooks", ({ body }) => {
      expect(body).toEqual({
        url: "https://hooks.example.test/meridian",
        event_types: [],
      });
      return {
        status: 201,
        json: {
          secret: "rotated-secret",
          webhook: webhookView({ id: WH_ID + "-r2" }),
        },
      };
    });
    mock.on("DELETE", `/api/webhooks/${WH_ID}`, () => ({ status: 204, json: null }));

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Rotate" }));
    fireEvent.click(await screen.findByRole("button", { name: "Rotate now" }));

    await screen.findByText("rotated-secret");
    expect(mock.callsTo("DELETE", `/api/webhooks/${WH_ID}`).length).toBe(1);
    cleanup();
  });

  it("surfaces test-fire envelope ids next to the log affordance", async () => {
    const mock = setupApi();
    mock.on("POST", `/api/webhooks/${WH_ID}/test`, () => ({
      status: 202,
      json: { event_id: "test-envelope-id-42" },
    }));

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Send test" }));
    await screen.findByText(/Test envelope/);
    const posts = mock.callsTo("POST", `/api/webhooks/${WH_ID}/test`);
    expect(posts.length).toBe(1);
    cleanup();
  });
});
