// Integration test: /settings/sla — calendar list with referenced-by
// counts, weekday-window editor hydration, ladder open/save flows PUTting
// an SDK-typed ReplaceLadderRequest, offset-suggested kind on add-row.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import SlaSettingsPage from "../../app/settings/sla/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/settings/sla",
}));

const CAL_ID = "20000000-0000-4000-8000-000000000001";
const POLICY_A = "30000000-0000-4000-8000-00000000000a";
const POLICY_B = "30000000-0000-4000-8000-00000000000b";

const CALENDAR = {
  id: CAL_ID,
  org_id: "o",
  name: "Corp hours",
  calendar: {
    name: "Corp hours",
    timezone: "Europe/Berlin",
    windows: [["08:00", "17:00"], ["08:00", "17:00"], null, null, null, null, null],
    holidays: ["2026-12-25"],
  },
};

const POLICIES = [
  { id: POLICY_A, org_id: "o", name: "Gold SLA", is_default: true, uses_embedded_calendar: false, calendar_id: CAL_ID, targets: { p1: 60, p4: 1440 } },
  { id: POLICY_B, org_id: "o", name: "Legacy", is_default: false, uses_embedded_calendar: true, calendar_id: null, targets: {} },
];

const LADDER_STEPS = [
  { id: "40000000-0000-4000-8000-000000000001", org_id: "o", policy_id: POLICY_A, step_index: 0, at_pct: 75, trigger_kind: "escalate", enabled: true, notify_user_ids: [] },
  { id: "40000000-0000-4000-8000-000000000002", org_id: "o", policy_id: POLICY_A, step_index: 1, at_pct: 100, trigger_kind: "breach", enabled: false, notify_user_ids: [] },
];

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/sla/calendars", () => ({ json: [CALENDAR] }));
  mock.on("GET", "/api/v1/sla/policies", () => ({ json: POLICIES }));
  mock.on("GET", `/api/v1/sla/policies/${POLICY_A}/ladder`, () => ({ json: LADDER_STEPS }));
  mock.install();
  return mock;
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/settings/sla integration", () => {
  it("lists calendars with referenced-by counts and policies with target summaries", async () => {
    setupApi();
    render(
      <AuthProvider>
        <SlaSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Corp hours");
    expect(await screen.findByText("referenced by 1 policy")).toBeInTheDocument();
    expect(screen.getByText(/Gold SLA/)).toBeInTheDocument();
    expect(screen.getByText("P1 60m · P4 1440m")).toBeInTheDocument();
    cleanup();
  });

  it("hydrates the editor from the selected calendar (windows, tz, holidays)", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <SlaSettingsPage />
      </AuthProvider>,
    );
    await screen.findByText("Corp hours");
    fireEvent.click(screen.getByRole("button", { name: /Corp hours/i }));

    expect(await screen.findByLabelText("Name")).toHaveValue("Corp hours");
    expect(screen.getByLabelText("Timezone")).toHaveValue("Europe/Berlin");
    expect(screen.getByLabelText("Monday open")).toBeChecked();
    expect(screen.getByLabelText("Tuesday closes")).toHaveValue("17:00");
    expect(screen.getByLabelText("Wednesday open")).not.toBeChecked();
    expect(screen.getByText("2026-12-25 ✕")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save calendar" })).toBeInTheDocument();
    // Clicking the holiday chip removes it from the draft.
    fireEvent.click(screen.getByText("2026-12-25 ✕"));
    expect(screen.queryByText("2026-12-25 ✕")).not.toBeInTheDocument();
    expect(mock.callsTo("GET", "/api/v1/sla/calendars").length).toBe(1);
    cleanup();
  });

  it("opens a ladder, shows fire-once note + disabled flag, and saves via PUT", async () => {
    const mock = setupApi();
    mock.on("PUT", `/api/v1/sla/policies/${POLICY_A}/ladder`, () => ({ json: LADDER_STEPS }));

    render(
      <AuthProvider>
        <SlaSettingsPage />
      </AuthProvider>,
    );
    const openButtons = await screen.findAllByRole("button", { name: /Escalation ladder/i });
    fireEvent.click(openButtons[0] as Element);

    await waitFor(() =>
      expect(screen.getAllByLabelText(/offset percent/i)).toHaveLength(2),
    );
    expect(screen.getByLabelText("Enable step 2")).not.toBeChecked();
    expect(screen.getByText(/fires at most once per ticket per target/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Save ladder" }));
    await waitFor(() => {
      const puts = mock.callsTo("PUT", `/api/v1/sla/policies/${POLICY_A}/ladder`);
      expect(puts.length).toBe(1);
      const body = puts[0]?.body as
        | { steps: { at_pct: number; trigger_kind: string; step_index: number }[] }
        | undefined;
      expect((body?.steps ?? []).map((s) => s.step_index)).toEqual([0, 1]);
      expect(body?.steps[0]).toMatchObject({ at_pct: 75, trigger_kind: "escalate" });
    });
    cleanup();
  });

  it("adding a row suggests a kind from the offset mapping before save", async () => {
    setupApi();
    render(
      <AuthProvider>
        <SlaSettingsPage />
      </AuthProvider>,
    );
    const buttons = await screen.findAllByRole("button", { name: /Escalation ladder/i });
    fireEvent.click(buttons[0] as Element);
    await screen.findByLabelText(/Step 1 offset percent/i);
    fireEvent.click(screen.getByRole("button", { name: "Add step" }));

    const newOffset = screen.getByLabelText(/Step 3 offset percent/i);
    expect(newOffset).toHaveValue(50);
    const newKind = screen.getByLabelText(/Step 3 notification kind/i) as HTMLSelectElement;
    // Escalate is preselected via suggestKindForOffset(50).
    expect(newKind.value).toBe("escalate");
    cleanup();
  });

  it("renders an empty state when no policies exist", async () => {
    const mock = new MockApi();
    mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
    mock.on("GET", "/api/v1/sla/calendars", () => ({ json: [] }));
    mock.on("GET", "/api/v1/sla/policies", () => ({ json: [] }));
    mock.install();
    render(
      <AuthProvider>
        <SlaSettingsPage />
      </AuthProvider>,
    );
    expect(await screen.findByText(/No calendars yet/)).toBeInTheDocument();
    expect(await screen.findByText(/No SLA policies are defined/)).toBeInTheDocument();
    cleanup();
  });
});
