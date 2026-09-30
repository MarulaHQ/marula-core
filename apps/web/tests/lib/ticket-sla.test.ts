// Unit tests for the shared SLA clock reader.
//
// The behaviour worth pinning is not "it fetches" — it is that a second caller
// does NOT fetch, that concurrent callers for the same ticket share one read,
// and that a page of rows is never turned into fifty simultaneous requests.
// Those three are what keep the ledger from hammering our own API, and none of
// them is visible in a rendered page.
//
// `MockApi` cannot measure this: it invokes route handlers synchronously and
// does not await them, so an async handler never delays a response and the
// "concurrency" it reports is always the full fan-out. These tests therefore
// stub `fetch` directly and hold each response open long enough to observe how
// many are open at once.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { clearTicketSlaCache, invalidateTicketSla, readTicketSla } from "../../lib/ticket-sla";
import { ME_FIXTURE, seedAuthTokens } from "../helpers/mock-api";
import { setFetchRouter } from "../helpers/fetch-router";

const ids = Array.from(
  { length: 12 },
  (_, i) => `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
);

/** Long enough that any overlapping requests are unambiguously simultaneous. */
const HOLD_MS = 20;

interface FetchSpy {
  paths: string[];
  peak: () => number;
  restore: () => void;
}

/**
 * openapi-fetch hands the router a bare string for some verbs and a constructed
 * `Request` for GET, so the URL has to be unwrapped either way. Stringifying a
 * Request yields "[object Request]" and a URL parse of that throws.
 */
function pathOf(input: RequestInfo | URL): string {
  if (typeof input === "string") return new URL(input).pathname;
  if (input instanceof URL) return input.pathname;
  return new URL(input.url).pathname;
}

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    // openapi-fetch decides whether to parse a body by its content type, so a
    // stub without one yields `data: undefined` and looks like an empty 200.
    headers: new Headers({ "content-type": "application/json" }),
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as unknown as Response;
}

const clockBody = (id: string): unknown => ({
  applied: true,
  breached: false,
  minutes_remaining: 42,
  paused: false,
  state: {
    ticket_id: id,
    policy_id: "00000000-0000-4000-8000-0000000000ff",
    paused_business_secs: 0,
    paused_since: null,
    breached: false,
    first_response_due: "2026-01-01T00:00:00Z",
    first_responded_at: null,
    resolution_due: "2026-01-01T04:00:00Z",
    resolved_at: null,
  },
});

/**
 * Every clock read resolves after `HOLD_MS`, recording the running overlap.
 *
 * Routed through `setFetchRouter` rather than assigned to `globalThis.fetch`:
 * the SDK client captured the router at import time, so replacing the global
 * later is silently ignored.
 */
function installSlaFetch(): FetchSpy {
  const open = new Set<string>();
  const paths: string[] = [];
  let peak = 0;

  setFetchRouter(async (input) => {
    const path = pathOf(input);
    paths.push(path);
    open.add(path);
    peak = Math.max(peak, open.size);
    await new Promise((resolve) => setTimeout(resolve, HOLD_MS));
    open.delete(path);
    const id = path.split("/")[4] ?? "";
    return jsonResponse(clockBody(id));
  });

  return {
    paths,
    peak: () => peak,
    restore: () => setFetchRouter(null),
  };
}

function installForbiddenFetch(): FetchSpy {
  const paths: string[] = [];
  setFetchRouter(async (input) => {
    paths.push(pathOf(input));
    return jsonResponse({ error: { message: "forbidden" } }, 403);
  });
  return { paths, peak: () => 0, restore: () => setFetchRouter(null) };
}

const readsFor = (spy: FetchSpy, id: string): number =>
  spy.paths.filter((p) => p === `/api/v1/tickets/${id}/sla`).length;

let spy: FetchSpy;

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  clearTicketSlaCache();
  spy = installSlaFetch();
});

afterEach(() => {
  spy.restore();
});

describe("shared SLA clock reader", () => {
  it("reads a ticket's clock once and serves later callers from cache", async () => {
    const first = await readTicketSla(ids[0] as string);
    const second = await readTicketSla(ids[0] as string);

    expect(first?.applied).toBe(true);
    expect(second?.applied).toBe(true);
    expect(readsFor(spy, ids[0] as string)).toBe(1);
  });

  it("shares one read between callers that arrive while it is in flight", async () => {
    const all = await Promise.all([
      readTicketSla(ids[1] as string),
      readTicketSla(ids[1] as string),
      readTicketSla(ids[1] as string),
    ]);

    expect(all.every((v) => v?.applied === true)).toBe(true);
    // One waiter exists per id, not per caller: the later two joined the
    // promise the first one created.
    expect(readsFor(spy, ids[1] as string)).toBe(1);
  });

  it("caps how many clock reads are open at once", async () => {
    await Promise.all(ids.map((id) => readTicketSla(id)));

    // Twelve tickets against a cap of six: the overlap must never exceed the
    // cap, and every ticket must still have been read exactly once.
    expect(spy.peak()).toBeGreaterThan(1);
    expect(spy.peak()).toBeLessThanOrEqual(6);
    for (const id of ids) expect(readsFor(spy, id)).toBe(1);
  });

  it("caches a read the caller may not make, so it is not retried forever", async () => {
    spy.restore();
    spy = installForbiddenFetch();

    expect(await readTicketSla(ids[2] as string)).toBeNull();
    expect(await readTicketSla(ids[2] as string)).toBeNull();
    expect(readsFor(spy, ids[2] as string)).toBe(1);
  });

  it("re-reads a ticket after its clock is invalidated", async () => {
    await readTicketSla(ids[3] as string);
    invalidateTicketSla(ids[3] as string);
    await readTicketSla(ids[3] as string);

    expect(readsFor(spy, ids[3] as string)).toBe(2);
  });

  it("does not need the viewer to be the ticket's assignee", async () => {
    // A guard against a future "optimisation" that scopes the read to me: the
    // ledger shows every reachable ticket, not just my own work.
    expect(ME_FIXTURE.user_id).toBeTruthy();
    const value = await readTicketSla(ids[4] as string);
    expect(value).not.toBeNull();
  });
});
