// Flow 19 — Dispatch v3 routing (Wave 32 lane 7): a dispatcher-editable
// day start, automatic heuristic travel at save time, the typed
// zero-transfer warning surfaced on the board, the calendar travel band
// drawn apart from the appointment block, and mileage reconciliation
// turning a travel draft into a reimbursable expense.
//
// Order-independent: stages its own client/site/appointments/draft with
// per-run unique names, and cancels the appointments it created at the end.
// Runs in NONE mode (the stack configures no routing provider), which is
// exactly the posture this flow proves: honest estimates + silent
// suggestions.
import { expect, test } from "../helpers/harness";
import { authedFetch, firstDemoTicket } from "../helpers/api";

interface ClientView {
  id: string;
  name: string;
}

interface SiteView {
  id: string;
  name: string;
}

interface AppointmentView {
  id: string;
  starts_at: string;
  ends_at: string;
  travel_minutes: number;
  travel_provider: string | null;
  travel_quality: string | null;
}

interface TravelWarning {
  kind: string;
  appointment_id: string | null;
  required_minutes: number | null;
  short_minutes: number | null;
}

interface TechnicianJourney {
  user_id: string;
  display_name: string;
  stops: { appointment_id: string }[];
  warnings: TravelWarning[];
}

interface RoutingDay {
  status: {
    provider_id: string;
    quality: string;
    measured: boolean;
    suggestions_available: boolean;
  };
  technicians: TechnicianJourney[];
  suggestions: { available: boolean; items: unknown[] };
}

interface TravelDraftView {
  id: string;
  status: string;
  expense_id: string | null;
}

interface ReconciliationView {
  currency: string;
  mileage_rate_minor: number;
  mileage_rate_unit: string;
  rows: {
    user_id: string;
    draft_count: number;
    expensed_count: number;
    reimbursement_minor: number;
  }[];
  totals: {
    draft_count: number;
    expensed_count: number;
    reimbursement_minor: number;
  };
}

interface MeView {
  user_id: string;
  display_name: string;
}

test("travel books from the day start, warns on zero transfer, and reconciles mileage", async ({
  authedPage,
  ownerTokens,
}, testInfo) => {
  const token = ownerTokens.access_token;
  const api = authedFetch(token);
  const run = Date.now() % 1000000;
  const day = new Date().toISOString().slice(0, 10);

  const me = await api<MeView>("/me");
  const meId = me.user_id;
  expect(meId, "/me must expose the caller id").toBeTruthy();

  const clients = await api<ClientView[]>("/clients?limit=5");
  const client = clients[0];
  expect(client, "demo org must have a client").toBeTruthy();

  const siteA = await api<SiteView>(`/clients/${client.id}/sites`, {
    method: "POST",
    body: {
      name: `E2E Routing North ${run}`,
      latitude: 52.5,
      longitude: 13.4,
    },
  });
  const siteB = await api<SiteView>(`/clients/${client.id}/sites`, {
    method: "POST",
    body: {
      name: `E2E Routing South ${run}`,
      latitude: 52.52,
      longitude: 13.45,
    },
  });

  // A dispatcher pins the day's start at a home address; travel estimates
  // run from there (NONE mode = the built-in heuristic, labeled honestly).
  await api("/dispatch/journey-start", {
    method: "PUT",
    body: {
      user_id: meId,
      date: day,
      kind: "address",
      label: `E2E Home ${run}`,
      address: `E2E Home ${run}`,
      latitude: 52.3,
      longitude: 13.2,
    },
  });

  const ticket = await firstDemoTicket(token);
  const now = new Date();

  // Find a free back-to-back window on the owner's day so the flow is
  // order-independent on a dirty demo org (earlier runs may have left
  // appointments behind).
  const dayView = await api<{
    technicians: {
      user_id: string;
      appointments: { starts_at: string; ends_at: string }[];
    }[];
  }>(`/dispatch/day?date=${day}`);
  const own = dayView.technicians.find((tech) => tech.user_id === meId);
  const busy = (own?.appointments ?? [])
    .map((a) => [Date.parse(a.starts_at), Date.parse(a.ends_at)] as const)
    .sort((left, right) => left[0] - right[0]);
  const windowStart = Date.parse(`${day}T08:00:00Z`);
  const windowEnd = Date.parse(`${day}T22:00:00Z`);
  let slotStart = windowStart;
  for (const [start, end] of busy) {
    if (start - slotStart >= 150 * 60 * 1000) break;
    slotStart = Math.max(slotStart, end);
  }
  expect(slotStart + 150 * 60 * 1000, "the day needs a free 2.5h window").toBeLessThanOrEqual(
    windowEnd,
  );
  const slotAStart = new Date(slotStart).toISOString();
  const slotAEnd = new Date(slotStart + 60 * 60 * 1000).toISOString();
  const slotBEnd = new Date(slotStart + 120 * 60 * 1000).toISOString();

  const created: string[] = [];
  try {
    const appointmentA = await api<AppointmentView>("/appointments", {
      method: "POST",
      body: {
        title: `E2E Routing A ${run}`,
        ticket_id: ticket.id,
        client_id: client.id,
        site_id: siteA.id,
        kind: "on_site",
        starts_at: slotAStart,
        ends_at: slotAEnd,
      },
    });
    created.push(appointmentA.id);
    // Travel computes at save time from the pinned start point.
    expect(appointmentA.travel_minutes).toBeGreaterThan(0);
    expect(appointmentA.travel_provider).toBe("none");
    expect(appointmentA.travel_quality).toBe("heuristic");

    // Back-to-back at a different site: zero transfer time for a real drive.
    const appointmentB = await api<AppointmentView>("/appointments", {
      method: "POST",
      body: {
        title: `E2E Routing B ${run}`,
        ticket_id: ticket.id,
        client_id: client.id,
        site_id: siteB.id,
        kind: "on_site",
        starts_at: slotAEnd,
        ends_at: slotBEnd,
      },
    });
    created.push(appointmentB.id);

  // API: typed warning + silent suggestions in NONE mode.
  const routing = await api<RoutingDay>(`/dispatch/routing?date=${day}`);
  expect(routing.status.provider_id).toBe("none");
  expect(routing.status.quality).toBe("heuristic");
  expect(routing.status.suggestions_available).toBe(false);
  expect(routing.suggestions.available).toBe(false);
  expect(routing.suggestions.items).toHaveLength(0);
  const journey = routing.technicians.find((tech) =>
    tech.stops.some((stop) => stop.appointment_id === appointmentB.id),
  );
  expect(journey, "the day view must carry the technician's journey").toBeTruthy();
  const zeroTransfer = journey!.warnings.find(
    (warning) =>
      warning.kind === "zero_transfer" && warning.appointment_id === appointmentB.id,
  );
  expect(zeroTransfer, "back-to-back stops need a zero_transfer warning").toBeTruthy();
  expect(zeroTransfer!.required_minutes ?? 0).toBeGreaterThan(0);

  // UI: the board's routing strip says it in words; suggestions stay absent.
  await authedPage.goto("/dispatch");
  const strip = authedPage.getByTestId("routing-strip");
  await expect(strip).toBeVisible({ timeout: 20_000 });
  await expect(strip).toContainText("no transfer time");
  await expect(strip).not.toContainText("Route suggestions");
  await authedPage.screenshot({
    path: `design/qa-p32-dispatch-routing/captures/${testInfo.project.name}-dispatch-strip.png`,
  });

  // UI: the calendar draws travel as its own band above the appointment.
  await authedPage.goto("/calendar");
  await expect(
    authedPage.locator(`[data-testid="week-travel-${appointmentA.id}"]`),
  ).toBeVisible({ timeout: 20_000 });
  await authedPage.screenshot({
    path: `design/qa-p32-dispatch-routing/captures/${testInfo.project.name}-calendar-travel.png`,
  });

  // Reconciliation: log actual mileage, submit the draft, book the expense.
  const draft = await api<TravelDraftView>("/dispatch/travel-drafts", {
    method: "POST",
    body: {
      user_id: meId,
      ticket_id: ticket.id,
      appointment_id: appointmentB.id,
      started_at: new Date(now.getTime() - 90 * 60 * 1000).toISOString(),
      ended_at: new Date(now.getTime() - 60 * 60 * 1000).toISOString(),
      distance: 20,
      distance_unit: "mi",
      vehicle: "E2E Van",
      billable: true,
    },
  });
  await api(`/dispatch/travel-drafts/${draft.id}/submit`, {
    method: "POST",
    body: {},
  });
  await api(`/dispatch/travel-drafts/${draft.id}/expense`, {
    method: "POST",
    body: { idempotency_key: crypto.randomUUID() },
  });

  const from = new Date(now.getTime() - 24 * 60 * 60 * 1000).toISOString();
  const to = new Date(now.getTime() + 24 * 60 * 60 * 1000).toISOString();
  const reconciliation = await api<ReconciliationView>(
    `/dispatch/reconciliation?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}&user_id=${meId}`,
  );
  expect(reconciliation.mileage_rate_unit).toBe("mi");
  const row = reconciliation.rows.find((r) => r.user_id === meId);
  expect(row, "the caller's reconciliation row").toBeTruthy();
  expect(row!.draft_count).toBeGreaterThanOrEqual(1);
  expect(row!.expensed_count).toBeGreaterThanOrEqual(1);
  // 20 mi at the 67 minor-unit default rate.
  expect(row!.reimbursement_minor).toBeGreaterThanOrEqual(1340);

  // UI: the reconciliation card renders the rollup and the rate.
  await authedPage.goto("/dispatch/travel");
  const card = authedPage.getByTestId("reconciliation-card");
  await expect(card).toBeVisible({ timeout: 20_000 });
  await expect(card).toContainText("Reimbursable");
  await authedPage.screenshot({
    path: `design/qa-p32-dispatch-routing/captures/${testInfo.project.name}-reconciliation.png`,
    fullPage: true,
  });

  } finally {
    // Restore: cancel the appointments this flow created (drafts are
    // immutable records once submitted; the journey start is a set, so the
    // auto-suggestion simply goes unused tomorrow).
    for (const id of created) {
      await api(`/appointments/${id}`, {
        method: "PATCH",
        body: { status: "cancelled" },
      }).catch(() => undefined);
    }
  }
});
