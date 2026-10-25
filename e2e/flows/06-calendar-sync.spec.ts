// Flow 6 — Calendar sync (the lane-7 close-out surface): connection health,
// one-time feed URL, sync-now with outcome counters.
//
// The sync-now leg runs against a REAL local iCal feed served by an in-test
// HTTP server — the provider seam's first-party iCal path parses and counts
// events, so the outcome counters are deterministic, never mocked.
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";
import http from "node:http";

interface Connection {
  id: string;
  name: string;
  provider: string;
  direction: string;
  status: string;
}

const ICS_BODY = [
  "BEGIN:VCALENDAR",
  "VERSION:2.0",
  "PRODID:-//Marula E2E//EN",
  "BEGIN:VEVENT",
  "UID:e2e-busy-1@marula.test",
  "DTSTAMP:20260915T090000Z",
  "DTSTART:20260916T100000Z",
  "DTEND:20260916T110000Z",
  "SUMMARY:E2E busy block",
  "END:VEVENT",
  "END:VCALENDAR",
].join("\r\n");

/** Per-worker ephemeral iCal feed (random port — no cross-worker clashes). */
let icsPort = 0;

test.beforeAll(async () => {
  icsPort = await new Promise<number>((resolve, reject) => {
    const server = http
      .createServer((_req, res) => {
        res.writeHead(200, { "content-type": "text/calendar" });
        res.end(ICS_BODY);
      })
      .listen(0, "127.0.0.1", () => {
        const addr = server.address();
        resolve(typeof addr === "object" && addr ? addr.port : 0);
      });
    server.on("error", reject);
  });
});

test.afterAll(async () => {
  // The ephemeral server dies with the worker process; nothing to do.
});

// The three tests share one owner's calendar connections — run in order.
test.describe.configure({ mode: "serial" });

async function cleanConnections(token: string) {
  const get = authedFetch(token);
  let existing: Connection[] = [];
  try {
    existing = await get<Connection[]>("/calendar-sync");
  } catch {
    return; // listing raced with a sibling worker's cleanup — nothing to do
  }
  for (const c of existing) {
    // 404s are fine: a sibling worker's cleanup may have beaten us to it.
    await authedFetch(token)(`/calendar-sync/${c.id}`, { method: "DELETE" }).catch(() => {});
  }
}

test("panel shows the connect affordance when no connections exist", async ({ authedPage, ownerTokens }) => {
  await cleanConnections(ownerTokens.access_token);

  await authedPage.goto("/calendar");
  await expect(authedPage.getByTestId("connect-calendar")).toBeVisible({ timeout: 30_000 });
});

test("connect an outbound feed, see connection health + one-time feed URL", async ({
  authedPage,
  ownerTokens,
}) => {
  await cleanConnections(ownerTokens.access_token);

  await authedPage.goto("/calendar");
  await authedPage.getByTestId("connect-calendar").click();
  await expect(authedPage.getByTestId("connect-form")).toBeVisible({ timeout: 15_000 });
  await authedPage
    .locator('[data-testid="connect-form"] input')
    .first()
    .fill(`E2E outbound ${Date.now() % 100000}`);
  // Outbound needs no external feed URL; inbound/two-way validate one.
  await authedPage.getByTestId("connect-direction").selectOption("outbound");
  await authedPage.getByTestId("connect-save").click();

  const conn = authedPage.getByTestId(new RegExp("sync-connection-")).first();
  await expect(conn).toBeVisible({ timeout: 20_000 });
  // One-time feed URL is shown once at connect (invite-token doctrine).
  await expect(authedPage.getByTestId("one-time-feed-url")).toBeVisible();
});

// Blocked by the egress private-address screen (Wave 55). The feed below is a
// loopback server, and the guard refuses a private address before a socket
// exists, which is the guard working as designed. Running this offline needs a
// dev-gated transport allowance, not a relaxed screen; until that is ruled on,
// the case is `fixme` so the gap shows in the report instead of vanishing like
// a `skip` would.
test.fixme("sync-now against a real local iCal feed reports outcome counters", async ({
  authedPage,
  ownerTokens,
}) => {
  await cleanConnections(ownerTokens.access_token);

  await authedPage.goto("/calendar");
  await authedPage.getByTestId("connect-calendar").click();
  await expect(authedPage.getByTestId("connect-form")).toBeVisible({ timeout: 15_000 });
  await authedPage
    .locator('[data-testid="connect-form"] input')
    .first()
    .fill(`E2E inbound ${Date.now() % 100000}`);
  await authedPage.getByTestId("connect-direction").selectOption("inbound");
  await authedPage
    .getByPlaceholder("https://calendar.example.com/feed.ics")
    .fill(`http://127.0.0.1:${icsPort}/feed.ics`);
  await authedPage.getByTestId("connect-save").click();

  const conn = authedPage.getByTestId(new RegExp("sync-connection-")).first();
  await expect(conn).toBeVisible({ timeout: 20_000 });
  const id = (await conn.getAttribute("data-testid"))?.replace("sync-connection-", "");
  await authedPage.getByTestId(`sync-now-${id}`).click();
  await expect(authedPage.getByTestId(`sync-outcome-${id}`)).toBeVisible({ timeout: 30_000 });
});
