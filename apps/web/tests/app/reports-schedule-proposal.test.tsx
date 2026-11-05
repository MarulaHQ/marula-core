// Integration test: the Schedules tab's "Describe a schedule" affordance.
//
// Three promises, one per case: a sentence the parser understands fills the
// EXISTING create form with visible fields and saves through the EXISTING POST;
// a sentence it cannot express is refused with the raw text still in the box,
// and nothing is filled in; an edited field rewrites the sentence and the save
// carries the edit.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import ReportsPage from "../../app/reports/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/reports",
  useSearchParams: () => new URLSearchParams(),
}));

const DEF_ID = "00000000-0000-4000-8000-00000000d001";
const USER_ID = "00000000-0000-4000-8000-00000000u001";

const CATALOG = { views: [] };

const DEFINITIONS = {
  definitions: [
    {
      id: DEF_ID,
      name: "Weekly exec digest",
      description: null,
      spec: { view: "tickets", group_by: [], measures: [{ agg: "count" }], viz: "table" },
      scope: "self",
      created_by: ME_FIXTURE.user_id,
      created_at: "2026-09-01T00:00:00Z",
      updated_at: "2026-09-12T00:00:00Z",
    },
  ],
  next_cursor: null,
};

const SCHEDULES = { schedules: [], next_cursor: null };

const USERS = {
  users: [
    {
      id: USER_ID,
      display_name: "Alice Admin",
      email: "alice@example.com",
      is_active: true,
    },
  ],
};

function setupApi() {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/report-catalog", () => ({ json: CATALOG }));
  mock.on("GET", "/api/v1/report-definitions", () => ({ json: DEFINITIONS }));
  mock.on("GET", "/api/v1/report-schedules", () => ({ json: SCHEDULES }));
  mock.on("GET", "/api/v1/org/users", () => ({ json: USERS }));
  mock.on("POST", "/api/v1/report-schedules", (ctx) => ({
    json: {
      schedule: { id: "00000000-0000-4000-8000-00000000s001", ...(ctx.body as object) },
      mutation: {},
    },
  }));
  mock.install();
  return mock;
}

async function openSchedulesTab() {
  fireEvent.click(await screen.findByRole("tab", { name: "Schedules" }, { timeout: 5000 }));
}

async function describeSchedule(text: string) {
  const input = await screen.findByLabelText("In your own words", {}, { timeout: 5000 });
  fireEvent.change(input, { target: { value: text } });
  fireEvent.click(screen.getByRole("button", { name: "Read it" }));
  return input;
}

async function fillAndSave(recipientLabel: string) {
  const dialog = await screen.findByRole("dialog");
  fireEvent.change(within(dialog).getByLabelText("Schedule name"), {
    target: { value: "Weekly exec digest" },
  });
  fireEvent.click(within(dialog).getByLabelText("Report"));
  fireEvent.click(await screen.findByRole("option", { name: "Weekly exec digest" }));
  fireEvent.click(
    await within(dialog).findByRole("checkbox", { name: new RegExp(recipientLabel) }, { timeout: 5000 }),
  );
  fireEvent.click(within(dialog).getByRole("button", { name: "Save schedule" }));
}

beforeEach(() => {
  cleanup();
  window.localStorage.clear();
  seedAuthTokens();
});

describe("/reports — describe a schedule", () => {
  it("parses a weekly sentence, shows its fields, and saves them through the existing POST", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );
    await openSchedulesTab();

    await describeSchedule("every Monday at 09:00, Europe/London");

    // The sentence restates exactly what the fields say, and nothing is guessed.
    expect(await screen.findByText("Every Monday at 09:00, Europe/London")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Use this schedule" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByLabelText("Time of day")).toHaveValue("09:00");
    expect(within(dialog).getByLabelText("Time zone")).toHaveValue("Europe/London");
    // Radix renders the selected weekday in the trigger.
    expect(within(dialog).getByLabelText("Weekday")).toHaveTextContent("Monday");

    await fillAndSave("Alice Admin");

    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/report-schedules")).toHaveLength(1);
    });
    const body = mock.lastBody<Record<string, unknown>>("POST", "/api/v1/report-schedules");
    expect(body).toMatchObject({
      name: "Weekly exec digest",
      report_definition_id: DEF_ID,
      cadence: "weekly",
      time_of_day: "09:00",
      day_of_week: 0,
      day_of_month: null,
      timezone: "Europe/London",
      recipients: [USER_ID],
    });
    expect(typeof body.idempotency_key).toBe("string");
  });

  it("refuses a cadence the API cannot express and leaves the sentence untouched", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );
    await openSchedulesTab();

    const input = await describeSchedule("every other Tuesday at 9am");

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/never every other week/);
    expect(alert).toHaveTextContent(/Nothing was filled in/);
    // The raw sentence is still there to edit, and no form was opened.
    expect(input).toHaveValue("every other Tuesday at 9am");
    expect(screen.queryByRole("button", { name: "Use this schedule" })).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(mock.callsTo("POST", "/api/v1/report-schedules")).toHaveLength(0);
  });

  it("refuses 'every weekday' rather than reading it as one weekday", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );
    await openSchedulesTab();

    await describeSchedule("every weekday at 8:30");

    expect(await screen.findByRole("alert")).toHaveTextContent(/fires on one weekday/);
  });

  it("rewrites the sentence when a parsed field is edited, and saves the edit", async () => {
    const mock = setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );
    await openSchedulesTab();

    await describeSchedule("monthly on the 15th at 7am, Europe/London");
    expect(await screen.findByText("Every month on the 15th at 07:00, Europe/London")).toBeInTheDocument();

    // Edit the day: the sentence must follow the field, not the original text.
    fireEvent.click(screen.getByLabelText("Day of the month"));
    fireEvent.click(await screen.findByRole("option", { name: "20" }));
    expect(screen.getByText("Every month on the 20th at 07:00, Europe/London")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Use this schedule" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByLabelText("Day of the month")).toHaveTextContent("20");

    await fillAndSave("Alice Admin");
    await waitFor(() => {
      expect(mock.callsTo("POST", "/api/v1/report-schedules")).toHaveLength(1);
    });
    expect(mock.lastBody<Record<string, unknown>>("POST", "/api/v1/report-schedules")).toMatchObject({
      cadence: "monthly",
      time_of_day: "07:00",
      day_of_week: null,
      day_of_month: 20,
      timezone: "Europe/London",
    });
  });

  it("clears a stale proposal when the sentence is edited, rather than showing two answers", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );
    await openSchedulesTab();

    const input = await describeSchedule("daily at 6pm, Europe/London");
    expect(await screen.findByText("Every day at 18:00, Europe/London")).toBeInTheDocument();

    fireEvent.change(input, { target: { value: "daily at 7pm, Europe/London" } });
    expect(screen.queryByText("Every day at 18:00, Europe/London")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Read it" }));
    expect(await screen.findByText("Every day at 19:00, Europe/London")).toBeInTheDocument();
  });

  it("blocks Use this schedule while an edited field is one the route would refuse", async () => {
    setupApi();
    render(
      <AuthProvider>
        <ReportsPage />
      </AuthProvider>,
    );
    await openSchedulesTab();

    await describeSchedule("daily at 6pm, Europe/London");
    expect(await screen.findByText("Every day at 18:00, Europe/London")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Time (HH:MM)"), { target: { value: "25:00" } });

    expect(await screen.findByText(/Write the time as HH:MM/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Use this schedule" })).toBeDisabled();
  });
});
