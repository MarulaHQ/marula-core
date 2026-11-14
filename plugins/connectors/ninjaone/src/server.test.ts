import { createHmac } from "node:crypto";
import { createServer, request as httpRequest, type Server } from "node:http";
import assert from "node:assert/strict";
import { test } from "node:test";

import type { Config } from "./config.js";
import { forwardTicketEvent } from "./delivery.js";
import {
  MAX_WEBHOOK_BYTES,
  WEBHOOK_PATH,
  createWebhookHandler,
} from "./server.js";

const SECRET = "whsec_test_not_a_real_secret";

const CONFIGURED: Config = {
  marulaBaseUrl: "https://marula.test",
  marulaApiKey: "mrl_test_key",
  marulaWebhookSecret: SECRET,
  port: 8788,
  ninjaOneBaseUrl: "https://app.ninjarmm.com",
  ninjaOneToken: "ninja_test_token",
};

const TICKET_EVENT = Buffer.from(
  JSON.stringify({
    type: "ticket.updated",
    payload: {
      ticket_id: "t-1001",
      ticket_number: "T-1001",
      ticket_title: "Printer offline",
      device_id: "42",
    },
  }),
);

/** The receiver checks the replay window against the real clock. */
function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}

function signatureFor(
  rawBody: Buffer,
  secret: string = SECRET,
  timestamp: number = nowUnix(),
): string {
  const hex = createHmac("sha256", secret)
    .update(
      Buffer.concat([
        Buffer.from(String(timestamp)),
        Buffer.from("."),
        rawBody,
      ]),
    )
    .digest("hex");
  return `t=${timestamp},v1=${hex}`;
}

interface Harness {
  url: string;
  forwarded: Buffer[];
  close: () => Promise<void>;
}

async function startHarness(): Promise<Harness> {
  const forwarded: Buffer[] = [];
  const handler = createWebhookHandler({
    secret: SECRET,
    forward: async (rawBody) => {
      forwarded.push(rawBody);
      await forwardTicketEvent(CONFIGURED, rawBody);
    },
    logger: { warn: () => undefined },
  });
  const server: Server = createServer(handler);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("expected a TCP address");
  }
  return {
    url: `http://127.0.0.1:${address.port}${WEBHOOK_PATH}`,
    forwarded,
    close: () =>
      new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      ),
  };
}

async function post(
  url: string,
  body: Buffer,
  options: { signature?: string | null } = {},
): Promise<{ status: number; json: () => Promise<Record<string, unknown>> }> {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    "content-length": String(body.length),
  };
  if (options.signature !== null) {
    headers["x-marula-signature"] = options.signature ?? signatureFor(body);
  }
  return await send(url, "POST", headers, body);
}

/** A client that bypasses `globalThis.fetch`, so a stubbed vendor cannot see it. */
async function send(
  url: string,
  method: string,
  headers: Record<string, string>,
  body?: Buffer,
): Promise<{ status: number; json: () => Promise<Record<string, unknown>> }> {
  const target = new URL(url);
  return await new Promise((resolve, reject) => {
    const request = httpRequest(
      {
        hostname: target.hostname,
        port: target.port,
        path: `${target.pathname}${target.search}`,
        method,
        headers,
      },
      (response) => {
        const chunks: Buffer[] = [];
        response.on("data", (chunk: Buffer) => chunks.push(chunk));
        response.on("end", () => {
          resolve({
            status: response.statusCode ?? 0,
            json: async () =>
              JSON.parse(Buffer.concat(chunks).toString("utf8")) as Record<
                string,
                unknown
              >,
          });
        });
      },
    );
    request.on("error", reject);
    request.end(body);
  });
}

async function withHarness(run: (harness: Harness) => Promise<void>) {
  const harness = await startHarness();
  try {
    await run(harness);
  } finally {
    await harness.close();
  }
}

function stubFetch(response: () => Response | Promise<Response>): {
  calls: number;
  restore: () => void;
} {
  const stub = {
    calls: 0,
    restore: () => {
      globalThis.fetch = original;
    },
  };
  const original = globalThis.fetch;
  globalThis.fetch = (async () => {
    stub.calls += 1;
    return await response();
  }) as typeof globalThis.fetch;
  return stub;
}

test("answers 200 only after the push settles", async () => {
  const vendor = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    await withHarness(async (harness) => {
      const response = await post(harness.url, TICKET_EVENT);
      assert.equal(response.status, 200);
      assert.deepEqual(await response.json(), { ok: true });
      assert.equal(vendor.calls, 1, "the vendor call must have happened");
      assert.equal(harness.forwarded.length, 1);
    });
  } finally {
    vendor.restore();
  }
});

test("answers 502 when the forward fails, and a retry meets the same answer", async () => {
  const vendor = stubFetch(() => new Response("nope", { status: 500 }));
  try {
    await withHarness(async (harness) => {
      const first = await post(harness.url, TICKET_EVENT);
      const second = await post(harness.url, TICKET_EVENT);
      assert.equal(first.status, 502);
      assert.equal(second.status, 502);
      assert.deepEqual(await first.json(), {
        error: "forward failed",
        detail: "NinjaOne push failed (500)",
      });
      // A 5xx is the only retry this design has, so the sender's second attempt
      // must reach the vendor again rather than being swallowed as a no-op.
      assert.equal(vendor.calls, 2);
    });
  } finally {
    vendor.restore();
  }
});

test("a repeated delivery produces the same answer and one push each", async () => {
  const vendor = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    await withHarness(async (harness) => {
      const first = await post(harness.url, TICKET_EVENT);
      const second = await post(harness.url, TICKET_EVENT);
      assert.equal(first.status, 200);
      assert.equal(second.status, 200);
      assert.equal(vendor.calls, 2);
      const [a, b] = harness.forwarded;
      assert.ok(a?.equals(TICKET_EVENT));
      assert.ok(b?.equals(TICKET_EVENT));
    });
  } finally {
    vendor.restore();
  }
});

test("answers 401 for an unsigned delivery and never forwards it", async () => {
  const vendor = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    await withHarness(async (harness) => {
      const response = await post(harness.url, TICKET_EVENT, {
        signature: null,
      });
      assert.equal(response.status, 401);
      assert.equal((await response.json()).reason, "malformed-header");
      assert.equal(harness.forwarded.length, 0);
      assert.equal(vendor.calls, 0);
    });
  } finally {
    vendor.restore();
  }
});

test("answers 401 for a signature over different bytes", async () => {
  const vendor = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    await withHarness(async (harness) => {
      const response = await post(harness.url, TICKET_EVENT, {
        signature: signatureFor(Buffer.from("another body")),
      });
      assert.equal(response.status, 401);
      assert.equal((await response.json()).reason, "bad-signature");
      assert.equal(harness.forwarded.length, 0);
    });
  } finally {
    vendor.restore();
  }
});

test("answers 404 on every path but the documented one", async () => {
  const vendor = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    await withHarness(async (harness) => {
      const base = harness.url.slice(0, -WEBHOOK_PATH.length);
      for (const path of ["/", "/marula/hook/extra", `/${WEBHOOK_PATH}x`]) {
        const response = await post(`${base}${path}`, TICKET_EVENT);
        assert.equal(response.status, 404, `expected 404 for ${path}`);
      }
      assert.equal(harness.forwarded.length, 0);
      assert.equal(vendor.calls, 0);
    });
  } finally {
    vendor.restore();
  }
});

test("answers 405 for a method other than POST", async () => {
  await withHarness(async (harness) => {
    const response = await send(harness.url, "GET", {});
    assert.equal(response.status, 405);
  });
});

test("answers 413 and never forwards a body over the cap", async () => {
  const vendor = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    await withHarness(async (harness) => {
      const oversized = Buffer.alloc(MAX_WEBHOOK_BYTES + 1, 0x61);
      const response = await post(harness.url, oversized, {
        signature: signatureFor(oversized),
      });
      assert.equal(response.status, 413);
      assert.equal((await response.json()).field, "body");
      assert.equal(harness.forwarded.length, 0);
      assert.equal(vendor.calls, 0);
    });
  } finally {
    vendor.restore();
  }
});

test("answers 400 naming the missing device link, and posts nothing", async () => {
  const vendor = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    await withHarness(async (harness) => {
      const body = Buffer.from(
        JSON.stringify({
          type: "ticket.updated",
          payload: { ticket_id: "t-1001" },
        }),
      );
      const response = await post(harness.url, body);
      assert.equal(response.status, 400);
      assert.deepEqual(await response.json(), {
        error: "payload.device_id is required",
        field: "payload.device_id",
      });
      assert.equal(vendor.calls, 0, "a refusal must not be pushed");
    });
  } finally {
    vendor.restore();
  }
});

test("answers 400 for a body that is not JSON", async () => {
  await withHarness(async (harness) => {
    const body = Buffer.from("this is not json");
    const response = await post(harness.url, body);
    assert.equal(response.status, 400);
    assert.equal((await response.json()).field, "body");
  });
});

test("answers 200 and forwards nothing for an event it does not act on", async () => {
  const vendor = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    await withHarness(async (harness) => {
      const body = Buffer.from(
        JSON.stringify({ type: "client.created", payload: { id: "c1" } }),
      );
      const response = await post(harness.url, body);
      assert.equal(response.status, 200);
      assert.equal(vendor.calls, 0);
    });
  } finally {
    vendor.restore();
  }
});
