import assert from "node:assert/strict";
import { test } from "node:test";

import { loadConfig, noneMode, type Config } from "./config.js";
import { DeliveryRefusal, forwardTicketEvent } from "./delivery.js";
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

function ticketEvent(payload: Record<string, unknown>): Buffer {
  return Buffer.from(JSON.stringify({ type: "ticket.updated", payload }));
}

const TICKET = {
  id: "t-1001",
  number: "T-1001",
  title: "Printer offline",
};

const DEVICE_ID = "42";

function payloadFor(overrides: Record<string, unknown> = {}) {
  return {
    ticket_id: TICKET.id,
    ticket_number: TICKET.number,
    ticket_title: TICKET.title,
    device_id: DEVICE_ID,
    ...overrides,
  };
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

test("buildActivity names the ticket when the event carries number and title", () => {
  assert.deepEqual(buildActivity({ type: "ticket.created", ticket: TICKET }), {
    activityType: "MARULA_EVENT",
    activityContent:
      "Marula event ticket.created for [T-1001] Printer offline",
  });
});

test("buildActivity degrades to the fields the event actually carries", () => {
  // No title: the number still identifies the ticket.
  assert.equal(
    buildActivity({
      type: "ticket.updated",
      ticket: { id: "t-1001", number: "T-1001" },
    }).activityContent,
    "Marula event ticket.updated for [T-1001]",
  );
  // No number: the title carries its id so the note stays traceable.
  assert.equal(
    buildActivity({
      type: "ticket.updated",
      ticket: { id: "t-1001", title: "Printer offline" },
    }).activityContent,
    "Marula event ticket.updated for Printer offline (t-1001)",
  );
  // Neither: the id alone. A real identifier, never a placeholder.
  assert.equal(
    buildActivity({ type: "ticket.updated", ticket: { id: "t-1001" } })
      .activityContent,
    "Marula event ticket.updated for t-1001",
  );
});

test("a delivery posts the note against the device the payload names", async () => {
  const stub = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    const result = await forwardTicketEvent(CONFIGURED, ticketEvent(payloadFor()));
    assert.equal(result, "delivered");
    const body = JSON.parse(String(stub.calls[0]?.init?.body)) as {
      deviceId: string;
      activityType: string;
      activityContent: string;
    };
    assert.deepEqual(body, {
      deviceId: "42",
      activityType: "MARULA_EVENT",
      activityContent:
        "Marula event ticket.updated for [T-1001] Printer offline",
    });
    assert.equal(
      stub.calls[0]?.url,
      "https://app.ninjarmm.com/api/v2/device/activities",
    );
  } finally {
    stub.restore();
  }
});

test("a delivery with no device link is refused, and nothing is posted", async () => {
  // There is no ticket-to-device mapping in the public payload, so the honest
  // answer is a refusal naming the field. Posting to a guessed device is worse.
  const stub = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    const payload = payloadFor();
    delete (payload as Record<string, unknown>)["device_id"];
    await assert.rejects(
      forwardTicketEvent(CONFIGURED, ticketEvent(payload)),
      (error: unknown) => {
        assert.ok(error instanceof DeliveryRefusal);
        assert.equal(error.field, "payload.device_id");
        return true;
      },
    );
    assert.equal(stub.calls.length, 0);
  } finally {
    stub.restore();
  }
});

test("a delivery with no ticket id is refused by field", async () => {
  const stub = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    const payload = payloadFor();
    delete (payload as Record<string, unknown>)["ticket_id"];
    await assert.rejects(
      forwardTicketEvent(CONFIGURED, ticketEvent(payload)),
      (error: unknown) => {
        assert.ok(error instanceof DeliveryRefusal);
        assert.equal(error.field, "payload.ticket_id");
        return true;
      },
    );
    assert.equal(stub.calls.length, 0);
  } finally {
    stub.restore();
  }
});

test("ticket number and title are optional, and their absence invents nothing", async () => {
  const stub = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    const result = await forwardTicketEvent(
      CONFIGURED,
      ticketEvent({ ticket_id: "t-1001", device_id: DEVICE_ID }),
    );
    assert.equal(result, "delivered");
    const body = JSON.parse(String(stub.calls[0]?.init?.body)) as {
      activityContent: string;
    };
    assert.equal(
      body.activityContent,
      "Marula event ticket.updated for t-1001",
    );
  } finally {
    stub.restore();
  }
});

test("a body that is not JSON is refused as the body", async () => {
  await assert.rejects(
    forwardTicketEvent(CONFIGURED, Buffer.from("not json")),
    (error: unknown) => {
      assert.ok(error instanceof DeliveryRefusal);
      assert.equal(error.field, "body");
      return true;
    },
  );
});

test("an event this connector does not act on is ignored", async () => {
  const result = await forwardTicketEvent(
    CONFIGURED,
    Buffer.from(
      JSON.stringify({ type: "client.updated", payload: { id: "c1" } }),
    ),
  );
  assert.equal(result, "ignored");
});

test("a failed push is not swallowed", async () => {
  const stub = stubFetch(() => new Response("nope", { status: 500 }));
  try {
    await assert.rejects(
      forwardTicketEvent(CONFIGURED, ticketEvent(payloadFor())),
      /NinjaOne push failed \(500\)/,
    );
  } finally {
    stub.restore();
  }
});

test("pushActivity in none mode dials nothing and reports no external call", async () => {
  const stub = stubFetch(() => new Response(null, { status: 200 }));
  try {
    const { result, logged } = await quietConsole(async () => {
      return await pushActivity(
        { ...CONFIGURED, ninjaOneToken: undefined },
        "42",
        buildActivity({ type: "ticket.created", ticket: TICKET }),
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
    const activity = buildActivity({ type: "ticket.created", ticket: TICKET });
    assert.equal(await pushActivity(CONFIGURED, DEVICE_ID, activity), true);
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
      deviceId: DEVICE_ID,
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
      pushActivity(
        CONFIGURED,
        DEVICE_ID,
        buildActivity({ type: "ticket.created", ticket: TICKET }),
      ),
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
      pushActivity(
        CONFIGURED,
        DEVICE_ID,
        buildActivity({ type: "ticket.created", ticket: TICKET }),
      ),
      /connection refused/,
    );
  } finally {
    globalThis.fetch = original;
  }
});
