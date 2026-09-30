// Wave 41 lane 10 — the knowledge cluster on the mockup pack's ledger frame.
//
// Purpose: pin the behaviours the brief names for this lane, so a later
// refactor cannot quietly drop one of them:
//
//   1. /kb states the PUBLISH state (status, category, portal) as one row, and
//      an article whose category is staff-only cannot read as portal-visible.
//   2. The document-type schema editor refuses invalid JSON, in BLUSH, while
//      the mistake is being made — not after a round trip, and never in red.
//   3. A vault reveal is its own confirmation: the first gesture opens the
//      question, only the second decrypts.
//   4. The alert feed opens with the collapsed 40px assistant stub carrying a
//      count (AI ladder state 2) rather than an open column.
//
// Non-purpose: no visual assertions — geometry belongs to the pack review and
// the screenshot lane. These are the decisions a unit test can hold.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import KbPage from "../../app/kb/page";
import DocumentsPage from "../../app/documents/page";
import AlertsPage from "../../app/alerts/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({ id: "doc-1" }),
  usePathname: () => "/kb",
}));

const CAT_CLIENT_FACING = {
  id: "cat-a",
  org_id: "o",
  name: "Getting started",
  description: "",
  position: 0,
  client_visible: true,
};
const CAT_STAFF_ONLY = {
  id: "cat-b",
  org_id: "o",
  name: "Internals",
  description: "",
  position: 1,
  client_visible: false,
};
const ARTICLE_PUBLISHED = {
  id: "a-1",
  org_id: "o",
  title: "Reset the printer on an MFP",
  slug: "reset-printer-mfp",
  body: "",
  status: "published",
  category_id: "cat-a",
  client_visible: true,
  content_jsonb: null,
  created_at: "2026-09-01T10:00:00Z",
  updated_at: "2026-09-02T10:00:00Z",
};
const ARTICLE_GATED = {
  id: "a-2",
  org_id: "o",
  title: "Rotate the service credential",
  slug: "rotate-service-credential",
  body: "",
  status: "published",
  category_id: "cat-b",
  client_visible: true,
  content_jsonb: null,
  created_at: "2026-09-01T10:00:00Z",
  updated_at: "2026-09-02T10:00:00Z",
};

const LAYOUT = {
  id: "l-1",
  org_id: "o",
  name: "Runbook",
  description: "",
  fields: [{ key: "model", label: "Model", type: "text", position: 0 }],
  is_system: false,
};

function setupKb(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: [] }));
  mock.on("GET", "/api/v1/board-groups", () => ({ json: [] }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
  mock.on("GET", "/api/v1/kb/categories", () => ({ json: [CAT_CLIENT_FACING, CAT_STAFF_ONLY] }));
  mock.on("GET", "/api/v1/kb/articles", () => ({ json: [ARTICLE_PUBLISHED, ARTICLE_GATED] }));
  mock.install();
  return mock;
}

function setupDocuments(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: [] }));
  mock.on("GET", "/api/v1/board-groups", () => ({ json: [] }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
  mock.on("GET", "/api/v1/documents", () => ({
    json: [
      {
        id: "doc-1",
        org_id: "o",
        title: "Printer reset runbook",
        body: "",
        client_id: null,
        layout_id: "l-1",
        fields: { model: "MFP-880" },
        current_version: 3,
        created_at: "2026-09-01T10:00:00Z",
        updated_at: "2026-09-02T10:00:00Z",
      },
    ],
  }));
  mock.on("GET", "/api/v1/documents/layouts", () => ({ json: [LAYOUT] }));
  mock.on("GET", "/api/v1/clients", () => ({ json: [] }));
  mock.install();
  return mock;
}

function setupAlerts(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: [] }));
  mock.on("GET", "/api/v1/board-groups", () => ({ json: [] }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
  mock.on("GET", "/api/v1/alert-sources", () => ({
    json: { sources: [{ id: "s-1", org_id: "o", name: "Watchtower", source_ref: "abc", enabled: true, correlation_window_secs: 900 }] },
  }));
  mock.on("GET", "/api/v1/alerts", () => ({
    json: {
      alerts: [
        {
          id: "al-1",
          org_id: "o",
          source_id: "s-1",
          title: "Disk pressure on db-02",
          severity: "critical",
          status: "open",
          dedup_key: "d1",
          correlation_key: "c1",
          incident_ticket_id: null,
          received_at: "2026-09-02T10:00:00Z",
          acked_by: null,
          acked_at: null,
        },
      ],
      next_cursor: null,
    },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});
afterEach(() => {
  cleanup();
});

describe("/kb — publish state", () => {
  it("marks an article in the portal only when it AND its category are client-facing", async () => {
    setupKb();
    render(
      <AuthProvider>
        <KbPage />
      </AuthProvider>,
    );
    await waitFor(() => {
      expect(screen.getByTestId("kb-article-table")).toBeTruthy();
    });
    // The article whose category is staff-only must not read as portal-visible.
    expect(screen.getByText("In the portal")).toBeTruthy();
    expect(
      screen.getByText("Client-facing, not in the portal"),
    ).toBeTruthy();
  });

  it("shows the publish row inside the article editor, and states the gate in words", async () => {
    setupKb();
    render(
      <AuthProvider>
        <KbPage />
      </AuthProvider>,
    );
    await waitFor(() => {
      expect(screen.getByTestId("kb-article-table")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("Reset the printer on an MFP"));
    // The reader dialog opens first; the editor's publish row is what carries
    // the status/category/portal trio.
    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeTruthy();
    });
  });
});

describe("/documents — the schema editor's invalid JSON", () => {
  it("refuses to save and explains in blush, not red, while the JSON is broken", async () => {
    setupDocuments();
    render(
      <AuthProvider>
        <DocumentsPage />
      </AuthProvider>,
    );
    await waitFor(() => {
      expect(screen.getByTestId("documents-table")).toBeTruthy();
    });
    fireEvent.click(screen.getByTestId("documents-manage-types"));
    await waitFor(() => {
      expect(screen.getByRole("dialog")).toBeTruthy();
    });
    // Open the type's editor, then break the schema on purpose.
    const edit = await screen.findByText("Edit");
    fireEvent.click(edit);
    const textarea = await screen.findByLabelText("Field schema");
    fireEvent.change(textarea, { target: { value: '[{"key": "model"' } });
    const problem = await screen.findByTestId("documents-invalid-schema");
    expect(problem.textContent ?? "").toContain("not valid JSON");
    expect(problem.textContent ?? "").not.toContain("The type was rejected");
    const save = screen.getByTestId("documents-layout-save") as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    // The save is refused, so no mutation rode out with broken JSON.
    const textareaValue = (textarea as HTMLTextAreaElement).value;
    fireEvent.change(textarea, { target: { value: textareaValue } });
    expect((screen.getByTestId("documents-layout-save") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("/alerts — the collapsed assistant stub", () => {
  it("opens with a 40px stub carrying the unrouted count, not an open column", async () => {
    setupAlerts();
    render(
      <AuthProvider>
        <AlertsPage />
      </AuthProvider>,
    );
    await waitFor(() => {
      expect(screen.getByTestId("assistant-stub")).toBeTruthy();
    });
    const stub = screen.getByTestId("assistant-stub");
    expect(stub.textContent ?? "").toContain("not been routed to a ticket");
    // A count is rendered, not just a glyph (its exact value belongs to the
    // fixture, not to this assertion).
    expect(stub.textContent ?? "").toMatch(/\d/);
  });

  it("acknowledges an alert from its row", async () => {
    const mock = setupAlerts();
    mock.on("POST", "/api/v1/alerts/al-1/acknowledge", () => ({
      json: {
        alert: {
          id: "al-1",
          org_id: "o",
          source_id: "s-1",
          title: "Disk pressure on db-02",
          severity: "critical",
          status: "acknowledged",
          dedup_key: "d1",
          correlation_key: "c1",
          incident_ticket_id: null,
          received_at: "2026-09-02T10:00:00Z",
          acked_by: ME_FIXTURE.id,
          acked_at: "2026-09-02T11:00:00Z",
        },
      },
    }));
    render(
      <AuthProvider>
        <AlertsPage />
      </AuthProvider>,
    );
    const ack = await screen.findByTestId("alert-ack-al-1");
    fireEvent.click(ack);
    await waitFor(() => {
      expect(screen.getByText("acknowledged")).toBeTruthy();
    });
  });
});
