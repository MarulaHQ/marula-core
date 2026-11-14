import assert from "node:assert/strict";
import { test } from "node:test";

import { loadConfig, noneMode, type Config } from "./config.js";
import { buildCompany, pushCompany } from "./hudu.js";

const CONFIGURED: Config = {
  marulaBaseUrl: "https://marula.test",
  marulaApiKey: "mrl_test_key",
  marulaWebhookSecret: "whsec_test",
  port: 8789,
  huduBaseUrl: "https://acme.huducloud.com",
  huduApiKey: "hudu_test_key",
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

test("none mode is on until both Hudu settings are present", () => {
  assert.equal(noneMode(CONFIGURED), false);
  assert.equal(noneMode({ ...CONFIGURED, huduBaseUrl: undefined }), true);
  assert.equal(noneMode({ ...CONFIGURED, huduApiKey: undefined }), true);
  assert.equal(noneMode({ ...CONFIGURED, huduApiKey: "" }), true);
});

test("loadConfig refuses to start without a Marula key", () => {
  assert.throws(
    () => loadConfig({}),
    /MARULA_API_KEY is required/,
    "a connector with no Marula credential must fail loudly, not run blind",
  );
});

test("loadConfig defaults and Hudu credentials", () => {
  const config = loadConfig({
    MARULA_API_KEY: "mrl_test_key",
  });
  assert.equal(config.marulaBaseUrl, "http://localhost:8080");
  assert.equal(config.port, 8789);
  assert.equal(config.marulaWebhookSecret, "");
  assert.equal(config.huduBaseUrl, undefined);
  assert.equal(noneMode(config), true);

  const configured = loadConfig({
    MARULA_API_KEY: "mrl_test_key",
    MARULA_BASE_URL: "https://marula.test",
    MARULA_WEBHOOK_SECRET: "whsec_test",
    PORT: "9999",
    HUDU_BASE_URL: "https://acme.huducloud.com",
    HUDU_API_KEY: "hudu_test_key",
  });
  assert.deepEqual(configured, { ...CONFIGURED, port: 9999 });
  assert.equal(noneMode(configured), false);
});

test("buildCompany names the company and tags it with the client id", () => {
  const company = buildCompany({ id: "c1b2a3d4-e5f6", name: "Acme Dental" });
  assert.deepEqual(company, {
    name: "Acme Dental",
    nickname: "marula-c1b2a3d4",
  });
});

test("buildCompany tolerates an id shorter than the nickname prefix", () => {
  assert.deepEqual(buildCompany({ id: "c1", name: "Acme" }), {
    name: "Acme",
    nickname: "marula-c1",
  });
});

test("pushCompany in none mode dials nothing and reports no external call", async () => {
  const stub = stubFetch(() => new Response(null, { status: 200 }));
  try {
    const { result, logged } = await quietConsole(async () => {
      return await pushCompany(
        { ...CONFIGURED, huduApiKey: undefined },
        buildCompany({ id: "c1b2a3d4", name: "Acme Dental" }),
      );
    });
    assert.equal(await result, false);
    assert.equal(stub.calls.length, 0, "none mode must not reach the network");
    // None mode is observable, not silent: the payload is printed instead.
    assert.match(logged.join("\n"), /would upsert Hudu company/);
  } finally {
    stub.restore();
  }
});

test("pushCompany posts the company with the Hudu API key", async () => {
  const stub = stubFetch(() => new Response("{}", { status: 200 }));
  try {
    const company = buildCompany({ id: "c1b2a3d4", name: "Acme Dental" });
    assert.equal(await pushCompany(CONFIGURED, company), true);
    assert.equal(stub.calls.length, 1);
    const call = stub.calls[0];
    assert.equal(call?.url, "https://acme.huducloud.com/api/v1/companies");
    assert.equal(call?.init?.method, "POST");
    const headers = new Headers(call?.init?.headers);
    assert.equal(headers.get("x-api-key"), "hudu_test_key");
    assert.equal(headers.get("content-type"), "application/json");
    assert.deepEqual(JSON.parse(String(call?.init?.body)), { company });
  } finally {
    stub.restore();
  }
});

test("pushCompany treats 422 as an existing company, not a failure", async () => {
  const stub = stubFetch(() => new Response("{}", { status: 422 }));
  try {
    assert.equal(
      await pushCompany(CONFIGURED, buildCompany({ id: "c1", name: "Acme" })),
      true,
    );
  } finally {
    stub.restore();
  }
});

test("pushCompany throws on a refused push", async () => {
  const stub = stubFetch(() => new Response("nope", { status: 500 }));
  try {
    await assert.rejects(
      pushCompany(CONFIGURED, buildCompany({ id: "c1", name: "Acme" })),
      /Hudu push failed \(500\)/,
    );
  } finally {
    stub.restore();
  }
});

test("pushCompany propagates a transport failure", async () => {
  const original = globalThis.fetch;
  globalThis.fetch = (async () => {
    throw new Error("connection refused");
  }) as typeof globalThis.fetch;
  try {
    await assert.rejects(
      pushCompany(CONFIGURED, buildCompany({ id: "c1", name: "Acme" })),
      /connection refused/,
    );
  } finally {
    globalThis.fetch = original;
  }
});
