// Wave 43 lane 18 (`p43-workflow-closeout-ui`) — drive REAL workflow runs for the
// run ledger, the run detail and the operator retry.
//
// Why this exists: the sanctioned entry point (`scripts/e2e-stack.sh up`) boots
// core-api and realtime but NOT the workflow worker, so no rule ever matches and
// no run is ever inserted (row 20, item G). This lane boots the worker by hand
// against the same stack (see the boot block in REVIEW.md / GATES.md) so the
// ledger captures show real executions instead of inserted fixture rows. That is
// a fact about the environment, stated rather than hidden.
//
// Every document is authored through the public path
// (`POST /api/v1/workflows/documents`, validated by core-domain before storage),
// executed by the worker, and read back through the public ledger
// (`GET /api/v1/workflows/runs/{id}`). The retry verbs go through the product's
// own route (`POST /api/v1/workflows/runs/{id}/retry`).
//
// Five documents, one shared `ticket.updated` event, plus one that never fires:
//   green     — only implemented nodes, so it must reach 3/3. The succeeded run.
//   mixed     — one comment (succeeds) then an unimplemented capture node, so it
//               fails at step 1 and KEEPS a prefix. The refused-retry subject.
//   bare      — the capture node alone, so it fails at step 0 and keeps nothing.
//   retryable — a condition node (declared idempotent) that the worker refuses,
//               so a resume retry is ACCEPTED and mints a new run.
//   parked    — a delay node, so the run waits and the gold banner has a subject.
//   idle      — trigger `ticket.created`, which the shared event never fires, so
//               the never-run empty state has a real workflow behind it.
//
// Usage: set -a; . e2e/.e2e-stack.env; set +a
//        node e2e/p43-workflow-closeout-drive.mjs
import { mkdirSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";

const OUT = "design/qa-p43-workflow-closeout-ui/traces";
const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const PNG_1x1 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

mkdirSync(OUT, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The auth gate throttles a cold first attempt; retry the way the flows do.
let token = null;
for (let i = 0; i < 6; i += 1) {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (res.ok) {
    token = (await res.json()).access_token;
    break;
  }
  await sleep(4000);
}
if (!token) throw new Error("owner login failed through the auth gate");

const api = async (path, init = {}) => {
  const res = await fetch(`${API}/api/v1${path}`, {
    method: init.method ?? "GET",
    headers: {
      authorization: `Bearer ${token}`,
      ...(init.body === undefined ? {} : { "content-type": "application/json" }),
      ...(init.headers ?? {}),
    },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${init.method ?? "GET"} ${path} -> ${res.status}: ${text}`);
  return text === "" ? undefined : JSON.parse(text);
};

// Records the status AND body, because a refusal is the evidence here.
const apiRaw = async (path, init = {}) => {
  const res = await fetch(`${API}/api/v1${path}`, {
    method: init.method ?? "GET",
    headers: {
      authorization: `Bearer ${token}`,
      ...(init.body === undefined ? {} : { "content-type": "application/json" }),
    },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  });
  const text = await res.text();
  let json = null;
  try {
    json = text === "" ? null : JSON.parse(text);
  } catch {
    json = { raw: text.slice(0, 400) };
  }
  return { status: res.status, body: json };
};

const me = await api("/me");
const ownerId = me.user_id ?? me.user?.id ?? me.id;

const ticketsRaw = await api("/tickets?limit=5");
const tickets = Array.isArray(ticketsRaw) ? ticketsRaw : (ticketsRaw.tickets ?? []);
if (tickets.length === 0) throw new Error("no seeded ticket");
const ticket = tickets.length > 1 ? tickets[1] : tickets[0];
const ticketId = ticket.id ?? ticket.ticket?.id;

// The registry, read through the product's own catalogue route. This is what the
// trigger picker renders, and it is kept as evidence.
const catalogue = await api("/workflows/nodes");
writeFileSync(`${OUT}/node-catalogue.json`, JSON.stringify(catalogue, null, 2) + "\n");

// The seeded demo org carries no consent wording, and no public path authors
// version 1 (lane 19's REVIEW H3 — the POST answers 422 here). Insert the
// product's own default text exactly as `register` seeds it for a new org
// (`crates/core-db/src/orgs.rs:352`), which is the RUNBOOK's documented fixture.
let consentVersion;
try {
  consentVersion = (await api("/org/sign-off-consents/current")).version;
} catch {
  const orgId = execFileSync(
    "psql",
    [
      process.env.DATABASE_URL,
      "-v",
      "ON_ERROR_STOP=1",
      "-tAc",
      "SELECT org_id FROM automation_rules ORDER BY created_at LIMIT 1",
    ],
    { encoding: "utf8" },
  ).trim();
  execFileSync(
    "psql",
    [
      process.env.DATABASE_URL,
      "-v",
      "ON_ERROR_STOP=1",
      "-c",
      `INSERT INTO sign_off_consents (id, org_id, version, body, created_by) VALUES (gen_random_uuid(), '${orgId}', 1, 'I confirm the work described above was completed to my satisfaction and I am authorised to sign off on it.', NULL) ON CONFLICT (org_id, version) DO NOTHING;`,
    ],
    { encoding: "utf8" },
  );
  consentVersion = (await api("/org/sign-off-consents/current")).version;
  console.log(`consent wording absent -> seeded version ${consentVersion} (RUNBOOK fixture)`);
}

const attRes = await fetch(`${API}/api/v1/tickets/${ticketId}/attachments`, {
  method: "POST",
  headers: {
    authorization: `Bearer ${token}`,
    "content-type": "image/png",
    "x-filename": "signature-lane18.png",
  },
  body: Buffer.from(PNG_1x1, "base64"),
});
if (!attRes.ok) throw new Error(`attachment upload failed: ${attRes.status}`);
const attachmentId = (await attRes.json()).id;

const stop = { id: "stop", node_type: "flow.stop", params: {} };

const docs = {
  green: {
    trigger: "ticket.updated",
    def: {
      version: "1",
      nodes: [
        {
          id: "comment",
          node_type: "ticket.comment",
          params: { ticketId, body: `Closeout run ${Date.now() % 100000}: the executor reached this comment.` },
        },
        {
          id: "notify",
          node_type: "notification.send",
          params: {
            userIds: [ownerId],
            kind: "ticket_update",
            title: "Closeout run finished",
            body: "The green workflow ran end to end.",
          },
        },
        stop,
      ],
      edges: [
        { from: "comment", to: "notify" },
        { from: "notify", to: "stop" },
      ],
      loop_max_depth: 1,
    },
  },
  mixed: {
    trigger: "ticket.updated",
    def: {
      version: "1",
      nodes: [
        {
          id: "comment",
          node_type: "ticket.comment",
          params: { ticketId, body: "Closeout mixed run: this step succeeded and must be kept." },
        },
        {
          id: "capture",
          node_type: "signOff.capture",
          params: {
            ticketId,
            signerName: "Dana Client",
            attachmentId,
            consentVersion: String(consentVersion),
          },
        },
        stop,
      ],
      edges: [
        { from: "comment", to: "capture" },
        { from: "capture", to: "stop" },
      ],
      loop_max_depth: 1,
    },
  },
  bare: {
    trigger: "ticket.updated",
    def: {
      version: "1",
      nodes: [
        {
          id: "capture",
          node_type: "signOff.capture",
          params: {
            ticketId,
            signerName: "Dana Client",
            attachmentId,
            consentVersion: String(consentVersion),
          },
        },
        stop,
      ],
      edges: [{ from: "capture", to: "stop" }],
      loop_max_depth: 1,
    },
  },
  retryable: {
    trigger: "ticket.updated",
    def: {
      version: "1",
      nodes: [
        {
          id: "check",
          node_type: "field.equals",
          params: { field: "ticket.updated.title", value: "anything" },
        },
        stop,
      ],
      edges: [{ from: "check", to: "stop" }],
      loop_max_depth: 1,
    },
  },
  parked: {
    trigger: "ticket.updated",
    def: {
      version: "1",
      nodes: [
        { id: "wait", node_type: "flow.delay", params: { duration: "1h" } },
        stop,
      ],
      edges: [{ from: "wait", to: "stop" }],
      loop_max_depth: 1,
    },
  },
  idle: {
    trigger: "ticket.created",
    def: {
      version: "1",
      nodes: [
        { id: "comment", node_type: "ticket.comment", params: { ticketId, body: "Never fires on this event." } },
        stop,
      ],
      edges: [{ from: "comment", to: "stop" }],
      loop_max_depth: 1,
    },
  },
};

const tag = Date.now();
const authored = {};
for (const [label, { trigger, def }] of Object.entries(docs)) {
  authored[label] = await api("/workflows/documents", {
    method: "POST",
    body: { name: `lane18 ${label} ${tag}`, trigger_event: trigger, definition: def },
  });
  console.log(`authored ${label}: rule ${authored[label].id}`);
}

// One real event starts every ticket.updated document.
await api(`/tickets/${ticketId}`, {
  method: "PATCH",
  body: {
    title: `Lane 18 closeout run ${tag}`,
    body: null,
    priority: null,
    assignee_id: null,
    client_id: null,
    site_id: null,
    contact_id: null,
    due_at: null,
    custom_fields: null,
    type_id: null,
    category_id: null,
    idempotency_key: `lane18-${tag}`,
  },
});

// Wait for the terminal runs.
const wanted = new Set(
  ["green", "mixed", "bare", "retryable", "parked"].map((l) => authored[l].id),
);
let seen = {};
const deadline = Date.now() + 90_000;
while (Date.now() < deadline) {
  const list = await api("/workflows/runs?limit=100");
  for (const run of list.runs ?? []) if (run.rule_id && wanted.has(run.rule_id)) seen[run.rule_id] = run;
  const settled = Object.values(seen).filter((r) => !["running"].includes(r.status));
  if (settled.length === wanted.size) break;
  await sleep(1500);
}

const report = { ticket_id: ticketId, attachment_id: attachmentId, documents: {}, runs: {}, retries: {} };
for (const [label, doc] of Object.entries(authored)) {
  report.documents[label] = { id: doc.id, name: doc.name, trigger: docs[label].trigger };
}

for (const label of Object.keys(docs)) {
  const run = seen[authored[label].id];
  if (!run) {
    report.runs[label] = { found: false };
    continue;
  }
  const detail = await api(`/workflows/runs/${run.id}`);
  writeFileSync(`${OUT}/run-${label}.json`, JSON.stringify(detail, null, 2) + "\n");
  report.runs[label] = {
    found: true,
    run_id: run.id,
    status: run.status,
    error_code: run.error_code ?? null,
    error: run.error ?? null,
    steps: (detail.steps ?? []).map((s) => ({
      step_index: s.step_index,
      node_id: s.node_id,
      status: s.status,
      error_code: s.error_code ?? null,
    })),
  };
  console.log(
    `${label}: ${run.status}${run.error_code ? ` (${run.error_code})` : ""} — ` +
      (detail.steps ?? []).map((s) => `${s.node_id}:${s.status}`).join(" → "),
  );
}

// --- the two retry verbs, against the failed runs ---------------------------
async function retry(label, mode) {
  const run = report.runs[label];
  if (!run?.found) return;
  const res = await apiRaw(`/workflows/runs/${run.run_id}/retry`, {
    method: "POST",
    body: { mode, idempotency_key: randomUUID() },
  });
  const key = `${label}:${mode}`;
  report.retries[key] = {
    target_run: run.run_id,
    http_status: res.status,
    accepted: res.status < 300,
    new_run_id: res.body?.run?.id ?? null,
    refusal_reason: res.body?.error?.reason ?? null,
    refusal_message: res.body?.error?.message ?? null,
    body: res.status < 300 ? undefined : res.body,
  };
  writeFileSync(`${OUT}/retry-${label}-${mode}.json`, JSON.stringify(res, null, 2) + "\n");
  console.log(
    `retry ${key}: HTTP ${res.status} ${
      res.status < 300 ? `→ new run ${res.body?.run?.id}` : `refused: ${res.body?.error?.message}`
    }`,
  );
  return res.body?.run?.id ?? null;
}

await retry("mixed", "resume");
await retry("mixed", "from_start");
const acceptedNew = await retry("retryable", "resume");
await sleep(3000);
// Read the run a retry minted, so the detail surface has a real retried run.
if (acceptedNew) {
  try {
    const child = await api(`/workflows/runs/${acceptedNew}`);
    writeFileSync(`${OUT}/run-retry-child.json`, JSON.stringify(child, null, 2) + "\n");
    report.runs.retry_child = { found: true, run_id: acceptedNew, status: child.status };
    console.log(`retry child: ${child.status}`);
  } catch (err) {
    console.log(`retry child read failed: ${err.message}`);
  }
}

writeFileSync(`${OUT}/drive-report.json`, JSON.stringify(report, null, 2) + "\n");
console.log(`\ndrive-report.json written; ledger filter also has ${Object.keys(authored).length} rules`);
console.log(`never-run rule id: ${authored.idle.id}`);
