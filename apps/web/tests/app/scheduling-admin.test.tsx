// /dispatch/scheduling admin: template matrix, typed validation,
// group member management. Meeting-type writes are envelope-gated.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import DispatchSchedulingPage from "../../app/dispatch/scheduling/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({}),
}));

const USER_A = {
  id: "00000000-0000-4000-8000-00000000u001",
  org_id: "o",
  email: "tech@meridian.test",
  display_name: "Tara Tech",
  is_active: true,
  created_at: "2026-08-01T00:00:00Z",
};
const USER_B = { ...USER_A, id: "00000000-0000-4000-8000-00000000u002", display_name: "Boris Backup" };
const GROUP_A = {
  id: "00000000-0000-4000-8000-00000000g001",
  org_id: "o",
  name: "Service Desk",
  description: "Front-line pool",
  member_user_ids: [USER_A.id],
  created_at: "2026-08-01T00:00:00Z",
  created_by: null,
};
const TEMPLATE = {
  id: "00000000-0000-4000-8000-00000000m001",
  org_id: "o",
  name: "On-site visit",
  description: null,
  duration_minutes: 90,
  buffer_minutes: 15,
  booking_window_days: 21,
  assignee_scope: "group",
  assignee_user_id: null,
  assignee_group_id: GROUP_A.id,
  auto_email_enabled: true,
  auto_email_subject: "Schedule your visit",
  auto_email_body: "Pick a slot",
  is_active: true,
  parameters: {},
  created_at: "2026-08-01T00:00:00Z",
  created_by: null,
};

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/meeting-types", () => ({ json: [TEMPLATE] }));
  mock.on("GET", "/api/v1/scheduling-groups", () => ({ json: [GROUP_A] }));
  mock.on("GET", "/api/v1/org/users", () => ({ json: { users: [USER_A, USER_B], total: 2 } }));
  mock.on("POST", "/api/v1/meeting-types", () => ({
    json: {
      meeting_type: { ...TEMPLATE, id: "00000000-0000-4000-8000-00000000m002", name: "Phone intake" },
      mutation: { seq: 2, updated_at: "2026-09-17T00:00:00Z", version: 1 },
    },
  }));
  mock.on("POST", `/api/v1/scheduling-groups/${GROUP_A.id}/members`, () => ({
    json: { ...GROUP_A, member_user_ids: [USER_A.id, USER_B.id] },
  }));
  mock.on(
    "DELETE",
    `/api/v1/scheduling-groups/${GROUP_A.id}/members/${USER_B.id}`,
    () => ({ json: { ...GROUP_A, member_user_ids: [USER_A.id] } }),
  );
  mock.install();
  return mock;
}

function page() {
  return (
    <AuthProvider>
      <DispatchSchedulingPage />
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

describe("/dispatch/scheduling — template admin", () => {
  it("renders templates with duration, buffer, window, scope, and auto-email badges", async () => {
    setupApi();
    render(page());
    const row = await screen.findByTestId("template-On-site visit");
    expect(row).toHaveTextContent("90m");
    expect(row).toHaveTextContent("buffer 15m");
    expect(row).toHaveTextContent("window 21d");
    expect(row).toHaveTextContent("Group · Service Desk");
    expect(row).toHaveTextContent("auto-email");
  });

  it("creates a meeting type through the dialog", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByTestId("template-On-site visit");

    fireEvent.click(screen.getByTestId("new-template"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Phone intake" } });
    fireEvent.click(screen.getByRole("button", { name: "Create template" }));

    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/meeting-types")).toHaveLength(1);
    });
    const body = mock.lastBody<Record<string, unknown>>("POST", "/api/v1/meeting-types");
    expect(body).toMatchObject({ name: "Phone intake", duration_minutes: 30, booking_window_days: 14 });
    expect(await screen.findByTestId("template-Phone intake")).toBeInTheDocument();
  });

  it("blocks save with typed validation errors and no POST", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByTestId("template-On-site visit");

    fireEvent.click(screen.getByTestId("new-template"));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "" } });
    fireEvent.change(screen.getByLabelText("Duration (min)"), { target: { value: "0" } });
    fireEvent.click(screen.getByTestId("mt-auto-email"));
    fireEvent.click(screen.getByRole("button", { name: "Create template" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Template name is required.");
    expect(screen.getByRole("alert")).toHaveTextContent("Duration must be a whole number between 5 and 480 minutes.");
    expect(screen.getByRole("alert")).toHaveTextContent("Auto-email needs both a subject and a body.");
    expect(mock.callsTo("POST", "/api/v1/meeting-types")).toHaveLength(0);
  });

  it("adds and removes group members", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByTestId("template-On-site visit");

    fireEvent.click(screen.getByLabelText("Pick a group to manage"));
    fireEvent.click(screen.getByRole("option", { name: /Service Desk \(1\)/ }));
    expect(await screen.findByTestId("group-panel-Service Desk")).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("Add member"));
    fireEvent.click(screen.getByRole("option", { name: "Boris Backup" }));
    fireEvent.click(screen.getByRole("button", { name: "Add member" }));
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/scheduling-groups/${GROUP_A.id}/members`)).toHaveLength(1);
    });
    const body = mock.lastBody<Record<string, unknown>>("POST", `/api/v1/scheduling-groups/${GROUP_A.id}/members`);
    expect(body.user_id).toBe(USER_B.id);
    expect(typeof body.idempotency_key).toBe("string");

    fireEvent.click(screen.getByRole("button", { name: "Remove Boris Backup from Service Desk" }));
    await waitFor(() => {
      expect(
        mock.callsTo("DELETE", `/api/v1/scheduling-groups/${GROUP_A.id}/members/${USER_B.id}`),
      ).toHaveLength(1);
    });
  });
});
