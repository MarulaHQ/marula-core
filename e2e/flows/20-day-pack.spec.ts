// Flow 20 — the tech's day pack (Wave 32 lane 8): /today builds route-aware
// trip segments from the travel model's recorded leg origins, consolidates
// the grab list (pins, dispatch rules, checklist mentions) with where-to-grab
// and honest availability, ticks plan notes WITHOUT moving the ledger,
// checks a tool out from the panel row (lane 3's transaction), and adds
// tomorrow's list tonight when the van does not return to the office first.
//
// Order-independent: stages its own client/sites/stock/appointments with
// per-run unique names in a free window, then restores what it changed
// (picks and ticks deleted, tools checked back in, items archived,
// appointments cancelled). Runs in NONE mode, which is the honest posture
// this panel is built for: heuristic legs, no provider.
import { expect, test } from "../helpers/harness";
import { authedFetch, firstDemoTicket } from "../helpers/api";

interface MeView {
  user_id: string;
}

interface ClientView {
  id: string;
  name: string;
}

interface SiteView {
  id: string;
  name: string;
}

interface LocationView {
  id: string;
  name: string;
  kind: string;
  active: boolean;
}

interface ItemView {
  id: string;
  name: string;
  is_tool: boolean;
  kind: string;
}

interface AppointmentView {
  id: string;
  starts_at: string;
  ends_at: string;
  travel_from: { kind: string } | null;
}

interface PickResponse {
  pick: { id: string; line_key?: string };
}

interface DayPackLine {
  line_key: string;
  provenance: string;
  stock_item_id: string;
  item_name: string;
  availability: { status: string; available_units: number; required_units: number };
  source: { location_id: string; name: string } | null;
  tick: unknown;
}

interface DayPackTrip {
  index: number;
  opens_at_office: boolean;
  stops: { appointment_id: string }[];
  lines: DayPackLine[];
}

interface DayPack {
  has_stops: boolean;
  trips: DayPackTrip[];
  tomorrow?: {
    reason: string;
    lines: DayPackLine[];
  } | null;
  counts: { stops: number; lines: number; collected: number; unavailable: number };
}

interface CheckoutDetail {
  checkout: { id: string; from_location_id: string; holder_user_id: string };
  lines: { stock_item_id: string }[];
}

test("the day pack groups trips, ticks plan notes, and checks out from /today", async ({
  authedPage,
  ownerTokens,
}, testInfo) => {
  const token = ownerTokens.access_token;
  const api = authedFetch(token);
  const run = Date.now() % 1000000;
  const day = new Date().toISOString().slice(0, 10);
  const tomorrow = new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString().slice(0, 10);

  const me = await api<MeView>("/me");
  const clients = await api<ClientView[]>("/clients?limit=5");
  const client = clients[0];
  expect(client, "demo org must have a client").toBeTruthy();

  const siteA = await api<SiteView>(`/clients/${client.id}/sites`, {
    method: "POST",
    body: { name: `E2E Pack North ${run}`, latitude: 52.5, longitude: 13.4 },
  });
  const siteB = await api<SiteView>(`/clients/${client.id}/sites`, {
    method: "POST",
    body: { name: `E2E Pack South ${run}`, latitude: 52.52, longitude: 13.45 },
  });

  // The demo seed carries no stock locations; this flow stages its own shelf.
  const pickup = await api<LocationView>("/stock-locations", {
    method: "POST",
    body: { name: `E2E Pack shelf ${run}`, kind: "warehouse" },
  });

  // Scratch stock: a consumable and a tool with one serial, both on the
  // shelf the day pack should point at.
  const tape = await api<ItemView>("/stock-items", {
    method: "POST",
    body: {
      name: `E2E Pack Tape ${run}`,
      kind: "consumable",
      unit: "each",
      currency: "USD",
      sku: `E2E-PACK-TAPE-${run}`,
    },
  });
  await api("/stock/receive", {
    method: "POST",
    body: { item_id: tape.id, location_id: pickup.id, qty_milli: 8000 },
  });
  const crimper = await api<ItemView>("/stock-items", {
    method: "POST",
    body: {
      name: `E2E Pack Crimper ${run}`,
      kind: "serialized",
      is_tool: true,
      unit: "each",
      currency: "USD",
      sku: `E2E-PACK-CRIMP-${run}`,
    },
  });
  await api("/stock/receive", {
    method: "POST",
    body: {
      item_id: crimper.id,
      location_id: pickup.id,
      qty_milli: 1000,
      serial_numbers: [`E2E-PACK-${run}`],
    },
  });

  // The day starts at a site, so the first trip does NOT open at the
  // office. Tomorrow starts on the road too: direct-to-site.
  await api("/dispatch/journey-start", {
    method: "PUT",
    body: {
      user_id: me.user_id,
      date: day,
      kind: "site",
      site_id: siteA.id,
      label: `E2E Pack Start ${run}`,
    },
  });
  await api("/dispatch/journey-start", {
    method: "PUT",
    body: {
      user_id: me.user_id,
      date: tomorrow,
      kind: "site",
      site_id: siteA.id,
      label: `E2E Pack Start ${run}`,
    },
  });

  const ticket = await firstDemoTicket(token);

  // Free back-to-back window on the owner's day (dirty demo org safe).
  const dayView = await api<{
    technicians: {
      user_id: string;
      appointments: { starts_at: string; ends_at: string }[];
    }[];
  }>(`/dispatch/day?date=${day}`);
  const own = dayView.technicians.find((tech) => tech.user_id === me.user_id);
  const busy = (own?.appointments ?? [])
    .map((a) => [Date.parse(a.starts_at), Date.parse(a.ends_at)] as const)
    .sort((left, right) => left[0] - right[0]);
  const windowStart = Date.parse(`${day}T06:00:00Z`);
  const windowEnd = Date.parse(`${day}T22:00:00Z`);
  let slotStart = windowStart;
  for (const [start, end] of busy) {
    if (start - slotStart >= 150 * 60 * 1000) break;
    slotStart = Math.max(slotStart, end);
  }
  expect(slotStart + 150 * 60 * 1000, "the day needs a free 2.5h window").toBeLessThanOrEqual(
    windowEnd,
  );
  const slotAEnd = new Date(slotStart + 60 * 60 * 1000).toISOString();
  const slotBEnd = new Date(slotStart + 120 * 60 * 1000).toISOString();

  const created: string[] = [];
  const pickIds: string[] = [];
  const tickKeys: { date: string; lineKey: string }[] = [];
  let checkoutId: string | null = null;
  try {
    const appointmentA = await api<AppointmentView>("/appointments", {
      method: "POST",
      body: {
        title: `E2E Pack A ${run}`,
        ticket_id: ticket.id,
        client_id: client.id,
        site_id: siteA.id,
        kind: "on_site",
        starts_at: new Date(slotStart).toISOString(),
        ends_at: slotAEnd,
      },
    });
    created.push(appointmentA.id);
    expect(appointmentA.travel_from?.kind, "travel runs from the site start").toBe("site");

    const appointmentB = await api<AppointmentView>("/appointments", {
      method: "POST",
      body: {
        title: `E2E Pack B ${run}`,
        ticket_id: ticket.id,
        client_id: client.id,
        site_id: siteB.id,
        kind: "on_site",
        starts_at: slotAEnd,
        ends_at: slotBEnd,
      },
    });
    created.push(appointmentB.id);

    // Tomorrow's stop, same ticket and site: the preview's raw material.
    const tomorrowView = await api<{
      technicians: {
        user_id: string;
        appointments: { starts_at: string; ends_at: string }[];
      }[];
    }>(`/dispatch/day?date=${tomorrow}`);
    const ownTomorrow = tomorrowView.technicians.find((tech) => tech.user_id === me.user_id);
    const tomorrowBusy = (ownTomorrow?.appointments ?? [])
      .map((a) => [Date.parse(a.starts_at), Date.parse(a.ends_at)] as const)
      .sort((left, right) => left[0] - right[0]);
    let tomorrowSlot = Date.parse(`${tomorrow}T06:00:00Z`);
    for (const [start, end] of tomorrowBusy) {
      if (start - tomorrowSlot >= 90 * 60 * 1000) break;
      tomorrowSlot = Math.max(tomorrowSlot, end);
    }
    const tomorrowStop = await api<AppointmentView>("/appointments", {
      method: "POST",
      body: {
        title: `E2E Pack Tomorrow ${run}`,
        ticket_id: ticket.id,
        client_id: client.id,
        site_id: siteA.id,
        kind: "on_site",
        starts_at: new Date(tomorrowSlot).toISOString(),
        ends_at: new Date(tomorrowSlot + 60 * 60 * 1000).toISOString(),
      },
    });
    created.push(tomorrowStop.id);

    // Pins: the job needs 4 tape ends and 1 crimper.
    const tapePin = await api<PickResponse>("/day-pack/picks", {
      method: "POST",
      body: {
        ticket_id: ticket.id,
        stock_item_id: tape.id,
        qty_milli: 4000,
        note: `E2E pack ${run}`,
      },
    });
    pickIds.push(tapePin.pick.id);
    const crimperPin = await api<PickResponse>("/day-pack/picks", {
      method: "POST",
      body: { ticket_id: ticket.id, stock_item_id: crimper.id, qty_milli: 1000 },
    });
    pickIds.push(crimperPin.pick.id);

    // API: one trip so far (both legs leave the site start), first line is
    // the pin, and the source is the shelf it was received into.
    const pack = await api<DayPack>(`/day-pack?date=${day}`);
    const tripA = pack.trips.find((trip) =>
      trip.stops.some((stop) => stop.appointment_id === appointmentA.id),
    );
    expect(tripA, "the pack must carry the first stop's trip").toBeTruthy();
    expect(tripA!.opens_at_office).toBe(false);
    // Back-to-back legs from the same site start seal ONE trip pack.
    const tripBSealed = pack.trips.find((trip) =>
      trip.stops.some((stop) => stop.appointment_id === appointmentB.id),
    );
    expect(tripBSealed, "the second stop rides the same trip").toBeTruthy();
    expect(tripBSealed!.index).toBe(tripA!.index);
    const tapeLine = tripA!.lines.find((line) => line.stock_item_id === tape.id);
    expect(tapeLine, "the pinned consumable is a grab line").toBeTruthy();
    expect(tapeLine!.provenance).toBe("pick");
    expect(tapeLine!.availability.status).toBe("available");
    expect(tapeLine!.source?.location_id).toBe(pickup.id);

    // Tomorrow preview: direct to site, so tonight's list carries the pins.
    expect(pack.tomorrow?.reason).toBe("direct_to_site");
    expect(
      pack.tomorrow?.lines.some((line) => line.stock_item_id === tape.id),
    ).toBe(true);

    // API: an office return re-opens the trip pack. Re-pinning the day
    // start at the office and re-saving the second stop stamps its leg as
    // office-origin while the first stop keeps its site origin.
    await api("/dispatch/journey-start", {
      method: "PUT",
      body: {
        user_id: me.user_id,
        date: day,
        kind: "office",
        label: `E2E Pack Office ${run}`,
        address: `E2E Pack Office ${run}`,
        latitude: 52.3,
        longitude: 13.2,
      },
    });
    const resaved = await api<AppointmentView>(`/appointments/${appointmentB.id}`, {
      method: "PATCH",
      body: { title: `E2E Pack B ${run} (resaved)` },
    });
    expect(resaved.travel_from?.kind, "the re-saved leg leaves the office").toBe("office");

    const regrouped = await api<DayPack>(`/day-pack?date=${day}`);
    const regroupedA = regrouped.trips.find((trip) =>
      trip.stops.some((stop) => stop.appointment_id === appointmentA.id),
    );
    const regroupedB = regrouped.trips.find((trip) =>
      trip.stops.some((stop) => stop.appointment_id === appointmentB.id),
    );
    expect(regroupedA, "stop A keeps its trip").toBeTruthy();
    expect(regroupedB, "stop B opens its own trip").toBeTruthy();
    expect(regroupedA!.index).not.toBe(regroupedB!.index);
    // The first trip now opens at the office (the day start moved there);
    // the second trip is the recorded office RETURN, so it opens there too.
    expect(regroupedA!.opens_at_office).toBe(true);
    expect(regroupedB!.opens_at_office).toBe(true);
    expect(regroupedB!.stops[0]!.appointment_id).toBe(appointmentB.id);

    // UI: the panel renders the trips, the office chip, and tomorrow.
    await authedPage.goto("/today");
    const panel = authedPage.getByTestId("day-pack");
    await expect(panel).toBeVisible({ timeout: 20_000 });
    await expect(authedPage.getByTestId("day-pack-office-chip").first()).toContainText(
      "Collect at the office",
    );
    // The line shows twice by design: today's trip AND tonight's preview.
    const tapeRows = authedPage.getByTestId("day-pack-line").filter({ hasText: tape.name });
    await expect(tapeRows).toHaveCount(2);
    const tapeRow = tapeRows.first();
    await expect(tapeRow).toBeVisible();
    await expect(authedPage.getByTestId("day-pack-tomorrow")).toBeVisible();
    await authedPage.screenshot({
      path: `design/qa-p32-today-day-pack/captures/${testInfo.project.name}-day-pack.png`,
      fullPage: true,
    });

    // UI: ticking writes a plan note and survives a reload; unticking removes it.
    const beforeTick = await api<DayPack>(`/day-pack?date=${day}`);
    const tickBox = tapeRow.getByTestId("day-pack-tick");
    await tickBox.click();
    await expect(tickBox).toHaveAttribute("aria-pressed", "true", { timeout: 20_000 });
    const afterTick = await api<DayPack>(`/day-pack?date=${day}`);
    expect(afterTick.counts.collected).toBeGreaterThanOrEqual(1);
    tickKeys.push({ date: day, lineKey: `pick:${tapePin.pick.id}` });
    await tickBox.click();
    await expect(tickBox).toHaveAttribute("aria-pressed", "false", { timeout: 20_000 });
    const afterUntick = await api<DayPack>(`/day-pack?date=${day}`);
    expect(afterUntick.counts.collected).toBe(beforeTick.counts.collected);

    // UI: one-tap checkout from the tool row (lane 3's transaction).
    const toolRow = authedPage
      .getByTestId("day-pack-line")
      .filter({ hasText: crimper.name })
      .first();
    await toolRow.getByTestId("day-pack-checkout").click();
    const dialog = authedPage.getByTestId("tools-checkout-dialog");
    await expect(dialog).toBeVisible({ timeout: 20_000 });
    await authedPage.getByTestId("tools-checkout-save").click();
    await expect(dialog).toBeHidden({ timeout: 20_000 });

    const checkouts = await api<CheckoutDetail[]>("/tool-checkouts?status=open&limit=50");
    const mine = checkouts.find((row) =>
      row.lines.some((line) => line.stock_item_id === crimper.id),
    );
    expect(mine, "the panel checkout must land in the checkout register").toBeTruthy();
    expect(mine!.checkout.from_location_id).toBe(pickup.id);
    checkoutId = mine!.checkout.id;

    // Availability sharpens after the unit leaves: no unit in stock.
    const afterCheckout = await api<DayPack>(`/day-pack?date=${day}`);
    const toolLine = afterCheckout.trips
      .flatMap((trip) => trip.lines)
      .find((line) => line.stock_item_id === crimper.id);
    expect(toolLine?.availability.available_units).toBe(0);

    // Route-aware flip: an office start tomorrow suppresses tonight's list.
    await api("/dispatch/journey-start", {
      method: "PUT",
      body: {
        user_id: me.user_id,
        date: tomorrow,
        kind: "office",
        label: `E2E Pack Office ${run}`,
        address: `E2E Pack Office ${run}`,
        latitude: 52.3,
        longitude: 13.2,
      },
    });
    const flipped = await api<DayPack>(`/day-pack?date=${day}`);
    expect(flipped.tomorrow ?? null).toBeNull();
    await authedPage.reload();
    await expect(authedPage.getByTestId("day-pack")).toBeVisible({ timeout: 20_000 });
    await expect(authedPage.getByTestId("day-pack-tomorrow")).toHaveCount(0);

    // The config surface renders the rules table (empty or populated).
    await authedPage.goto("/settings/toolboxes");
    await expect(authedPage.getByTestId("toolbox-new-name")).toBeVisible({ timeout: 20_000 });
    await authedPage.screenshot({
      path: `design/qa-p32-today-day-pack/captures/${testInfo.project.name}-toolbox-config.png`,
      fullPage: true,
    });
  } finally {
    for (const pickId of pickIds) {
      await api(`/day-pack/picks/${pickId}`, { method: "DELETE" }).catch(() => undefined);
    }
    for (const tick of tickKeys) {
      await api(
        `/day-pack/ticks?date=${tick.date}&line_key=${encodeURIComponent(tick.lineKey)}`,
        { method: "DELETE" },
      ).catch(() => undefined);
    }
    if (checkoutId !== null) {
      await api(`/tool-checkouts/${checkoutId}/checkin`, {
        method: "POST",
        body: { lines: [] },
      }).catch(() => undefined);
    }
    for (const id of created) {
      await api(`/appointments/${id}`, {
        method: "PATCH",
        body: { status: "cancelled" },
      }).catch(() => undefined);
    }
    await api(`/stock-items/${crimper.id}/archive`, { method: "POST", body: {} }).catch(
      () => undefined,
    );
    await api(`/stock-items/${tape.id}/archive`, { method: "POST", body: {} }).catch(
      () => undefined,
    );
  }
});
