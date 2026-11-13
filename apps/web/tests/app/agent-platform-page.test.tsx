// Integration test: /settings/agent-platform — deny-by-default
// capability registry (grant + revoke), the skills list, and the
// proposal approval queue with before/after diff and decline semantics.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import AgentPlatformPage from "../../app/settings/agent-platform/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/agent-platform",
}));

const PROPOSAL_ID = "60000000-0000-4000-8000-000000000001";

/** Capability grants the mock serves; handlers mutate this list. */
const grants: Record<string, unknown>[] = [];
const proposals: Record<string, unknown>[] = [];

function proposal(overrides: Record<string, unknown> = {}) {
  return {
    id: PROPOSAL_ID,
    entity: "ticket",
    entity_id: "71000000-0000-4000-8000-000000000001",
    op: "update",
    agent_kind: "agent",
    agent_ref: "claude/opus",
    capability: "tickets:write",
    dial_mode: "proposed",
    classification: "internal",
    status: "proposed",
    base_version: 12,
    before_value: { status: "open" },
    proposed_value: { status: "waiting" },
    rationale: "Client confirmed a waiting state on the call.",
    skill_slug: "triage",
    skill_version: 2,
    session_id: "sess-1",
    for_user: null,
    rule_set_hash: null,
    applied_at: null,
    decided_at: null,
    decided_by: null,
    decision_note: null,
    created_at: "2026-09-02T12:00:00Z",
    ...overrides,
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/mcp/capabilities", () => ({ json: { grants } }));
  mock.on("GET", "/api/v1/agent-skills", () => ({
    json: {
      skills: [
        {
          id: "62000000-0000-4000-8000-000000000001",
          slug: "triage",
          version: 2,
          manifest: { capabilities: ["tickets:read"], prompts: [], rules: [] },
          created_by: ME_FIXTURE.user_id,
          created_at: "2026-09-01T00:00:00Z",
        },
      ],
    },
  }));
  mock.on("GET", "/api/v1/agent-proposals", () => ({ json: { proposals, next_cursor: null } }));
  mock.on("PUT", "/api/v1/mcp/capabilities", ({ body }) => {
    const b = body as { capability: string; mode: string };
    const grant = {
      capability: b.capability,
      mode: b.mode,
      granted_by: ME_FIXTURE.user_id,
      granted_at: "2026-09-03T00:00:00Z",
      updated_at: "2026-09-03T00:00:00Z",
    };
    grants.unshift(grant);
    return { json: grant };
  });
  mock.on(
    "DELETE",
    `/api/v1/mcp/capabilities/${"clients:read"}`,
    () => {
      // A DELETE that answered 200 while leaving the server-side list intact made
      // the reconcile put the row straight back, so the revoke leg could never pass.
      grants.length = 0;
      return { json: {} };
    },
  );
  mock.on("POST", `/api/v1/agent-proposals/${PROPOSAL_ID}/decline`, () => {
    const row = proposals.find((p) => p.id === PROPOSAL_ID);
    if (row !== undefined) row.status = "declined";
    return { json: proposal({ status: "declined", decided_at: "2026-09-03T00:00:00Z" }) };
  });
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  grants.length = 0;
  proposals.length = 0;
  proposals.push(proposal());
});

describe("/settings/agent-platform integration", () => {
  it("shows the deny-by-default empty registry and the published skills list", async () => {
    setupApi();
    render(
      <AuthProvider>
        <AgentPlatformPage />
      </AuthProvider>,
    );
    await screen.findByText(/deny-by-default/);
    await screen.findByText(/No capabilities granted/);
    expect(screen.getByText("triage")).toBeInTheDocument();
    expect(screen.getByText("v2")).toBeInTheDocument();
    cleanup();
  });

  it("grants a capability with optimistic render then reconciles, and revokes it", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <AgentPlatformPage />
      </AuthProvider>,
    );
    await screen.findByText(/No capabilities granted/);
    await user.type(screen.getByLabelText("Capability"), "clients:read");
    await user.click(screen.getByRole("button", { name: "Grant" }));
    await waitFor(() => expect(mock.callsTo("PUT", "/api/v1/mcp/capabilities").length).toBe(1));
    expect(mock.lastBody("PUT", "/api/v1/mcp/capabilities")).toMatchObject({
      capability: "clients:read",
      mode: "proposed",
    });
    // The catalog table above badges every capability it lists, so the grant
    // itself has to be found in the registry region rather than anywhere on
    // the page. `findByText` alone matches the catalog badge too.
    const registry = () => within(screen.getByTestId("mcp-capability-registry"));
    await registry().findByText("clients:read");
    expect(screen.queryByText(/No capabilities granted/)).not.toBeInTheDocument();

    await user.click(registry().getByRole("button", { name: "Revoke" }));
    await waitFor(() =>
      expect(mock.callsTo("DELETE", "/api/v1/mcp/capabilities/clients:read").length).toBe(1),
    );
    await screen.findByText(/No capabilities granted/);
    expect(registry().queryByText("clients:read")).not.toBeInTheDocument();
    cleanup();
  });

  it("declines a proposed write — status flips and the declined row writes nothing", async () => {
    const mock = setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <AgentPlatformPage />
      </AuthProvider>,
    );
    await screen.findByText(/Client confirmed a waiting state/);
    await user.click(screen.getByRole("button", { name: "Show before/after" }));
    expect(screen.getByText(/"status": "open"/)).toBeInTheDocument();
    expect(screen.getByText(/"status": "waiting"/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Decline" }));
    await waitFor(() =>
      expect(mock.callsTo("POST", `/api/v1/agent-proposals/${PROPOSAL_ID}/decline`).length).toBe(1),
    );
    await screen.findByText("writes nothing");
    cleanup();
  });

  it("marks a failed load with the status danger pair, not body copy", async () => {
    const mock = setupApi();
    mock.on("GET", "/api/v1/mcp/capabilities", () => ({
      status: 503,
      json: { error: { message: "the capability registry is unreachable" } },
    }));
    render(
      <AuthProvider>
        <AgentPlatformPage />
      </AuthProvider>,
    );

    const text = await screen.findByText("Failed to load the agent platform surfaces.");
    // A failed read is error copy: the status danger pair on its own fill,
    // with the glyph beside it. It used to be plain body text, so the same
    // failure looked urgent on one settings page and like a paragraph here.
    const note = text.closest('[role="alert"]') as HTMLElement | null;
    expect(note).not.toBeNull();
    expect(note?.style.background).toBe("var(--m-status-danger-bg)");
    expect(note?.style.color).toBe("var(--m-status-danger-fg)");
    expect(note?.querySelector("svg")).not.toBeNull();
    cleanup();
  });
});
