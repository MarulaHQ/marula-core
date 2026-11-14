import assert from "node:assert/strict";
import { test } from "node:test";

import { loadConfig, noneMode, type Config } from "./config.js";
import { buildActivity, pushActivity } from "./ninjaone.js";

const CONFIGURED: Config = {
  marulaBaseUrl: "https://marula.test",
  marulaApiKey: "mrl_test_key",
  marulaWebhookSecret: "whsec_test",
  port: 8788,
  ninjaOneBaseUrl: "https://app.ninjarmm.com",
  ninjaOneToken: "ninja_test_token",
};

interface FetchCall {
  url: string;
  init: RequestInit | undefined;
}

function stubFetch(response: () => Response): {
  calls: FetchCall[];
  restore: () => void;
} {
  const calls: FetchCall[] = [];
  const original = globalThis.fetch;
  globalThis.fetch = (async (
    input: string | URL | Request,
    init?: RequestInit,
  ) => {
    calls.push({ url: String(input), init });
    return response();
  }) as typeof globalThis.fetch;
  return {
    calls,
    restore: () => {
      globalThis.fetch = original;
    },
  };
}

function quietConsole<T>(run: () => T): { result: T; logged: string[] } {
  const logged: string[] = [];
  const original = console.log;
  console.log = (...args: unknown[]) => {
    logged.push(args.map(String).join(" "));
  };
  try {
    return { result: run(), logged };
  } finally {
    console.log = original;
  }
}

test("none mode is on until both NinjaOne settings are present", () => {
  assert.equal(noneMode(CONFIGURED), false);
  assert.equal(noneMode({ ...CONFIGURED, ninjaOneBaseUrl: undefined }), true);
  assert.equal(noneMode({ ...CONFIGURED, ninjaOneToken: undefined }), true);
  assert.equal(noneMode({ ...CONFIGURED, ninjaOneToken: "" }), true);
});

test("loadConfig refuses to start without a Marula key", () => {
  assert.throws(
    () => loadConfig({}),
    /MARULA_API_KEY is required/,
    "a connector with no Marula credential must fail loudly, not run blind",
  );
});

test("loadConfig defaults and NinjaOne credentials", () => {
  const config = loadConfig({ MARULA_API_KEY: "mrl_test_key" });
  assert.equal(config.marulaBaseUrl, "http://localhost:8080");
  assert.equal(config.port, 8788);
  assert.equal(config.marulaWebhookSecret, "");
  assert.equal(config.ninjaOneBaseUrl, undefined);
  assert.equal(noneMode(config), true);

  const configured = loadConfig({
    MARULA_API_KEY: "mrl_test_key",
    MARULA_BASE_URL: "https://marula.test",
    MARULA_WEBHOOK_SECRET: "whsec_test",
    PORT: "9999",
    NINJAONE_BASE_URL: "https://app.ninjarmm.com",
    NINJAONE_TOKEN: "ninja_test_token",
  });
  assert.deepEqual(configured, { ...CONFIGURED, port: 9999 });
  assert.equal(noneMode(configured), false);
});

test("buildActivity names the ticket when the event carries one", () => {
  assert.deepEqual(
    buildActivity({
      type: "ticket.created",
      ticket: { id: "t-1001", number: "T-1001", title: "Printer offline" },
    }),
    {
      activityType: "MARULA_EVENT",
      activityContent:
        "Marula event ticket.created for [T-1001] Printer offline",
    },
  );
});

test("buildActivity still records the event when the ticket is missing", () => {
  // The event type is the only fact the receiver is guaranteed: the payload's
  // ticket_id is optional, and the connector must not drop the delivery.
  assert.deepEqual(buildActivity({ type: "ticket.updated" }), {
    activityType: "MARULA_EVENT",
    activityContent:
      "Marula event ticket.updated for (ticket details not included)",
  });
});

test("pushActivity in none mode dials nothing and reports no external call", async () => {
  const stub = stubFetch(() => new Response(null, { status: 200 }));
  try {
    const { result, logged } = await quietConsole(async () => {
      return await pushActivity(
        { ...CONFIGURED, ninjaOneToken: undefined },
        "42",
        buildActivity({ type: "ticket.created" }),
      );
    });
    assert.equal(await result, false);
    assert.equal(stub.calls.length, 0, "none mode must not reach the network");
    // None mode is observable, not silent: the payload is printed instead.
    assert.match(logged.join("\n"), /would POST activity to NinjaOne device 42/);
  } finally {
    stub.restore();
  }
});

test("pushActivity posts the note with the bearer token", async () => {
  const stub = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    const activity = buildActivity({
      type: "ticket.created",
      ticket: { id: "t-1001", number: "T-1001", title: "Printer offline" },
    });
    assert.equal(await pushActivity(CONFIGURED, "42", activity), true);
    assert.equal(stub.calls.length, 1);
    const call = stub.calls[0];
    assert.equal(
      call?.url,
      "https://app.ninjarmm.com/api/v2/device/activities",
    );
    assert.equal(call?.init?.method, "POST");
    const headers = new Headers(call?.init?.headers);
    assert.equal(headers.get("authorization"), "Bearer ninja_test_token");
    assert.equal(headers.get("content-type"), "application/json");
    assert.deepEqual(JSON.parse(String(call?.init?.body)), {
      deviceId: "42",
      ...activity,
    });
  } finally {
    stub.restore();
  }
});

test("pushActivity throws on a refused push", async () => {
  const stub = stubFetch(() => new Response("nope", { status: 401 }));
  try {
    await assert.rejects(
      pushActivity(CONFIGURED, "42", buildActivity({ type: "ticket.created" })),
      /NinjaOne push failed \(401\)/,
    );
  } finally {
    stub.restore();
  }
});

test("pushActivity propagates a transport failure", async () => {
  const original = globalThis.fetch;
  globalThis.fetch = (async () => {
    throw new Error("connection refused");
  }) as typeof globalThis.fetch;
  try {
    await assert.rejects(
      pushActivity(CONFIGURED, "42", buildActivity({ type: "ticket.created" })),
      /connection refused/,
    );
  } finally {
    globalThis.fetch = original;
  }
});
