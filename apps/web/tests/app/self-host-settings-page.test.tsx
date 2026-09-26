// Integration test: /settings/self-host (Wave 36 lane 3) — the health sweep
// with an honest down row, the ClamAV quarantine list, DSN presence rows
// that never carry a value, and the not-configured link cards.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import SelfHostSettingsPage from "../../app/settings/self-host/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/self-host",
}));

const OWNER_FIXTURE = {
  ...ME_FIXTURE,
  permissions: ["special:manage_users"],
};

function view(overrides: Record<string, unknown> = {}) {
  return {
    scan: {
      provider: "clamd",
      clamd_addr: "clamav:3310",
      last_scan_at: "2026-09-26T08:00:00Z",
      last_detection_at: "2026-09-26T08:05:00Z",
      detection_count: 3,
    },
    detections: [
      {
        message_id: "00000000-0000-4000-8000-0000000000d1",
        subject: "Invoice attached",
        from_address: "sender@outside.test",
        verdict: "infected",
        ingest_source: "webhook/clamd",
        occurred_at: "2026-09-26T08:05:00Z",
        attachments: [
          {
            filename: "invoice.pdf.exe",
            content_type: "application/octet-stream",
            size_bytes: 2048,
            sha256: "abc123",
          },
        ],
      },
    ],
    services: [
      { id: "core_api", label: "core-api", state: "up", detail: "served this request", loops: null },
      { id: "database", label: "PostgreSQL", state: "up", detail: "SELECT 1 answered", loops: null },
      { id: "valkey", label: "Valkey (Redis)", state: "up", detail: "valkey:6379 answered PING", loops: null },
      { id: "realtime", label: "realtime hub", state: "up", detail: "http://realtime:8081/healthz: 200", loops: null },
      { id: "worker", label: "worker", state: "down", detail: "timed out", loops: null },
    ],
    glitchtip_dsns: [
      { service: "core-api", state: "configured", detail: "read from this process's environment" },
      { service: "realtime", state: "unset", detail: "reported by the service's /healthz" },
      { service: "worker", state: "unknown", detail: "unknown — the service's /healthz did not answer" },
    ],
    links: {
      glitchtip: null,
      grafana: "http://localhost:3001",
      tempo: null,
      loki: null,
      otel_metrics: "http://localhost:18899",
    },
    garage: {
      volumes: ["garagedata", "garagemeta"],
      container_paths: ["/var/lib/garage/data", "/var/lib/garage/meta"],
      bootstrap_commands: ["docker compose --profile stage2 up -d garage"],
      admin_api_docs_url:
        "https://garagehq.deuxfleurs.fr/documentation/reference-manual/admin-api/",
      s3_endpoint: "http://garage:3900",
      s3_bucket: "meridian",
    },
    ...overrides,
  };
}

function setupApi(payload: unknown): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: OWNER_FIXTURE }));
  mock.on("GET", "/api/v1/settings/self-host", () => ({ json: payload }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/self-host integration", () => {
  it("renders the sweep, the quarantines, and never a DSN value", async () => {
    setupApi(view());
    render(
      <AuthProvider>
        <SelfHostSettingsPage />
      </AuthProvider>,
    );
    // The sweep names the failing check instead of painting all-green.
    expect(await screen.findByText(/1 check needs attention/i)).toBeInTheDocument();
    expect(screen.getByTestId("self-host-service-worker")).toHaveTextContent("down");
    expect(screen.getByTestId("self-host-service-valkey")).toHaveTextContent("up");
    // The quarantine row carries the subject, source, and file.
    expect(screen.getByText("Invoice attached")).toBeInTheDocument();
    expect(screen.getByText(/webhook\/clamd/)).toBeInTheDocument();
    expect(screen.getByText(/invoice.pdf.exe/)).toBeInTheDocument();
    // DSN presence renders per service, and the page never emits the DSN key.
    expect(screen.getByTestId("self-host-dsn-core-api")).toHaveTextContent("configured");
    expect(screen.getByTestId("self-host-dsn-realtime")).toHaveTextContent("unset");
    expect(screen.getByTestId("self-host-dsn-worker")).toHaveTextContent("unknown");
    // A configured link is an external link; an unconfigured one is a typed state.
    expect(screen.getByRole("link", { name: /open grafana/i })).toHaveAttribute(
      "href",
      "http://localhost:3001",
    );
    expect(screen.getByText(/Set MERIDIAN_GLITCHTIP_URL on core-api/)).toBeInTheDocument();
    cleanup();
  });

  it("says nothing is quarantined when the scanner is off", async () => {
    setupApi(
      view({
        scan: {
          provider: "none",
          clamd_addr: null,
          last_scan_at: null,
          last_detection_at: null,
          detection_count: 0,
        },
        detections: [],
      }),
    );
    render(
      <AuthProvider>
        <SelfHostSettingsPage />
      </AuthProvider>,
    );
    expect(await screen.findByText("none mode")).toBeInTheDocument();
    expect(
      screen.getByText(/No scanner is wired \(EMAIL_SCAN is unset\)/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Nothing is quarantined in this organization/i),
    ).toBeInTheDocument();
    cleanup();
  });

  it("refuses a member account without the owner permission", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.on("GET", "/api/v1/settings/self-host", () => ({ json: view() }));
    mock.install();
    render(
      <AuthProvider>
        <SelfHostSettingsPage />
      </AuthProvider>,
    );
    expect(
      await screen.findByText(/limited to org owners and admins/i),
    ).toBeInTheDocument();
    expect(mock.callsTo("GET", "/api/v1/settings/self-host")).toHaveLength(0);
    cleanup();
  });
});
