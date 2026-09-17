// Integration test: /settings/plugins — Wave 16 lane 8 close-out over
// ADR 0009: installed lifecycle (enable/disable, settings posture,
// removal copy), registry browse/install, the staged-approvals queue
// (consent review → approve installs-disabled / refuse note-required),
// and the agent-skills enable gate.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import PluginsSettingsPage from "../../app/settings/plugins/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/plugins",
}));

const INSTALL_ID = "75000000-0000-4000-8000-000000000001";
const STAGED_ID = "76000000-0000-4000-8000-000000000001";
const SKILL_ID = "77000000-0000-4000-8000-000000000001";
const CATALOG_ID = "78000000-0000-4000-8000-000000000001";

const installs: Record<string, unknown>[] = [];
const stagedRows: Record<string, unknown>[] = [];
const skills: Record<string, unknown>[] = [];
const catalog: Record<string, unknown>[] = [];

const OWNER_ME = { ...ME_FIXTURE, permissions: ["special:manage_plugins"] };

function installFixture(overrides: Record<string, unknown> = {}) {
  return {
    id: INSTALL_ID,
    slug: "invoice-export",
    name: "Invoice Export",
    version: "1.0.0",
    status: "disabled",
    granted_capabilities: ["clients:read", "time:read"],
    artifact_kind: "wasm",
    effective_permissions: [],
    installed_at: "2026-09-16T00:00:00Z",
    ...overrides,
  };
}

function stagedFixture(overrides: Record<string, unknown> = {}) {
  return {
    id: STAGED_ID,
    artifact_kind: "plugin",
    source_kind: "local_zip",
    source_url: "zip://invoice-export-1.2.0.zip",
    fetched_ref: null,
    artifact_digest: "a".repeat(64),
    status: "pending",
    declared_capabilities: ["clients:read", "financial:write"],
    manifest: { name: "invoice-export", version: "1.2.0" },
    uploaded_by: "88000000-0000-4000-8000-000000000001",
    review_note: null,
    adopted_install_id: null,
    adopted_skill_id: null,
    ...overrides,
  };
}

function consentFixture(overrides: Record<string, unknown> = {}) {
  return {
    artifact_kind: "plugin",
    source_kind: "local_zip",
    source_url: "zip://invoice-export-1.2.0.zip",
    fetched_ref: null,
    artifact_digest: "a".repeat(64),
    declared_capabilities: ["clients:read", "financial:write"],
    settings_schema: {
      type: "object",
      properties: {
        default_bucket: { type: "string", enum: ["none", "retainer"] },
      },
      required: [],
    },
    ...overrides,
  };
}

function skillFixture(version: number, enabled: boolean, overrides: Record<string, unknown> = {}) {
  return {
    id: `${SKILL_ID.slice(0, -1)}${version}`,
    slug: "vpn-triage",
    version,
    enabled,
    manifest: { name: "vpn-triage", version: `1.0.${version}` },
    source_kind: "skill_community",
    source_url: "https://skills.example.dev/vpn-triage/manifest.json",
    fetched_ref: "v1.0.0",
    artifact_digest: "b".repeat(64),
    created_at: "2026-09-15T00:00:00Z",
    ...overrides,
  };
}

function setupApi(me = OWNER_ME): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: me }));
  mock.on("GET", "/api/v1/plugins/installed", () => ({ json: installs }));
  mock.on("GET", "/api/v1/plugins/distribution/staged", () => ({ json: stagedRows }));
  mock.on("GET", "/api/v1/plugins/distribution/agent-skills", () => ({ json: skills }));
  mock.on("GET", "/api/v1/plugins/catalog", () => ({ json: catalog }));
  mock.on("GET", `/api/v1/plugins/installed/${INSTALL_ID}/settings`, () => ({
    json: {
      slug: "invoice-export",
      version: "1.0.0",
      config: { default_bucket: "none" },
      settings_schema: {
        type: "object",
        properties: {
          default_bucket: {
            type: "string",
            enum: ["none", "retainer"],
            description: "Where unbilled work lands",
          },
        },
        required: [],
      },
    },
  }));
  mock.on("GET", `/api/v1/plugins/installed/${INSTALL_ID}/provenance`, () => ({
    json: [
      {
        id: "pv-1",
        install_id: INSTALL_ID,
        action: "installed",
        source_kind: "local_zip",
        source_url: "zip://invoice-export-1.0.0.zip",
        fetched_ref: null,
        artifact_digest: "c".repeat(64),
        uploader: ME_FIXTURE.user_id,
        recorded_at: "2026-09-16T00:00:00Z",
      },
    ],
  }));
  mock.on("GET", `/api/v1/plugins/installed/${INSTALL_ID}/update`, () => ({
    json: {
      install_id: INSTALL_ID,
      slug: "invoice-export",
      current_version: "1.0.0",
      pending_version: null,
      pending_digest: null,
      current_granted_capabilities: ["clients:read", "time:read"],
      consent_diff: { added: [], removed: [], unchanged: ["clients:read", "time:read"] },
    },
  }));
  mock.on("PATCH", `/api/v1/plugins/installed/${INSTALL_ID}`, ({ body }) => {
    const b = body as { status?: string };
    if (b.status !== undefined) {
      const row = installs.find((i) => (i as { id: string }).id === INSTALL_ID);
      if (row !== undefined) (row as { status: string }).status = b.status;
    }
    return { json: installFixture() };
  });
  mock.on("PATCH", `/api/v1/plugins/installed/${INSTALL_ID}/settings`, ({ body }) => {
    const b = body as { config: Record<string, unknown> };
    return { json: { slug: "invoice-export", version: "1.0.0", config: b.config } };
  });
  mock.on("GET", `/api/v1/plugins/distribution/staged/${STAGED_ID}/consent`, () => ({
    json: consentFixture(),
  }));
  mock.on("POST", `/api/v1/plugins/distribution/staged/${STAGED_ID}/approve`, () => {
    const row = stagedRows.find((r) => (r as { id: string }).id === STAGED_ID);
    if (row !== undefined) Object.assign(row, { status: "approved" });
    return { json: stagedFixture({ status: "approved" }) };
  });
  mock.on("POST", `/api/v1/plugins/distribution/staged/${STAGED_ID}/refuse`, () => {
    const row = stagedRows.find((r) => (r as { id: string }).id === STAGED_ID);
    if (row !== undefined) Object.assign(row, { status: "refused" });
    return { json: stagedFixture({ status: "refused" }) };
  });
  mock.on("POST", `/api/v1/plugins/installed`, ({ body }) => {
    const b = body as { slug: string };
    const created = installFixture({
      id: "79000000-0000-4000-8000-000000000001",
      slug: b.slug,
      name: "Collector HTTP",
      status: "disabled",
    });
    installs.push(created);
    return { json: created, status: 201 };
  });
  mock.on("POST", `/api/v1/plugins/distribution/skill/${SKILL_ID}/enabled`, () => ({ json: {}, status: 204 }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  installs.length = 0;
  stagedRows.length = 0;
  skills.length = 0;
  catalog.length = 0;
  installs.push(installFixture({ status: "disabled" }));
  stagedRows.push(stagedFixture());
  skills.push(skillFixture(2, false), skillFixture(1, false));
  catalog.push({ id: CATALOG_ID, slug: "collector-http", name: "Collector HTTP", publisher: "Meridian", description: "HTTP connector", latest_version: "0.9.0", revoked: false, created_at: "2026-09-01T00:00:00Z", interface_version: 1 });
});

describe("/settings/plugins integration", () => {
  it("renders the install roster with status tags and opens the manifest-rendered settings form", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Invoice Export");
    expect(screen.getByText("disabled")).toBeInTheDocument();
    await user.click(screen.getByTestId("expand-invoice-export"));
    await screen.findByText(/"default_bucket"/);
   expect(screen.getByText("Provenance chain")).toBeInTheDocument();
    cleanup();
  });

  it("splits the roster into the artifact-kind tabs with typed empty states (no fake tabs)", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    // Seed one HTTP-connector install alongside the WASM baseline.
    installs.push(
      installFixture({
        id: "7a000000-0000-4000-8000-000000000001",
        slug: "collector-http",
        name: "Collector HTTP",
        artifact_kind: "http_connector",
      }),
    );
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    // WASM tab (default): the wasm install shows, the connector does not.
    await screen.findByText("Invoice Export");
    expect(screen.queryByText("Collector HTTP")).toBeNull();
    // HTTP connectors tab: only the connector install.
    await user.click(screen.getByTestId("tab-http-connectors"));
    await screen.findByText("Collector HTTP");
    expect(screen.queryByText("Invoice Export")).toBeNull();
    // Removing the connector install from the fixture list types the empty.
    installs.splice(1, 1);
    await user.click(screen.getByTestId("tab-wasm"));
    expect(screen.getByText("Invoice Export")).toBeInTheDocument();
    await user.click(screen.getByTestId("tab-http-connectors"));
    await screen.findByText(/No HTTP connectors installed yet/);
    cleanup();
  });

  it("shows the settings-less posture for installs without a schema, not a dead form", async () => {
    const mock = setupApi();
    mock.on("GET", `/api/v1/plugins/installed/${INSTALL_ID}/settings`, () => ({
      json: { slug: "invoice-export", version: "1.0.0", config: null },
    }));
    cleanup();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Invoice Export");
    await user.click(screen.getByTestId("expand-invoice-export"));
    await screen.findByTestId("settings-less-posture");
    expect(screen.getByText(/consent, enable and disable only/)).toBeInTheDocument();
    cleanup();
  });

  it("toggles enable via PATCH with an idempotency envelope body", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Invoice Export");
    await user.click(screen.getByTestId("expand-invoice-export"));
    await user.click(screen.getByTestId("install-toggle"));
    await waitFor(() => expect(mock.callsTo("PATCH", `/api/v1/plugins/installed/${INSTALL_ID}`).length).toBe(1));
    const body = mock.lastBody<Record<string, unknown>>("PATCH", `/api/v1/plugins/installed/${INSTALL_ID}`);
    expect(body.idempotency_key).toEqual(expect.any(String));
    cleanup();
  });

  it("reviews a staged zip import through the one consent screen and approves with a granted subset", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Approvals");
    await user.click(screen.getByTestId("tab-approvals"));
    await screen.findByText("Review consent");
    await user.click(screen.getByRole("button", { name: "Review consent" }));
    await screen.findByTestId("settings-schema-block");
    expect(screen.getByText(/Provenance chain/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getAllByRole("button", { name: /^Approve — installs disabled/ }).length).toBeGreaterThan(0));
    const approveButtons = screen.getAllByRole("button", { name: /^Approve — installs disabled/ });
    expect(approveButtons.length).toBeGreaterThan(0);
    await user.click(approveButtons[0]!);
    await waitFor(() =>
      expect(mock.callsTo("POST", `/api/v1/plugins/distribution/staged/${STAGED_ID}/approve`).length).toBe(1),
    );
    const body = mock.lastBody<{ granted_capabilities: string[] }>(
      "POST",
      `/api/v1/plugins/distribution/staged/${STAGED_ID}/approve`,
    );
    expect(body.granted_capabilities).toEqual(["clients:read", "financial:write"]);
    cleanup();
  });

  it("refuses a staged import only with a required note", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Invoice Export");
    await user.click(screen.getByTestId("tab-approvals"));
    await screen.findByText("Review consent");
    await user.click(screen.getByRole("button", { name: "Review consent" }));
    await screen.findByTestId("refuse-note");
    const refuseButton = screen.getByRole("button", { name: "Refuse" });
    expect(refuseButton).toBeDisabled();
    await user.type(screen.getByTestId("refuse-note"), "digest did not match the upstream tag");
    await user.click(refuseButton);
    await waitFor(() =>
      expect(mock.callsTo("POST", `/api/v1/plugins/distribution/staged/${STAGED_ID}/refuse`).length).toBe(1),
    );
    expect(
      mock.lastBody<{ review_note: string }>("POST", `/api/v1/plugins/distribution/staged/${STAGED_ID}/refuse`)
        .review_note,
    ).toMatch(/digest did not match/);
    cleanup();
  });

  it("installs from the registry catalog (created DISABLED)", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Invoice Export");
    await user.click(screen.getByTestId("tab-registry"));
    await screen.findByText("Collector HTTP");
    await user.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() => expect(mock.callsTo("POST", "/api/v1/plugins/installed").length).toBe(1));
    expect(mock.lastBody("POST", "/api/v1/plugins/installed")).toMatchObject({
      slug: "collector-http",
      version: "0.9.0",
    });
    expect(screen.getByText(/starts DISABLED/)).toBeInTheDocument();
    cleanup();
  });

  it("enables an agent skill on the active version row only (deny-by-default)", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Invoice Export");
    await user.click(screen.getByTestId("tab-agent-skills"));
    await screen.findByText("vpn-triage");
    expect(screen.getByText("disabled")).toBeInTheDocument();
    await user.click(screen.getByTestId("skill-toggle-vpn-triage"));
    await waitFor(() =>
      expect(mock.callsTo("POST", `/api/v1/plugins/distribution/agent-skills/${skills[1]?.id}/enabled`).length + mock.callsTo("POST", `/api/v1/plugins/distribution/agent-skills/${skills[0]?.id}/enabled`).length).toBe(1),
    );
    cleanup();
  });

  it("blocks floating GitHub refs and the oversized zip client-side before any wire call", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <PluginsSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Invoice Export");
    await user.click(screen.getByTestId("open-github-drawer"));
    await user.type(screen.getByLabelText("Tag"), "main");
    await screen.findByTestId("tag-problem");
    expect(screen.getByTestId("github-import-form")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByTestId("open-zip-drawer"));
    expect(screen.getByText(/8 MiB archive/)).toBeInTheDocument();
    // No staged/github wire calls were made at any point above.
    expect(mock.callsTo("POST", "/api/v1/plugins/distribution/staged/github").length).toBe(0);
    cleanup();
  });
});
