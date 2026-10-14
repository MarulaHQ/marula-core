#!/usr/bin/env node
/**
 * demo-ai-probe.mjs — prove the BYOK slot works before a prospect is in the
 * room (Wave 47 lane B2, `p47-byok-runbook`).
 *
 * Two live calls, one per leg of the AI edge:
 *
 *   1. COMPLETION, through the sealed credential. `POST
 *      /api/v1/ai/chat/sessions/{id}/turns` on the `general_chat` surface
 *      runs the same admission pipeline a real call runs — org master switch,
 *      per-surface grant, model allowlist, caps, sealed credential — and then
 *      makes a real provider call. A green leg proves the whole chain, not
 *      just the seal.
 *   2. EMBEDDING, against the 768-dimension contract (`SEARCH_EMBEDDING_DIM`
 *      in core-domain, enforced by `parse_embedding_response` in
 *      core-api/src/ai.rs). The product's embeddings adapter has no HTTP
 *      surface that reports its outcome — search degrades silently and the
 *      backfill sweep is worker-machine only — so the leg calls the
 *      provider's OpenAI-compatible `/embeddings` endpoint directly, with the
 *      key the operator supplies, and asserts the width the product would
 *      enforce. Mirrors ai.rs:33-40 for base URLs and ai.rs:111-121 for the
 *      width check.
 *
 * What it never does: print, log, or echo key material. Every line this
 * script writes passes a scrubber that redacts Bearer tokens, `sk-`-style
 * keys, and long opaque strings before they reach the terminal.
 *
 * Configuration (flags win, then environment):
 *   --api URL        API base              (PROBE_API_URL | E2E_API_URL | http://localhost:8080)
 *   --email ADDR     login                 (PROBE_EMAIL | default: owner@demo.local, the seed's owner)
 *   --password PASS  login password        (PROBE_PASSWORD | default: the shared demo password)
 *   --surface KEY    chat surface           (PROBE_SURFACE | default: general_chat)
 *   --provider NAME  provider/model pair   (PROBE_PROVIDER | default: first enabled catalog entry)
 *   --model NAME     model to call          (PROBE_MODEL)
 *   --embed-model M  embedding model        (PROBE_EMBED_MODEL | default: text-embedding-3-small)
 *   --api-key KEY    provider key for the embedding leg only (PROBE_API_KEY; not needed for ollama)
 *   --skip-embed     run the completion leg alone
 *   --timeout MS     per-call timeout in milliseconds (default 60000)
 *
 * Exit 0 = the completion leg proved (and the embedding leg proved or was
 * skipped). Exit 1 = a leg failed; the last line names the failure mode and
 * the fix. Exit 2 = the probe could not configure itself (missing flag, no
 * reachable API, login refused).
 *
 * Usage:
 *   node scripts/demo-ai-probe.mjs
 *   node scripts/demo-ai-probe.mjs --provider openai --model gpt-4.1 \
 *     --embed-model text-embedding-3-small --api-key "$OPENAI_API_KEY"
 */

// --- small helpers -----------------------------------------------------------

/**
 * @typedef {Object} ProbeOptions
 * @property {string} api
 * @property {string} email
 * @property {string} password
 * @property {string} surface
 * @property {string | null} provider
 * @property {string | null} model
 * @property {string | null} embedModel
 * @property {string | null} apiKey
 * @property {boolean} skipEmbed
 * @property {number} timeout
 */

/** @param {string} message @param {"info" | "ok" | "fail"} [tone] */
function say(message, tone = "info") {
  const line = scrub(message);
  if (tone === "ok") process.stdout.write(`ok    ${line}\n`);
  else if (tone === "fail") process.stdout.write(`FAIL  ${line}\n`);
  else process.stdout.write(`      ${line}\n`);
}

/**
 * Redacts anything that could be key material from text before it is
 * printed. The probe never handles a key it does not have to, but SSE error
 * reasons and provider messages are upstream text, so they get scrubbed too.
 * @param {string} text
 * @returns {string}
 */
function scrub(text) {
  return String(text)
    .replace(/Bearer\s+[A-Za-z0-9._~+/-]+=*/gi, "Bearer [redacted]")
    .replace(/\b(sk|pk|rk)-[A-Za-z0-9_-]{8,}/g, "[redacted-key]")
    .replace(/\b[A-Za-z0-9_-]{40,}={0,2}\b/g, "[redacted-token]")
    .replace(/(api[-_]?key|token|secret|password)["']?\s*[:=]\s*["']?[^"'\s,}]+/gi, "$1=[redacted]");
}

/**
 * Typed failure the probe knows how to explain. `mode` is the machine reason
 * the server sent (or a probe-local mode); `fix` is the operator action.
 */
class ProbeFailure extends Error {
  /**
   * @param {string} mode
   * @param {string} what
   * @param {string} fix
   */
  constructor(mode, what, fix) {
    super(`${mode}: ${what} — ${fix}`);
    this.name = "ProbeFailure";
    this.mode = mode;
    this.what = what;
    this.fix = fix;
  }
}

/**
 * The remediation table for the admission pipeline's typed refusals. Every
 * reason the server can answer with, and the step that clears it. Mirrors
 * `authorize_surface_call` in crates/core-api/src/routes/ai_policy.rs.
 * @param {string} reason
 * @returns {{what: string, fix: string} | null}
 */
function admissionAdvice(reason) {
  switch (reason) {
    case "ai_features_disabled":
      return {
        what: "AI features are switched off for this org",
        fix: "Settings → AI → Allow AI in this organisation → On (the ai.features_enabled setting)",
      };
    case "ai_surface_not_granted":
      return {
        what: "no grant covers this user on the surface (deny-by-default)",
        fix: "Settings → AI → Surfaces → grant the surface to the sign-in role, then re-run",
      };
    case "ai_model_not_allowed":
      return {
        what: "the (provider, model) pair is not on the enabled allowlist",
        fix: "Settings → AI → Model catalog → add the pair, or pass --provider/--model that are catalogued",
      };
    case "ai_model_not_capable":
      return {
        what: "the allowlisted model lacks the capabilities this edge requires",
        fix: "catalog the pair with the completion and tools capability tags",
      };
    case "ai_hard_cap_reached":
      return {
        what: "a hard cap bound before dispatch",
        fix: "raise the cap in Settings → AI → Caps, or wait for the period window to roll",
      };
    case "ai_no_credential":
      return {
        what: "no active credential for the resolved provider",
        fix: "Settings → AI → Credential slot → save the provider key (any non-empty value works for ollama)",
      };
    case "ai_surface_internal":
      return {
        what: "that surface is internal and never directly callable",
        fix: "pick a native surface such as general_chat",
      };
    case "ai_surface_disabled":
      return {
        what: "the surface is disabled for this org",
        fix: "enable the surface in Settings → AI → Surfaces",
      };
    case "ai_personal_credentials_disabled":
      return {
        what: "personal credentials are disabled by org policy",
        fix: "save the credential on the org slot instead",
      };
    default:
      return null;
  }
}

// --- argument + environment parsing ------------------------------------------

/**
 * @returns {ProbeOptions}
 */
function parseArgs() {
  /** @type {Record<string, string>} */
  const flags = {};
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (!arg.startsWith("--")) continue;
    const key = arg.slice(2);
    if (key === "skip-embed") {
      flags[key] = "true";
      continue;
    }
    const value = argv[i + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new ProbeFailure(
        "probe_usage",
        `flag --${key} needs a value`,
        "run with --help values as documented in the script header",
      );
    }
    flags[key] = value;
    i += 1;
  }

  const pick = (...candidates) => {
    for (const candidate of candidates) {
      if (candidate !== undefined && candidate !== "") return candidate;
    }
    return undefined;
  };

  const api = pick(flags.api, process.env.PROBE_API_URL, process.env.E2E_API_URL, "http://localhost:8080");
  const email = pick(flags.email, process.env.PROBE_EMAIL, "owner@demo.local");
  const password = pick(flags.password, process.env.PROBE_PASSWORD, "demo-password-123");

  if (!/^https?:\/\//.test(api)) {
    throw new ProbeFailure(
      "probe_usage",
      `API base URL must start with http:// or https:// (got ${api})`,
      "pass --api http://localhost:8080 or set PROBE_API_URL",
    );
  }
  const timeout = Number(pick(flags.timeout, process.env.PROBE_TIMEOUT, "60000"));
  if (!Number.isFinite(timeout) || timeout <= 0) {
    throw new ProbeFailure("probe_usage", `timeout must be a positive number (got ${timeout})`, "pass --timeout 60000");
  }

  return {
    api: api.replace(/\/+$/, ""),
    email,
    password,
    surface: pick(flags.surface, process.env.PROBE_SURFACE, "general_chat"),
    provider: pick(flags.provider, process.env.PROBE_PROVIDER, null),
    model: pick(flags.model, process.env.PROBE_MODEL, null),
    embedModel: pick(flags["embed-model"], process.env.PROBE_EMBED_MODEL, "text-embedding-3-small"),
    apiKey: pick(flags["api-key"], process.env.PROBE_API_KEY, null),
    skipEmbed: flags["skip-embed"] === "true",
    timeout,
  };
}

// --- HTTP ---------------------------------------------------------------------

/**
 * One JSON call. Never throws on a non-2xx: the caller branches on the typed
 * body. Network-level failures become a ProbeFailure the operator can act on.
 * @param {string} api API base URL, no trailing slash
 * @param {string} path repo-relative API path or absolute URL
 * @param {{method?: string, token?: string | null, body?: unknown, timeout?: number}} [init]
 * @returns {Promise<{status: number, json: any, text: string}>}
 */
async function call(api, path, init = {}) {
  const { method = "GET", token = null, body, timeout = 60000 } = init;
  const url = `${path.startsWith("http") ? path : `${api}${path}`}`;
  /** @type {Record<string, string>} */
  const headers = { accept: "application/json" };
  if (token) headers.authorization = `Bearer ${token}`;
  let payload;
  if (body !== undefined) {
    headers["content-type"] = "application/json";
    payload = JSON.stringify(body);
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);
  let response;
  try {
    response = await fetch(url, { method, headers, body: payload, signal: controller.signal });
  } catch (error) {
    const reason = error instanceof Error && error.name === "AbortError" ? "timed out" : String(error);
    throw new ProbeFailure(
      "provider_unreachable",
      `${method} ${url} could not be reached (${reason})`,
      "check the API base URL and that core-api is up",
    );
  } finally {
    clearTimeout(timer);
  }
  const text = await response.text();
  let json = null;
  try {
    json = text === "" ? null : JSON.parse(text);
  } catch {
    json = null;
  }
  if (!response.ok) {
    // Typed server refusal: surface reason/message verbatim (already scrubbed).
    const reason = json?.reason ?? `http_${response.status}`;
    const message = json?.message ?? response.statusText;
    return { status: response.status, json: { ...(json ?? {}), reason, message }, text };
  }
  return { status: response.status, json, text };
}

// --- login ---------------------------------------------------------------------

/**
 * Password login. MFA-gated accounts cannot drive this probe, and saying so
 * beats a confusing 401.
 * @param {ProbeOptions} options
 * @returns {Promise<string>} the access token (never printed)
 */
async function login(options) {
  const response = await call(options.api, "/api/v1/auth/login", {
    method: "POST",
    body: { email: options.email, password: options.password },
    timeout: options.timeout,
  });
  const body = response.json;
  if (body?.mfa_required === true) {
    throw new ProbeFailure(
      "login_mfa_required",
      `the account ${options.email} is MFA-gated`,
      "use a demo account without a second factor for the probe (the seeded demo logins are not gated)",
    );
  }
  const token = body?.access_token;
  if (response.status !== 200 || typeof token !== "string" || token === "") {
    throw new ProbeFailure(
      "login_refused",
      `login as ${options.email} failed (${body?.reason ?? response.status})`,
      "check PROBE_EMAIL/PROBE_PASSWORD against the seeded demo logins",
    );
  }
  return token;
}

// --- leg 1: completion through the sealed credential ---------------------------

/**
 * Reads SSE frames out of a chunk buffer. The runner writes
 * `event: <name>` plus `data: <json>` per frame, blank line between frames
 * (see `emit` in crates/core-api/src/routes/chat_runner.rs).
 * @param {string} chunk
 * @returns {{frames: Array<{event: string, data: any}>, rest: string}}
 */
function readFrames(chunk) {
  /** @type {Array<{event: string, data: any}>} */
  const frames = [];
  let buffer = chunk;
  for (;;) {
    const boundary = buffer.indexOf("\n\n");
    if (boundary === -1) break;
    const block = buffer.slice(0, boundary);
    buffer = buffer.slice(boundary + 2);
    const eventLine = block.split("\n").find((line) => line.startsWith("event:"));
    const dataLine = block.split("\n").find((line) => line.startsWith("data:"));
    if (!eventLine || !dataLine) continue;
    const raw = dataLine.slice(5).trim();
    try {
      frames.push({ event: eventLine.slice(6).trim(), data: JSON.parse(raw) });
    } catch {
      frames.push({ event: "unparseable_frame", data: { raw } });
    }
  }
  return { frames, rest: buffer };
}

/**
 * The completion leg. Streams one chat turn on the given surface and maps
 * every outcome to pass/fail with the server's own typed wording.
 * @param {ProbeOptions} options
 * @param {string} token
 * @param {{provider: string, model: string}} pair
 */
async function probeCompletion(options, token, pair) {
  const sessionId = crypto.randomUUID();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), options.timeout);
  let response;
  try {
    response = await fetch(`${options.api}/api/v1/ai/chat/sessions/${sessionId}/turns`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
      body: JSON.stringify({
        surface_key: options.surface,
        messages: [{ role: "user", content: "Reply with the single word: ready" }],
        provider: pair.provider,
        model: pair.model,
        session_opener: true,
      }),
      signal: controller.signal,
    });
  } catch (error) {
    clearTimeout(timer);
    const reason = error instanceof Error && error.name === "AbortError" ? "timed out" : String(error);
    throw new ProbeFailure(
      "provider_unreachable",
      `the chat turn could not be sent (${reason})`,
      "check that core-api is reachable at the configured API base URL",
    );
  }
  clearTimeout(timer);

  if (!response.ok) {
    let body = null;
    try {
      body = await response.json();
    } catch {
      body = null;
    }
    const reason = body?.reason ?? `http_${response.status}`;
    const message = body?.message ?? response.statusText;
    if (response.status === 401) {
      throw new ProbeFailure("login_expired", message, "re-run the probe to get a fresh token");
    }
    if (response.status === 429) {
      throw new ProbeFailure("turn_rate_limited", message, "wait a minute and re-run; the runner caps turns per user");
    }
    const advice = admissionAdvice(reason);
    if (advice) throw new ProbeFailure(reason, `${advice.what} (server said: ${message})`, advice.fix);
    throw new ProbeFailure(reason, message, "read docs/site/how-to/admin-ai-bringup.md for the bring-up order");
  }

  const started = {
    provider: "",
    model: "",
    resolution: "",
    warnings: [],
  };
  let completed = null;
  let deltaText = "";
  let buffer = "";
  const reader = response.body?.getReader();
  const decoder = new TextDecoder();
  if (reader) {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const { frames, rest } = readFrames(buffer);
      buffer = rest;
      for (const frame of frames) {
        if (frame.event === "turn_started") {
          started.provider = frame.data?.provider ?? "";
          started.model = frame.data?.model ?? "";
          started.resolution = frame.data?.resolution ?? "";
          started.warnings = Array.isArray(frame.data?.warnings) ? frame.data.warnings : [];
        }
        if (frame.event === "assistant_delta" && typeof frame.data?.content === "string") {
          deltaText += frame.data.content;
        }
        if (frame.event === "turn_completed") completed = frame.data;
      }
    }
  }

  if (completed === null) {
    throw new ProbeFailure(
      "turn_stream_truncated",
      "the stream ended before turn_completed",
      "re-run; if it repeats, check the core-api logs for the runner",
    );
  }
  if (completed.outcome !== "completed") {
    const error = completed.error ?? {};
    const mode = typeof error.event === "string" ? error.event : completed.outcome;
    if (mode === "model_rate_limited") {
      throw new ProbeFailure(
        "model_rate_limited",
        `the provider rate-limited the call (${error.reason ?? "no reason given"})`,
        "wait for the provider window, or lower the demo's call rate",
      );
    }
    if (mode === "model_transport_failed") {
      throw new ProbeFailure(
        "provider_unreachable",
        `the provider refused the transport (${error.reason ?? "no reason given"})`,
        "check the provider's base URL (OLLAMA_BASE_URL / OPENAI_BASE_URL on core-api) and that the runtime is up",
      );
    }
    if (mode === "hard_cap_reached") {
      throw new ProbeFailure(
        "ai_hard_cap_reached",
        "a hard cap bound mid-run",
        "raise the cap in Settings → AI → Caps and re-run",
      );
    }
    throw new ProbeFailure(
      "turn_failed",
      `the turn ended as ${completed.outcome}${error.reason ? ` (${error.reason})` : ""}`,
      "read the core-api logs for this turn",
    );
  }
  if (started.warnings.length > 0) {
    say(`soft-cap warning on this call: ${started.warnings.join(", ")} (the call still ran)`);
  }
  say(`provider ${started.provider}, model ${started.model}, resolution ${started.resolution}`);
  say(`the model answered: "${deltaText.trim().slice(0, 120) || "(empty content)"}"`, "ok");
}

// --- leg 2: embedding against the 768-dim contract ------------------------------

/**
 * The embedding leg. Calls the provider's OpenAI-compatible endpoint with the
 * operator-supplied key and asserts the exact width the product enforces.
 * @param {ProbeOptions} options
 */
async function probeEmbedding(options) {
  const provider = (options.provider ?? "").trim().toLowerCase();
  const base = (() => {
    if (provider === "ollama") {
      return (process.env.OLLAMA_BASE_URL ?? "http://localhost:11434/v1").replace(/\/+$/, "");
    }
    return (process.env.OPENAI_BASE_URL ?? "https://api.openai.com/v1").replace(/\/+$/, "");
  })();
  const usesKey = provider !== "ollama";
  if (usesKey && (options.apiKey === null || options.apiKey === "")) {
    throw new ProbeFailure(
      "probe_usage",
      "the embedding leg needs the provider key the completion leg seals",
      "pass --api-key (or PROBE_API_KEY), or use --skip-embed when the sealed slot is the only proof you need",
    );
  }
  const model = options.embedModel;
  const body = { model, input: ["marula byok probe"], encoding_format: "float" };
  if (provider !== "ollama") {
    // Matryoshka truncation, exactly as the product's adapter sends it.
    body.dimensions = 768;
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), options.timeout);
  let response;
  try {
    response = await fetch(`${base}/embeddings`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(usesKey ? { authorization: `Bearer ${options.apiKey}` } : {}),
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } catch (error) {
    clearTimeout(timer);
    const reason = error instanceof Error && error.name === "AbortError" ? "timed out" : String(error);
    throw new ProbeFailure(
      "provider_unreachable",
      `the embeddings endpoint at ${base} could not be reached (${reason})`,
      "check OLLAMA_BASE_URL / OPENAI_BASE_URL and that the model runtime is up",
    );
  }
  clearTimeout(timer);

  if (!response.ok) {
    const text = await response.text();
    throw new ProbeFailure(
      "embedding_rejected",
      `the provider answered ${response.status} to the embedding call`,
      "the key may lack embeddings access, or the model name may be wrong",
    );
  }
  let parsed = null;
  try {
    parsed = await response.json();
  } catch {
    throw new ProbeFailure(
      "embedding_malformed",
      "the embeddings response was not JSON",
      "check that the base URL serves an OpenAI-compatible /embeddings route",
    );
  }
  const vectors = Array.isArray(parsed?.data) ? parsed.data : [];
  if (vectors.length === 0) {
    throw new ProbeFailure("embedding_malformed", "the embeddings response carried no vectors", "retry; if it repeats, check the provider");
  }
  const widths = new Set(vectors.map((entry) => (Array.isArray(entry?.embedding) ? entry.embedding.length : -1)));
  if (widths.size !== 1 || !widths.has(768)) {
    throw new ProbeFailure(
      "embedding_wrong_width",
      `the model returned ${[...widths].join("/")}-dimension vectors, and the product requires exactly 768`,
      "configure a conforming embedding model (text-embedding-3-small, or nomic-embed-text on ollama)",
    );
  }
  say(`model ${model} returned ${vectors.length} vector(s), 768 dimensions each`, "ok");
}

// --- preflight ------------------------------------------------------------------

/**
 * Reads the org's AI posture so a failure names the real state, and picks the
 * (provider, model) pair when the operator did not pass one. The matrix read
 * is admin-only; a non-admin login still gets the status overview.
 * @param {ProbeOptions} options
 * @param {string} token
 * @returns {Promise<{provider: string, model: string}>}
 */
async function preflight(options, token) {
  const status = await call(options.api, "/api/v1/settings/ai/status", { token, timeout: options.timeout });
  if (status.status !== 200) {
    throw new ProbeFailure(
      "status_unreadable",
      `the AI status overview answered ${status.status}`,
      "re-run with an account that can read org settings",
    );
  }
  const active = status.json?.active_provider ?? "none";
  const activeModel = status.json?.active_model ?? "none";
  const configured = Array.isArray(status.json?.configured_providers) ? status.json.configured_providers : [];
  say(`active provider: ${active}, active model: ${activeModel}`);
  say(`legacy byok slot holds keys for: ${configured.length > 0 ? configured.join(", ") : "(none)"}`);

  if (options.provider !== null && options.model !== null) return { provider: options.provider, model: options.model };

  const matrix = await call(options.api, "/api/v1/settings/ai/matrix", { token, timeout: options.timeout });
  const catalog = Array.isArray(matrix.json?.catalog) ? matrix.json.catalog : [];
  const enabled = catalog.filter((entry) => entry?.enabled === true);
  if (options.provider !== null) {
    const entry = enabled.find((item) => item.provider === options.provider);
    if (entry) return { provider: entry.provider, model: entry.model };
    if (options.model !== null) return { provider: options.provider, model: options.model };
    throw new ProbeFailure(
      "ai_model_not_allowed",
      `provider ${options.provider} has no enabled catalog entry`,
      "add the pair in Settings → AI → Model catalog, or pass --provider and --model explicitly",
    );
  }
  if (enabled.length === 0) {
    throw new ProbeFailure(
      "ai_model_not_allowed",
      "the model catalog is empty, so nothing is allowlisted",
      "Settings → AI → Model catalog → add the (provider, model) pair with completion and tools tags",
    );
  }
  const entry = enabled[0];
  say(`no --provider/--model given; using the first enabled catalog entry`);
  return { provider: entry.provider, model: entry.model };
}

// --- main -----------------------------------------------------------------------

async function main() {
  const options = parseArgs();
  say(`api: ${options.api}`);
  const token = await login(options);
  say(`signed in as ${options.email}`, "ok");

  const pair = await preflight(options, token);
  say(`completion leg: ${pair.provider}/${pair.model} on ${options.surface}`);
  await probeCompletion(options, token, pair);

  if (options.skipEmbed) {
    say("embedding leg skipped (--skip-embed)");
    return;
  }
  say(`embedding leg: ${options.embedModel} against the 768-dimension contract`);
  await probeEmbedding(options);
}

main()
  .then(() => {
    process.stdout.write("\nPROBE PASS — the sealed credential made both live calls\n");
    process.exitCode = 0;
  })
  .catch((error) => {
    const message = error instanceof ProbeFailure ? error.message : `probe_error: ${String(error?.stack ?? error)}`;
    process.stdout.write(`\n${message}\n`);
    if (error instanceof ProbeFailure) {
      process.stdout.write(`NEXT: ${scrub(error.fix)}\n`);
      process.exitCode = 1;
    } else {
      process.stdout.write("NEXT: this was not a typed failure; the line above is the raw reason\n");
      process.exitCode = 2;
    }
  });
