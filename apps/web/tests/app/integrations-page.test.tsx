// Integration test: /settings/integrations — webhook create with event
// multiselect (wildcard-empty payload), registry refusal copy, show-once
// secret, paged delivery log, rotate flow (create replacement → delete
// original), test-fire, and the inbound-source surface (register with a
// show-once secret, ingest log off the audit ledger for generic receivers
// and the alerts ledger for the alert profile).
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
const SRC_ID = "70000000-0000-4000-8000-000000000001";
const BOARD_ID = "00000000-0000-4000-8000-000000000003";

function webhookView(overrides: Record<string, unknown> = {}) {
  return {
    id: WH_ID,
    url: "https://hooks.example.test/marula",
    event_types: [] as string[],
    enabled: true,
    created_at: "2026-01-01T00:00:00Z",
    created_by: null,
    ...overrides,
  };
}

function sourceView(overrides: Record<string, unknown> = {}) {
  return {
    id: SRC_ID,
    org_id: "00000000-0000-4000-8000-000000000002",
    name: "RMM production",
    source_ref: "rmm-prod",
    board_id: BOARD_ID,
    client_id: null,
    correlation_window_secs: 900,
    enabled: true,
    event_type: null,
    payload_schema_ref: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function auditEvent(overrides: Record<string, unknown> = {}) {
  return {
    id: "a-1",
    org_id: "00000000-0000-4000-8000-000000000002",
    actor: { Connector: "alert-source:rmm-prod" },
    action: "webhook.ingested",
    entity_type: "webhook",
    entity_id: "envelope-1",
    detail: { source_ref: "rmm-prod", source_id: SRC_ID, dedup_key: "prov-123" },
    ip: null,
    occurred_at: "2026-01-02T03:04:05Z",
    ...overrides,
  };
}

function alertRow(overrides: Record<string, unknown> = {}) {
  return {
    id: "al-1",
    org_id: "00000000-0000-4000-8000-000000000002",
    source_id: SRC_ID,
    title: "Disk almost full",
    description: null,
    severity: "warning",
    status: "open",
    dedup_key: "prov-123",
    correlation_key: "k-1",
    incident_ticket_id: null,
    received_at: "2026-01-02T03:04:05Z",
    occurred_at: "2026-01-02T03:04:05Z",
    acked_at: null,
    acked_by: null,
    resolved_at: null,
    resolved_by: null,
    attributes: {},
    ...overrides,
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/webhooks", () => ({ json: { webhooks: [webhookView()] } }));
  mock.on("GET", "/api/v1/alert-sources", () => ({ json: { sources: [] } }));
  mock.on("GET", "/api/v1/boards/nav", () => ({
    json: [{ id: BOARD_ID, name: "Support" }],
  }));
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
    mock.on("POST", "/api/v1/webhooks", ({ body }) => ({
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
      const posts = mock.callsTo("POST", "/api/v1/webhooks");
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
    mock.on("GET", `/api/v1/webhooks/${WH_ID}/deliveries`, () => {
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
      const rows = mock.callsTo("GET", `/api/v1/webhooks/${WH_ID}/deliveries`);
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
    mock.on("POST", "/api/v1/webhooks", ({ body }) => {
      expect(body).toEqual({
        url: "https://hooks.example.test/marula",
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
    mock.on("DELETE", `/api/v1/webhooks/${WH_ID}`, () => ({ status: 204, json: null }));

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Rotate" }));
    fireEvent.click(await screen.findByRole("button", { name: "Rotate now" }));

    await screen.findByText("rotated-secret");
    expect(mock.callsTo("DELETE", `/api/v1/webhooks/${WH_ID}`).length).toBe(1);
    cleanup();
  });

  it("surfaces test-fire envelope ids next to the log affordance", async () => {
    const mock = setupApi();
    mock.on("POST", `/api/v1/webhooks/${WH_ID}/test`, () => ({
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
    const posts = mock.callsTo("POST", `/api/v1/webhooks/${WH_ID}/test`);
    expect(posts.length).toBe(1);
    cleanup();
  });

  it("names the ungranted types when the registry refuses the subscription", async () => {
    const user = userEvent.setup();
    const mock = setupApi();
    mock.on("POST", "/api/v1/webhooks", () => ({
      status: 422,
      json: {
        error: {
          reason: "validation_failed",
          message:
            'unknown or ungranted event types ["rmm.device_offline"]; either subscribe only to platform types, or obtain a grant for custom types',
        },
      },
    }));

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByRole("button", { name: "Delivery log" }); // list loaded

    await user.type(screen.getByLabelText("Endpoint URL"), "https://new.endpoint/hook");
    await user.type(screen.getByLabelText("Custom event type"), "rmm.device_offline");
    fireEvent.click(screen.getByRole("button", { name: "Add type" }));
    fireEvent.click(screen.getByRole("button", { name: "Create webhook" }));

    expect(
      await screen.findByText(/The event registry refused “rmm\.device_offline”/),
    ).toBeInTheDocument();
    const posts = mock.callsTo("POST", "/api/v1/webhooks");
    expect(posts.length).toBe(1);
    // The refusal copy rides the wire payload through unchanged.
    expect(posts[0]?.body).toEqual({
      url: "https://new.endpoint/hook",
      event_types: ["rmm.device_offline"],
    });
    // No secret panel — the create was refused.
    expect(screen.queryByTestId("source-secret-panel")).not.toBeInTheDocument();
    cleanup();
  });
});

describe("/settings/integrations — inbound sources", () => {
  it("registers a generic source and reveals its signing secret exactly once", async () => {
    const user = userEvent.setup();
    const mock = setupApi();
    const created = sourceView({ event_type: "rmm.device_offline" });
    let registered = false;
    // The roster only carries the source after the create posts back.
    mock.on("GET", "/api/v1/alert-sources", () => ({
      json: { sources: registered ? [created] : [] },
    }));
    mock.on("POST", "/api/v1/alert-sources", ({ body }) => {
      registered = true;
      const key = (body as { idempotency_key?: string }).idempotency_key;
      return {
        status: 201,
        json: { ...created, mutation: { idempotency_key: key }, secret: "srcsec-live-value" },
      };
    });

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByTestId("register-source"));

    await user.type(screen.getByLabelText("Name"), "RMM production");
    await user.type(screen.getByLabelText("Declared event type"), "rmm.device_offline");

    // Board picker — Radix select driven through its combobox role.
    const boardPicker = screen.getByLabelText("Board");
    await user.click(boardPicker);
    await user.click(await screen.findByRole("option", { name: "Support" }));

    fireEvent.click(screen.getByRole("button", { name: "Register" }));

    // Show-once: the secret and the ingest path are visible now.
    expect(await screen.findByTestId("source-secret-value")).toHaveTextContent("srcsec-live-value");
    const posts = mock.callsTo("POST", "/api/v1/alert-sources");
    expect(posts.length).toBe(1);
    expect(posts[0]?.body).toMatchObject({
      name: "RMM production",
      board_id: BOARD_ID,
      event_type: "rmm.device_offline",
    });
    expect(typeof (posts[0]?.body as { idempotency_key?: string }).idempotency_key).toBe("string");

    // The source now lists with its generic-receiver ingest URL.
    const ingestUrl = await screen.findByTestId(`ingest-url-${SRC_ID}`);
    expect(ingestUrl.textContent).toContain("/api/v1/webhooks/ingest/rmm-prod");

    // Dismiss → never re-rendered.
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByTestId("source-secret-value")).not.toBeInTheDocument();
    cleanup();
  });

  it("reads a generic source's ingest log off the cursor-paged audit ledger", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/alert-sources", () => ({
      json: { sources: [sourceView({ event_type: "rmm.device_offline" })] },
    }));
    let auditCalls = 0;
    mock.on("GET", "/api/v1/audit-events", ({ query }) => {
      auditCalls += 1;
      if (auditCalls === 1) {
        expect(query["action"]).toBe("webhook.ingested");
        return { json: { events: [auditEvent()], next_cursor: "audit-cursor-2" } };
      }
      expect(query["cursor"]).toBe("audit-cursor-2");
      return { json: { events: [auditEvent({ id: "a-2" })], next_cursor: null } };
    });

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByTestId(`ingest-log-${SRC_ID}`));

    // The accepted envelope renders from the audit row (label + dedup key).
    await screen.findByText("emitted");
    expect(screen.getByText(/prov-123/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Load older" }));
    await waitFor(() => expect(auditCalls).toBe(2));
    expect(await screen.findByText("Beginning of log.")).toBeInTheDocument();
    cleanup();
  });

  it("walks past another source's envelopes to fill one source's ingest log", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/alert-sources", () => ({
      json: { sources: [sourceView({ event_type: "rmm.device_offline" })] },
    }));
    let auditCalls = 0;
    mock.on("GET", "/api/v1/audit-events", () => {
      auditCalls += 1;
      if (auditCalls === 1) {
        // The first page belongs to a different source entirely.
        return {
          json: {
            events: [
              auditEvent({ id: "other-1", detail: { source_ref: "other-src", dedup_key: null } }),
            ],
            next_cursor: "audit-cursor-2",
          },
        };
      }
      return { json: { events: [auditEvent({ id: "a-2" })], next_cursor: null } };
    });

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByTestId(`ingest-log-${SRC_ID}`));

    // One click walks to the page that carries this source's row.
    await waitFor(() => expect(auditCalls).toBe(2));
    expect(await screen.findByText("emitted")).toBeInTheDocument();
    expect(screen.getByText("Beginning of log.")).toBeInTheDocument();
    cleanup();
  });

  it("reads an alert-profile source's ingest log off the alerts ledger", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/alert-sources", () => ({ json: { sources: [sourceView()] } }));
    let alertCalls = 0;
    mock.on("GET", "/api/v1/alerts", ({ query }) => {
      alertCalls += 1;
      expect(query["source_id"]).toBe(SRC_ID);
      return { json: { alerts: [alertRow()] } };
    });

    render(
      <AuthProvider>
        <IntegrationsSettingsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByTestId(`ingest-log-${SRC_ID}`));

    expect(await screen.findByText("warning · open")).toBeInTheDocument();
    // One short page — the offset ledger is at its beginning.
    expect(screen.getByText("Beginning of log.")).toBeInTheDocument();
    expect(alertCalls).toBe(1);
    cleanup();
  });
});
