// Scheduling pod: lifecycle chips, bookings trail, copy-once dialog,
// two-step revoke. The Wave 11 lane 4 backend contract is consumed as-is.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import { SchedulingPod } from "../../components/ticket-workspace/scheduling-pod";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  useParams: () => ({}),
}));

const TICKET_ID = "00000000-0000-4000-8000-00000000k001";
const TYPE_A = {
  id: "mt-1",
  org_id: "o",
  name: "On-site visit",
  description: null,
  duration_minutes: 60,
  buffer_minutes: 15,
  booking_window_days: 14,
  assignee_scope: "user",
  assignee_user_id: "u-1",
  assignee_group_id: null,
  auto_email_enabled: true,
  auto_email_subject: "Schedule your visit",
  auto_email_body: "Pick a slot",
  is_active: true,
  parameters: {},
  created_at: "2026-09-01T00:00:00Z",
  created_by: null,
};
const LINK_ACTIVE = {
  id: "00000000-0000-4000-8000-00000000l0a1",
  org_id: "o",
  meeting_type_id: "mt-1",
  ticket_id: TICKET_ID,
  label: "Ticket x",
  status: "active",
  token_hash: "h1",
  first_viewed_at: "2026-09-10T09:00:00Z",
  view_count: 2,
  scheduled_at: null,
  scheduled_appointment_id: null,
  expires_at: "2026-09-30T00:00:00Z",
  created_at: "2026-09-09T00:00:00Z",
  created_by: null,
};
const LINK_SCHEDULED = {
  ...LINK_ACTIVE,
  id: "00000000-0000-4000-8000-00000000l0b2",
  status: "used",
  scheduled_at: "2026-09-15T14:00:00Z",
  scheduled_appointment_id: "00000000-0000-4000-8000-00000000a0b1",
};
const BOOKING = {
  link_id: LINK_SCHEDULED.id,
  appointment_id: "00000000-0000-4000-8000-00000000a0b1",
  booker_name: "Dana Client",
  booker_email: "dana@client.test",
  booker_notes: "Mornings please",
  created_at: "2026-09-12T10:00:00Z",
};

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/meeting-types", () => ({ json: [TYPE_A] }));
  mock.on("GET", "/api/v1/scheduling-links", () => ({ json: [LINK_ACTIVE, LINK_SCHEDULED] }));
  mock.on("GET", `/api/v1/scheduling-links/${LINK_SCHEDULED.id}/bookings`, () => ({
    json: [BOOKING],
  }));
  mock.on("POST", `/api/v1/scheduling-links/${LINK_ACTIVE.id}/revoke`, () => ({ json: {} }));
  mock.on("POST", "/api/v1/scheduling-links", () => ({
    json: {
      link: LINK_ACTIVE,
      token: "tok-123",
      booking_path: "/s/tok-123",
    },
  }));
  mock.install();
  return mock;
}

function page() {
  return (
    <AuthProvider>
      <SchedulingPod ticketId={TICKET_ID} ticketTitle="Fix crusher" />
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

describe("SchedulingPod — send/manage", () => {
  it("renders lifecycle states with view counts and booked time", async () => {
    setupApi();
    render(page());
    expect(await screen.findByText("viewed")).toBeInTheDocument();
    expect(screen.getAllByText("2 views").length).toBeGreaterThan(0);
    expect(screen.getByText("scheduled")).toBeInTheDocument();
    expect(screen.getByText(/booked/)).toBeInTheDocument();
  });

  it("expands the booking trail on a scheduled link", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByText("scheduled");
    fireEvent.click(screen.getByTestId("scheduling-bookings-toggle-00000000"));
    await waitFor(() => {
      expect(mock.callsTo("GET", `/api/v1/scheduling-links/${LINK_SCHEDULED.id}/bookings`)).toHaveLength(1);
    });
    expect(await screen.findByText("Dana Client")).toBeInTheDocument();
    expect(screen.getByText("dana@client.test")).toBeInTheDocument();
    expect(screen.getByText(/Mornings please/)).toBeInTheDocument();
  });

  it("revokes through a two-step confirm", async () => {
    const mock = setupApi();
    render(page());
    await screen.findByText("viewed");
    fireEvent.click(screen.getByTestId("scheduling-revoke-00000000"));
    fireEvent.click(screen.getByTestId("scheduling-revoke-confirm-00000000"));
    await waitFor(() => {
      expect(mock.callsTo("POST", `/api/v1/scheduling-links/${LINK_ACTIVE.id}/revoke`)).toHaveLength(1);
    });
  });

  it("creates a link and surfaces the copy-once URL", async () => {
    setupApi();
    render(page());
    await screen.findByText("viewed");
    fireEvent.click(screen.getByTestId("scheduling-create"));
    expect(await screen.findByTestId("scheduling-copy-once")).toBeInTheDocument();
    const input = screen.getByTestId("scheduling-token") as HTMLInputElement;
    expect(input.value).toContain("/s/tok-123");
  });

  it("links to the template admin", async () => {
    setupApi();
    render(page());
    expect(await screen.findByTestId("scheduling-manage-templates")).toBeInTheDocument();
  });
});
