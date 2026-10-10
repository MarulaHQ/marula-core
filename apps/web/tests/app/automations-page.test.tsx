// Integration test: /automations — Wave 43 lane 5 spec item (D): a rule
// without a ticket (subject_kind "board") is creatable through the editor
// and readable in the ledger, with the ticket-less trigger vocabulary
// offered under that subject.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import AutomationsPage from "../../app/automations/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/automations",
}));

const BOARD_ID = "00000000-0000-4000-8000-000000000003";
const RULE_ID = "60000000-0000-4000-8000-000000000001";

function ruleView(overrides: Record<string, unknown> = {}) {
  return {
    id: RULE_ID,
    name: "RMM device went offline",
    enabled: true,
    board_id: null,
    trigger_event: "webhook.ingested",
    subject_kind: "board",
    conditions: [],
    actions: [{ type: "set_priority", priority: "p2" }],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/automation-rules", () => ({ json: [ruleView()] }));
  mock.on("GET", "/api/v1/boards/nav", () => ({
    json: [{ id: BOARD_ID, name: "Support" }],
  }));
  mock.on("GET", "/api/v1/org/users", () => ({ json: { users: [] } }));
  mock.on("GET", "/api/v1/plugins/surface/automation-actions", () => ({ json: { entries: [] } }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/automations — ticket-less rules (Wave 43 lane 5, spec item D)", () => {
  it("reads a board-subject rule in the ledger with its subject labelled", async () => {
    setupApi();
    render(
      <AuthProvider>
        <AutomationsPage />
      </AuthProvider>,
    );
    // The rule name and its ticket-less badge both render from the payload.
    await screen.findByText("RMM device went offline");
    expect(screen.getByText("no ticket")).toBeInTheDocument();
    expect(screen.getByText("webhook.ingested")).toBeInTheDocument();
    cleanup();
  });

  it("creates a board-subject rule through the editor", async () => {
    const user = userEvent.setup();
    const mock = setupApi();
    mock.on("GET", "/api/v1/automation-rules", () => ({ json: [] }));
    mock.on("POST", "/api/v1/automation-rules", ({ body }) => {
      expect(body).toMatchObject({
        name: "RMM device paged us",
        subject_kind: "board",
        trigger_event: "webhook.ingested",
        board_id: null,
      });
      expect((body as { actions: unknown[] }).actions.length).toBeGreaterThan(0);
      return { status: 200, json: { id: RULE_ID } };
    });

    render(
      <AuthProvider>
        <AutomationsPage />
      </AuthProvider>,
    );
    fireEvent.click(await screen.findByTestId("automation-new-rule"));

    await user.type(screen.getByLabelText("Rule name"), "RMM device paged us");

    // Switch the subject to board — the trigger follows to a ticket-less event.
    await user.click(screen.getByLabelText("Rule subject"));
    await user.click(await screen.findByRole("option", { name: /Board — fires on events with no ticket/ }));
    await waitFor(() => {
      expect(screen.getByText("webhook.ingested")).toBeInTheDocument();
    });

    // One action makes the draft savable (defaults are a complete action).
    fireEvent.click(screen.getByRole("button", { name: "Add action" }));
    fireEvent.click(screen.getByRole("button", { name: "Save rule" }));

    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/automation-rules").length).toBe(1);
    });
    cleanup();
  });
});
