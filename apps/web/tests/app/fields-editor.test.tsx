// Custom-field editor: type matrix, typed validation, reorder, delete.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import BoardFieldsPage from "../../app/boards/[id]/fields/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({ id: "00000000-0000-4000-8000-00000000b001" }),
}));

const BOARD_ID = "00000000-0000-4000-8000-00000000b001";
const F_SELECT = {
  id: "f-1",
  board_id: BOARD_ID,
  org_id: "o",
  entity: "ticket",
  name: "Severity",
  kind: "select",
  options: ["low", "high"],
  required: true,
  sort_order: 0,
};
const F_TEXT = {
  id: "f-2",
  board_id: BOARD_ID,
  org_id: "o",
  entity: "ticket",
  name: "RMA number",
  kind: "text",
  options: [],
  required: false,
  sort_order: 1,
};

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: [] }));
  mock.on("GET", "/api/v1/board-groups", () => ({ json: [] }));
  mock.on("GET", "/api/v1/notifications/unread-count", () => ({ json: { count: 0 } }));
  mock.on("GET", "/api/v1/notifications", () => ({ json: { items: [], next_cursor: null } }));
  mock.on("GET", `/api/v1/boards/${BOARD_ID}/fields`, () => ({ json: [F_TEXT, F_SELECT] }));
  mock.on("POST", `/api/v1/boards/${BOARD_ID}/fields`, (ctx) => ({
    json: {
      id: "f-new",
      board_id: BOARD_ID,
      org_id: "o",
      entity: "ticket",
      required: false,
      sort_order: 2,
      ...(ctx.body as Record<string, unknown>),
    },
  }));
  mock.on("PATCH", `/api/v1/boards/${BOARD_ID}/fields/f-1`, () => ({ json: { ...F_SELECT, sort_order: 1 } }));
  mock.on("PATCH", `/api/v1/boards/${BOARD_ID}/fields/f-2`, () => ({ json: { ...F_TEXT, sort_order: 0 } }));
  mock.on("DELETE", `/api/v1/boards/${BOARD_ID}/fields/f-2`, () => ({ json: {} }));
  mock.install();
  return mock;
}

function page() {
  return (
    <AuthProvider>
      <BoardFieldsPage />
    </AuthProvider>
  );
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("/boards/[id]/fields — custom-field editor", () => {
  it("lists fields with kind labels, option counts, and required tags", async () => {
    setupApi();
    render(page());
    expect(await screen.findByTestId("field-row-Severity")).toBeInTheDocument();
    expect(screen.getByTestId("field-row-Severity")).toHaveTextContent("Select · 2 options");
    expect(screen.getByTestId("field-row-Severity")).toHaveTextContent("Required");
    expect(screen.getByTestId("field-row-RMA number")).toHaveTextContent("Short text");
  });

  it("creates a choice field through the type matrix with options", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByTestId("field-row-Severity");

    fireEvent.click(screen.getByTestId("add-field"));
    fireEvent.click(screen.getByTestId("field-kind-select"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Region" } });
    fireEvent.change(screen.getByLabelText("Options"), { target: { value: "EMEA" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    fireEvent.change(screen.getByLabelText("Options"), { target: { value: "APAC" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    fireEvent.click(screen.getByRole("button", { name: "Create field" }));

    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/boards/${BOARD_ID}/fields`)).toHaveLength(1);
    });
    const body = mock.lastBody<Record<string, unknown>>( "POST", `/api/v1/boards/${BOARD_ID}/fields`);
    expect(body).toMatchObject({ name: "Region", kind: "select", options: ["EMEA", "APAC"] });
    expect(await screen.findByTestId("field-row-Region")).toBeInTheDocument();
  });

  it("blocks saving a choice field with no options (typed validation)", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByTestId("field-row-Severity");

    fireEvent.click(screen.getByTestId("add-field"));
    fireEvent.click(screen.getByTestId("field-kind-multiselect"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Tags" } });
    fireEvent.click(screen.getByRole("button", { name: "Create field" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Choice fields need at least one option",
    );
    expect(mock.callsTo("POST", `/api/v1/boards/${BOARD_ID}/fields`)).toHaveLength(0);
  });

  it("reorders with the up/down controls and persists both sort_order swaps", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByTestId("field-row-Severity");

    fireEvent.click(screen.getByTestId("field-down-Severity"));
    await waitFor(() => {
      expect(mock.callsTo("PATCH", `/api/v1/boards/${BOARD_ID}/fields/f-1`)).toHaveLength(1);
      expect(mock.callsTo("PATCH", `/api/v1/boards/${BOARD_ID}/fields/f-2`)).toHaveLength(1);
    });
    expect(mock.lastBody("PATCH", `/api/v1/boards/${BOARD_ID}/fields/f-1`)).toMatchObject({ sort_order: 1 });
    expect(mock.lastBody("PATCH", `/api/v1/boards/${BOARD_ID}/fields/f-2`)).toMatchObject({ sort_order: 0 });
  });

  it("deletes with an inline confirm", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByTestId("field-row-RMA number");

    fireEvent.click(screen.getByTestId("field-delete-RMA number"));
    fireEvent.click(screen.getByTestId("field-confirm-delete-RMA number"));
    await waitFor(() => {
      expect(mock.callsTo("DELETE", `/api/v1/boards/${BOARD_ID}/fields/f-2`)).toHaveLength(1);
    });
    expect(screen.queryByTestId("field-row-RMA number")).not.toBeInTheDocument();
  });
});
