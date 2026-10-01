// Integration test: /settings/workflow against the real SDK client over a
// mocked fetch router — board picking, the DRAWN statuses list (Wave 41 lane
// 12), transition matrix editing PUTting an SDK-typed SetTransitionsRequest,
// the checklist render, and submit-time validation in the drawer.
//
// The statuses list replaced the old inline name inputs in lane 12, so the
// two tests that drove those inputs now drive the drawer and the staged
// save bar instead. The promises are the same: an admin can rename a status
// and a blank name never reaches the API.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import WorkflowSettingsPage from "../../app/settings/workflow/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";
import { edgeKey } from "../../lib/admin/workflow";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/workflow",
}));

const BOARD_ID = "00000000-0000-4000-8000-00000000ba01";
const S1 = "10000000-0000-4000-8000-000000000001";
const S2 = "10000000-0000-4000-8000-000000000002";
const S3 = "10000000-0000-4000-8000-000000000003";

const STATUSES = [
  { id: S1, board_id: BOARD_ID, org_id: "o", name: "New", category: "open", is_entry: true, sort_order: 0, color: null },
  { id: S2, board_id: BOARD_ID, org_id: "o", name: "Working", category: "in_progress", is_entry: false, sort_order: 1, color: null },
  { id: S3, board_id: BOARD_ID, org_id: "o", name: "Done", category: "closed", is_entry: false, sort_order: 2, color: null },
];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/boards/nav", () => ({
    json: [
      {
        id: BOARD_ID,
        name: "Service Desk",
        description: null,
        is_default: false,
        group_id: null,
        group_name: null,
        favorite: false,
        my_role: "admin",
      },
    ],
  }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/statuses`, () => ({ json: STATUSES }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/transitions`, () => ({ json: { mode: "floor", edges: [] } }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/checklist-template`, () => ({
    json: {
      items: [{ id: "i1", org_id: "o", template_id: "t1", sort_order: 0, text: "Backups verified" }],
      require_checklist_close: true,
      template: {
        id: "t1",
        board_id: BOARD_ID,
        org_id: "o",
        name: "Standard close",
        created_at: "",
        updated_at: "",
      },
    },
  }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/workflow integration", () => {
  it("renders the picked board's statuses with prominent category chips and template name", async () => {
    setupApi();
    render(
      <AuthProvider>
        <WorkflowSettingsPage />
      </AuthProvider>,
    );
    // Statuses render (name appears in row input AND matrix axes)…
    await screen.findAllByText("Working");
    // …and category chips are visible (prominent), not select-only.
    expect((await screen.findAllByText("In progress")).length).toBeGreaterThan(0);
    expect(await screen.findByLabelText("Template name")).toHaveValue("Standard close");
    expect(screen.getByLabelText("Step 1")).toHaveValue("Backups verified");
    cleanup();
  });

  it("read-only member sees editors disabled instead of hidden state", async () => {
    const mock = new MockApi();
    const restrictedMe = { ...ME_FIXTURE, permissions: [] };
    mock.on("GET", "/api/v1/me", () => ({ json: restrictedMe }));
    mock.on("GET", "/api/v1/boards/nav", () => ({
      json: [
        {
          id: BOARD_ID,
          name: "Service Desk",
          description: null,
          is_default: false,
          group_id: null,
          group_name: null,
          favorite: false,
          my_role: "member",
        },
      ],
    }));
    mock.on("GET", `/api/v1/boards/${BOARD_ID}/statuses`, () => ({ json: STATUSES }));
    mock.on("GET", `/api/v1/boards/${BOARD_ID}/transitions`, () => ({ json: { mode: "floor", edges: [] } }));
    mock.on("GET", `/api/v1/boards/${BOARD_ID}/checklist-template`, () => ({
      json: { items: [], require_checklist_close: false, template: null },
    }));
    mock.install();
    render(
      <AuthProvider>
        <WorkflowSettingsPage />
      </AuthProvider>,
    );
    expect(await screen.findByText(/Read-only/i)).toBeInTheDocument();
    // Await the statuses commit too: the read-only banner renders before the
    // statuses fetch settles, so a synchronous query here races it in full
    // suite runs (leaked DOM then duplicated buttons downstream).
    // A read-only member gets no write affordance at all: no New status, no
    // save bar, and the staged switches are disabled.
    expect((await screen.findAllByText("New")).length).toBeGreaterThan(0);
    expect(screen.queryByTestId("statuses-new")).not.toBeInTheDocument();
    expect(screen.queryByTestId("settings-staged-bar")).not.toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Pauses SLA for New" })).toBeDisabled();
    cleanup();
  });

  it("toggling one matrix cell saves the full explicit matrix via transitions PUT", async () => {
    const mock = setupApi();
    mock.on("PUT", `/api/v1/boards/${BOARD_ID}/transitions`, () => ({
      json: { mode: "explicit", edges: [] },
    }));

    render(
      <AuthProvider>
        <WorkflowSettingsPage />
      </AuthProvider>,
    );

    // Await the data settle too: the Save button renders before the
    // transitions fetch resolves and its dirty gate flips enabled→disabled
    // one render later in full-suite runs (same class of race as the
    // read-only test above).
    await screen.findAllByText("Working");
    const saveButton = await screen.findByRole("button", { name: "Save transitions" });
    await waitFor(() => expect(saveButton).toBeDisabled());

    // Floor default allowed New→Done; toggle it off → dirty.
    const cell = screen.getByLabelText("Allow New → Done");
    expect(cell).toHaveAttribute("aria-checked", "true");
    fireEvent.click(cell);

    await waitFor(() => expect(saveButton).toBeEnabled());
    fireEvent.click(saveButton);

    await waitFor(() => {
      const puts = mock.callsTo("PUT", `/api/v1/boards/${BOARD_ID}/transitions`);
      expect(puts.length).toBe(1);
      const body = puts[0]?.body as
        | { edges: { from_status_id: string; to_status_id: string }[] }
        | undefined;
      const keys = (body?.edges ?? []).map((e) => edgeKey(e.from_status_id, e.to_status_id));
      expect(keys).not.toContain(edgeKey(S1, S3));
      expect(keys).toContain(edgeKey(S1, S2)); // New→Working still allowed
      expect(keys).not.toContain(edgeKey(S3, S1)); // closed→open stays blocked
    });
    cleanup();
  });

  it("a blank name in the drawer never reaches the API, and the field says why", async () => {
    const user = userEvent.setup();
    const mock = setupApi();

    render(
      <AuthProvider>
        <WorkflowSettingsPage />
      </AuthProvider>,
    );
    await screen.findAllByText("Working");
    await user.click(screen.getByTestId("statuses-new"));
    const name = await screen.findByTestId("status-name-input");
    // No error before the submit: validation is a submit-time event, and the
    // primary is never disabled to hint at one.
    expect(screen.queryByTestId("status-name-field-error")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save status" }));
    expect(await screen.findByTestId("status-name-field-error")).toHaveTextContent(
      "Give the status a name.",
    );
    // The primary is still enabled, so a retry is one click.
    expect(screen.getByRole("button", { name: "Save status" })).toBeEnabled();
    expect(mock.callsTo("POST", `/api/v1/boards/${BOARD_ID}/statuses`).length).toBe(0);
    expect(mock.callsTo("PATCH", `/api/v1/boards/${BOARD_ID}/statuses/${S1}`).length).toBe(0);
    // The drawer is still open with the work in it.
    expect((screen.getByTestId("status-name-input") as HTMLInputElement).value).toBe("");
    cleanup();
  });
});
