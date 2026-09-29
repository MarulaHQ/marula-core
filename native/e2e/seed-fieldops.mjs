#!/usr/bin/env node
// Field-ops E2E fixtures (Wave 33 lane 2), idempotent.
//
// Purpose: give the Maestro flows deterministic codes to type into the scan
// injector — two locations, one consumable, one tool with a serial, and one
// toolbox kit whose stencil resolves the kit. Re-runnable: it reuses rows by
// code, returns any open tool checkout, and tops consumables back up, so a
// rerun starts from the same state.
//
// Usage:
//   node native/e2e/seed-fieldops.mjs                 # reads e2e/.e2e-stack.env
//   node native/e2e/seed-fieldops.mjs http://host:port
//
// Non-purpose: no app code, no schema changes — every call is a public API
// route the app itself uses.

import { randomUUID } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

const here = path.dirname(new URL(import.meta.url).pathname);
const repoRoot = path.resolve(here, "../..");

const EMAIL = process.env.MARULA_E2E_EMAIL ?? "owner@demo.local";
const PASSWORD = process.env.MARULA_E2E_PASSWORD ?? "demo-password-123";

const FIXTURES = {
  van: { name: "Van 7", code: "VAN-7", kind: "van" },
  shelf: { name: "Shelf B-2", code: "B-2", kind: "shelf" },
  consumable: {
    name: "Cable ties (pack of 100)",
    sku: "TIE-100",
    partNumber: "PN-TIE-100",
    unit: "box",
    kind: "consumable",
    targetMilli: 10_000,
  },
  tool: {
    name: "Cable tester",
    sku: "TESTER-1",
    unit: "each",
    kind: "serialized",
    isTool: true,
    serial: "CT-0001",
  },
  kit: { name: "Field network kit", code: "KIT-FIELD-1" },
};

function apiBase() {
  const arg = process.argv[2];
  if (arg !== undefined) return arg.replace(/\/$/, "");
  if (process.env.MARULA_API_URL) return process.env.MARULA_API_URL.replace(/\/$/, "");
  const envPath = path.join(repoRoot, "e2e/.e2e-stack.env");
  if (existsSync(envPath)) {
    const env = Object.fromEntries(
      readFileSync(envPath, "utf8")
        .split("\n")
        .filter((line) => line.includes("="))
        .map((line) => line.split("=").map((part) => part.trim())),
    );
    if (env.E2E_API_URL) return env.E2E_API_URL.replace(/\/$/, "");
    if (env.E2E_API_PORT) return `http://localhost:${env.E2E_API_PORT}`;
  }
  return "http://localhost:8080";
}

const base = apiBase();
let token = null;

async function api(method, route, body) {
  const response = await fetch(`${base}/api/v1${route}`, {
    method,
    headers: {
      "content-type": "application/json",
      ...(token !== null ? { authorization: `Bearer ${token}` } : {}),
    },
    ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
  });
  const text = await response.text();
  const data = text === "" ? null : JSON.parse(text);
  if (!response.ok) {
    throw new Error(`${method} ${route} -> ${response.status}: ${text.slice(0, 300)}`);
  }
  return data;
}

const key = () => randomUUID();

async function login() {
  const result = await api("POST", "/auth/login", { email: EMAIL, password: PASSWORD });
  if (result?.access_token === undefined) {
    throw new Error(`login did not return tokens: ${JSON.stringify(result).slice(0, 200)}`);
  }
  token = result.access_token;
}

async function ensureLocation(fixture) {
  const locations = await api("GET", "/stock-locations?active_only=true");
  const existing = locations.find((row) => row.code === fixture.code);
  if (existing !== undefined) return existing;
  return api("POST", "/stock-locations", {
    name: fixture.name,
    code: fixture.code,
    kind: fixture.kind,
    idempotency_key: key(),
  });
}

async function findItemBySku(sku) {
  const items = await api("GET", `/stock-items?search=${encodeURIComponent(sku)}`);
  return items.find((row) => row.sku === sku);
}

async function ensureItem(fixture) {
  const existing = await findItemBySku(fixture.sku);
  if (existing !== undefined) return existing;
  return api("POST", "/stock-items", {
    name: fixture.name,
    kind: fixture.kind,
    sku: fixture.sku,
    currency: "USD",
    unit: fixture.unit,
    ...(fixture.partNumber !== undefined ? { part_number: fixture.partNumber } : {}),
    ...(fixture.isTool === true ? { is_tool: true } : {}),
    idempotency_key: key(),
  });
}

async function ensureKit() {
  const kits = await api("GET", "/tool-kits?active=true");
  const existing = kits.find((row) => row.code === FIXTURES.kit.code);
  if (existing !== undefined) return existing;
  const created = await api("POST", "/tool-kits", {
    name: FIXTURES.kit.name,
    code: FIXTURES.kit.code,
    idempotency_key: key(),
  });
  return created.kit ?? created;
}

async function ensureKitMember(kitId, itemId) {
  const detail = await api("GET", `/tool-kits/${kitId}`);
  const hasMember = (detail.members ?? []).some((member) => member.stock_item_id === itemId);
  if (hasMember) return;
  await api("POST", `/tool-kits/${kitId}/members`, {
    stock_item_id: itemId,
    qty_milli: 1000,
    idempotency_key: key(),
  });
}

async function returnOpenCheckouts() {
  const checkouts = await api("GET", "/tool-checkouts");
  for (const row of checkouts) {
    if (row.checkout?.status === "returned") continue;
    if (row.checkout?.id === undefined) continue;
    await api("POST", `/tool-checkouts/${row.checkout.id}/checkin`, { idempotency_key: key() });
  }
}

/** A running timer blocks the timer flows, so every seed starts clean. */
async function stopActiveTimer() {
  const active = await api("GET", "/timers/active");
  if (active === null || active?.id === undefined || active.stopped_at != null) return;
  await api("POST", "/timers/stop", { idempotency_key: key() });
}

async function topUpConsumable(itemId, locationId) {
  const onHand = await api("GET", "/stock/on-hand");
  const row = onHand.find(
    (candidate) => candidate.stock_item_id === itemId && candidate.stock_location_id === locationId,
  );
  const current = row?.qty_milli ?? 0;
  const missing = FIXTURES.consumable.targetMilli - current;
  if (missing <= 0) return current;
  await api("POST", "/stock/receive", {
    item_id: itemId,
    location_id: locationId,
    qty_milli: missing,
    note: "E2E fixture top-up",
    idempotency_key: key(),
  });
  return current + missing;
}

async function ensureToolSerials(itemId, locationId, serial) {
  const serials = await api("GET", `/stock-items/${itemId}/serials`);
  const existing = serials.find((row) => row.serial_number === serial);
  if (existing !== undefined) return existing;
  const received = await api("POST", "/stock/receive", {
    item_id: itemId,
    location_id: locationId,
    qty_milli: 1000,
    serial_numbers: [serial],
    note: "E2E fixture receive",
    idempotency_key: key(),
  });
  return received;
}


/** A day-pack line the phone can tick: one appointment today + one pick for
 * the demo's own ticket. Idempotent — an existing pick for the same item on
 * that ticket is left alone, and a taken slot is tolerated. */
async function ensureDayPackFixture(itemId) {
  const myWork = await api("GET", "/tickets/my-work");
  const ticket = myWork[0];
  if (ticket === undefined || ticket.id === undefined) return null;
  const picks = await api("GET", `/day-pack/picks?ticket_id=${ticket.id}`);
  const existing = picks.find((row) => row.pick?.stock_item_id === itemId);
  // Clean any tick a previous run left so the flow always starts at zero.
  const pack = await api("GET", "/day-pack");
  const localDay = (() => {
    const now = new Date();
    const month = `${now.getMonth() + 1}`.padStart(2, "0");
    const day = `${now.getDate()}`.padStart(2, "0");
    return `${now.getFullYear()}-${month}-${day}`;
  })();
  for (const trip of pack.trips ?? []) {
    for (const line of trip.lines ?? []) {
      if (line.tick != null && line.tick.line_key !== undefined) {
        await api(
          "DELETE",
          `/day-pack/ticks?date=${localDay}&line_key=${encodeURIComponent(line.tick.line_key)}`,
        );
      }
    }
  }
  if (existing === undefined) {
    await api("POST", "/day-pack/picks", {
      ticket_id: ticket.id,
      stock_item_id: itemId,
      qty_milli: 2000,
      note: "E2E mobile pack",
      idempotency_key: key(),
    });
  }
  // A late local slot keeps the fixture away from the demo's own stops.
  const now = new Date();
  const startsAt = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 21, 0, 0);
  const endsAt = new Date(startsAt.getTime() + 60 * 60 * 1000);
  try {
    await api("POST", "/appointments", {
      title: "E2E mobile pack stop",
      ticket_id: ticket.id,
      kind: "on_site",
      starts_at: startsAt.toISOString(),
      ends_at: endsAt.toISOString(),
      idempotency_key: key(),
    });
  } catch (error) {
    // 409 means a previous run already holds the slot; the pack still reads.
    if (!String(error.message).includes("409")) throw error;
  }
  return ticket.id;
}

async function main() {
  console.log(`[seed-fieldops] API ${base}`);
  await login();
  const van = await ensureLocation(FIXTURES.van);
  const shelf = await ensureLocation(FIXTURES.shelf);
  const consumable = await ensureItem(FIXTURES.consumable);
  const tool = await ensureItem(FIXTURES.tool);
  await ensureToolSerials(tool.id, van.id, FIXTURES.tool.serial);
  const onHand = await topUpConsumable(consumable.id, van.id);
  const kit = await ensureKit();
  await ensureKitMember(kit.id, tool.id);
  await returnOpenCheckouts();
  await stopActiveTimer();
  const packTicketId = await ensureDayPackFixture(consumable.id);
  console.log(
    JSON.stringify(
      {
        van: FIXTURES.van.code,
        shelf: FIXTURES.shelf.code,
        shelfId: shelf.id,
        consumable: FIXTURES.consumable.sku,
        tool: FIXTURES.tool.sku,
        toolSerial: FIXTURES.tool.serial,
        kit: FIXTURES.kit.code,
        consumableOnHandMilli: onHand,
        packTicketId,
      },
      null,
      2,
    ),
  );
}

main().catch((error) => {
  console.error(`[seed-fieldops] failed: ${error.message}`);
  process.exit(1);
});
