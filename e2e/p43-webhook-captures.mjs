// Wave 43 lane 5 `p43-webhook-closeout-ui` — vision-review captures for the
// four UI close-out items, drawn against the REAL e2e stack (never mocks):
//   A  /settings/integrations — registry-driven multiselect (origin badges,
//      wildcard copy) and the 422 refusal naming ungranted types
//   B  the inbound-sources surface — registered generic source with its ingest
//      URL + source_ref, the show-once secret card, the register/edit dialogs,
//      and the ingest log off the audit ledger (one accepted envelope, then a
//      second page via "Load older")
//   C  /automations — the rule editor under the Board subject and the ledger
//      row carrying the `no ticket` badge
//   D  /settings/plugins Approvals — the consent screen showing Automation
//      triggers and Read nodes beside the endpoints/scopes block
// Captured light + dark (plus one 375px mobile pass and one reduced-motion
// shot) into design/qa-p43-webhook-extensibility/captures/, reviewed in
// ≤8-image batches per the lane mandate.
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p43-webhook-captures.mjs
import { chromium } from "playwright";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { createHash, createHmac } from "node:crypto";
import { execSync } from "node:child_process";
import { join } from "node:path";
import { tmpdir } from "node:os";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const OUT = "design/qa-p43-webhook-extensibility/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;
const SIG_HEADER = "X-Marula-Signature";

mkdirSync(OUT, { recursive: true });

// --- API context: login (durable-gate retry) + stage true state ---------------
// The HTTP layer is Node's global fetch, not playwright's request context:
// the signed ingest needs BYTE-EXACT bodies (the HMAC covers the raw bytes),
// and playwright's request layer re-encodes string bodies.
let token = null;
for (let attempt = 0; attempt < 5; attempt += 1) {
  const login = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (login.ok) {
    token = (await login.json()).access_token;
    break;
  }
  await new Promise((r) => setTimeout(r, 4000));
}
if (!token) throw new Error("owner login failed through the auth gate");
const auth = (path, init = {}) =>
  fetch(`${API}/api/v1${path}`, {
    ...init,
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
      ...(init.headers ?? {}),
    },
  });
const json = async (res) => (res.ok ? res.json() : null);

const boards = await json(await auth("/boards/nav"));
const boardId = boards?.[0]?.id;
if (!boardId) throw new Error("no board in the seeded demo org");

// --- stage B1: a generic M2M source + two accepted envelopes ------------------
const srcRes = await auth("/alert-sources", {
  method: "POST",
  body: JSON.stringify({
    name: `Capture RMM (${STAMP})`,
    source_ref: `capture-rmm-${STAMP}`,
    board_id: boardId,
    event_type: "rmm.device_offline",
    idempotency_key: `capture-src-${STAMP}`,
  }),
});
const createdSource = await json(srcRes);
if (!createdSource?.secret) throw new Error(`source registration failed: ${srcRes.status()}`);
const sourceRef = createdSource.source_ref;
const sourceId = createdSource.id;
const sign = (body) => createHmac("sha256", createdSource.secret).update(body).digest("hex");

// Two accepted envelopes so the audit-ledger log has a page plus an "older" page.
let accepted = 0;
for (const [i, key] of ["capture-envelope-1", "capture-envelope-2"].entries()) {
  const body = JSON.stringify({
    dedup_key: `${key}-${STAMP}`,
    entity_id: `workstation-0${i + 1}`,
    payload: { host: `ws-0${i + 1}.capture.local`, offline_minutes: 12 },
  });
  const res = await fetch(`${API}/api/v1/webhooks/ingest/${sourceRef}`, {
    method: "POST",
    headers: { "content-type": "application/json", [SIG_HEADER]: sign(body) },
    body,
  });
  if (res.ok) accepted += 1;
}
console.log(`[capture] generic source ${sourceRef}: ${accepted}/2 envelopes emitted`);

// --- stage C: a ticket-less (board-subject) automation rule -------------------
const ruleRes = await auth("/automation-rules", {
  method: "POST",
  body: JSON.stringify({
    name: `Capture paging rule (${STAMP})`,
    board_id: null,
    subject_kind: "board",
    trigger_event: "webhook.ingested",
    conditions: [],
    actions: [{ type: "set_priority", priority: "p2" }],
    idempotency_key: `capture-rule-${STAMP}`,
  }),
});
const captureRuleId = (await json(ruleRes))?.id ?? null;
console.log(`[capture] board-subject rule: ${captureRuleId}`);

// --- stage D: a connector zip declaring a trigger + a read node ---------------
const stageDir = join(tmpdir(), `p43-capture-${STAMP}`);
mkdirSync(stageDir, { recursive: true });
const manifest = {
  name: "Capture NMS Bridge",
  version: `1.0.${STAMP % 1000}`,
  interface_version: 1,
  description:
    "Capture fixture: an http_connector declaring one automation trigger and one read node for the consent-screen capture.",
  artifact_kind: "http_connector",
  capabilities: ["automation.triggers", "automation.reads"],
  connector_profile: {
    base_url: "https://nms.capture.example.test/api",
    webhook_endpoints: ["https://nms.capture.example.test/hooks/marula"],
    scopes: ["devices:read", "devices:write"],
  },
  surface_profile: {
    widgets: [],
    actions: [],
    render_kinds: [],
    field_validators: [],
    triggers: [
      {
        id: "device_offline",
        label: "Device offline",
        description: "the appliance stopped checking in",
        emitted_event_type: "nms.device_offline",
      },
      {
        id: "backup_failed",
        label: "Backup failed",
        description: "a scheduled backup exited non-zero",
        emitted_event_type: "nms.backup_failed",
      },
    ],
    reads: [
      {
        id: "list_devices",
        label: "List devices",
        description: "the org's appliance inventory",
        guardrails: {
          max_calls_per_run: 10,
          min_interval_secs: 30,
          max_output_bytes: 262144,
          may_return_secrets: false,
        },
      },
    ],
  },
};
writeFileSync(join(stageDir, "manifest.json"), JSON.stringify(manifest, null, 2));
const zipPath = join(stageDir, "capture-nms-bridge.zip");
execSync(`cd "${stageDir}" && zip -q -X capture-nms-bridge.zip manifest.json`);
const zipBytes = (await import("node:fs")).readFileSync(zipPath);
const stagedRes = await fetch(`${API}/api/v1/plugins/distribution/staged/zip`, {
  method: "POST",
  headers: { "content-type": "application/zip", authorization: `Bearer ${token}` },
  body: new Uint8Array(zipBytes),
});
const staged = stagedRes.status === 409 ? null : await json(stagedRes);
console.log(`[capture] staged connector zip: ${stagedRes.status}`);
let stagedId = staged?.id ?? null;
if (stagedId === null) {
  // Digest-dedup from an earlier run of this same script: find the row.
  const stagedList = await json(await auth("/plugins/distribution/staged"));
  stagedId = stagedList?.find((s) => s.manifest?.name === manifest.name)?.id ?? null;
}
rmSync(stageDir, { recursive: true, force: true });

// --- browser -------------------------------------------------------------------
const browser = await chromium.launch();
let shots = 0;
const shot = async (page, name) => {
  await page.screenshot({ path: `${OUT}/${name}.png` });
  shots += 1;
  console.log(`  [shot] ${name}`);
};

/** Login with the durable-gate retry the e2e helpers use. */
const loginWithRetry = async (page) => {
  for (let attempt = 0; attempt < 4; attempt += 1) {
    await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
    const submit = page.locator('button[type="submit"]');
    await submit.waitFor({ state: "visible", timeout: 30_000 });
    await page.waitForTimeout(1500);
    await page.fill('input[type="email"]', EMAIL);
    await page.fill('input[type="password"]', PASSWORD);
    await submit.click();
    try {
      await page.waitForURL(/tickets|today|dashboard/, { timeout: 45_000 });
      return;
    } catch {
      await page.waitForTimeout(5000);
    }
  }
  throw new Error("login failed after retries (auth gate may be throttling)");
};

const dismissTour = async (page) => {
  for (let step = 0; step < 10; step += 1) {
    const overlay = page.locator('[data-testid="tour-overlay"]');
    if (!(await overlay.isVisible().catch(() => false))) return;
    for (const name of [/don't show me tours/i, /^done$/i, /next/i]) {
      const target = page.getByRole("button", { name }).first();
      if (await target.isVisible().catch(() => false)) {
        await target.click({ timeout: 3000 }).catch(() => {});
        break;
      }
    }
    await page.waitForTimeout(400);
  }
  await page.keyboard.press("Escape").catch(() => {});
};

/** Navigate to a staff route, re-authenticating if the client-side guard
 * bounces to /login (the guard can fire before the cookie session
 * resolves, and the durable auth gate can be armed by an earlier run). */
const gotoStaff = async (page, path, marker) => {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await page.goto(`${WEB}${path}`, { waitUntil: "domcontentloaded" }).catch(() => {});
    await page.waitForTimeout(2500);
    if (!page.url().includes("/login")) {
      if (marker) {
        await page
          .locator(marker)
          .first()
          .waitFor({ state: "visible", timeout: 25_000 })
          .catch(() => {});
      }
      await page.waitForTimeout(600);
      return;
    }
    await loginWithRetry(page);
  }
  throw new Error(`could not reach ${path} (repeatedly bounced to /login)`);
};

for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({
    colorScheme: theme,
    viewport: { width: 1440, height: 1000 },
    reducedMotion: theme === "dark" ? "reduce" : "no-preference",
  });
  const page = await ctx.newPage();
  page.on("pageerror", (err) => console.error(`[pageerror ${theme}]`, err.message));
  await loginWithRetry(page);
  await gotoStaff(page, "/settings/integrations", "text=Event types");

  // --- A: the outbound create form + the registry refusal ---------------------
  await page.waitForTimeout(800);
  await shot(page, `${theme}-01-outbound-multiselect`);

  // Type an ungranted type, add it, submit — the 422 refusal names it.
  await page.getByLabel("Custom event type").fill("acme.notgranted");
  await page.getByRole("button", { name: "Add type" }).click();
  await page.waitForTimeout(300);
  await shot(page, `${theme}-02-registry-refusal-form`);
  await page.getByLabel("Endpoint URL").fill("https://hooks.capture.example.test/marula");
  await page.getByRole("button", { name: "Create webhook" }).click();
  const refusal = page.getByRole("alert").filter({ hasText: /registry refused/i });
  await refusal.first().waitFor({ state: "visible", timeout: 20_000 }).catch(() => {});
  await page.waitForTimeout(400);
  await shot(page, `${theme}-03-registry-refusal-copy`);

  // --- B: the inbound surface --------------------------------------------------
  await page.getByTestId("register-source").click();
  await page.waitForTimeout(600);
  await shot(page, `${theme}-04-register-source-dialog`);
  // Close before touching the page behind the scrim.
  await page.keyboard.press("Escape");
  await page.waitForTimeout(500);

  const createdCard = page.locator("div").filter({ hasText: new RegExp(sourceRef) }).last();
  await createdCard.scrollIntoViewIfNeeded().catch(() => {});
  await page.waitForTimeout(500);
  await shot(page, `${theme}-05-inbound-source-card`);

  // Ingest log: M2M rows off the audit trail, then walk the older page.
  await page.getByTestId(`ingest-log-${sourceId}`).click();
  await page.getByText("emitted").first().waitFor({ timeout: 25_000 }).catch(() => {});
  await page.waitForTimeout(600);
  await shot(page, `${theme}-06-m2m-ingest-log`);

  const older = page.getByRole("button", { name: "Load older" });
  if (await older.isVisible().catch(() => false)) {
    await older.click();
    await page.waitForTimeout(1200);
  }
  await shot(page, `${theme}-07-ingest-log-older-page`);

  // The edit dialog for the same source.
  const editButtons = page.getByRole("button", { name: "Edit" });
  await editButtons.last().click();
  await page.waitForTimeout(400);
  await shot(page, `${theme}-08-edit-source-dialog`);
  await page.keyboard.press("Escape");

  // --- C: the automations editor with the board subject -----------------------
  await gotoStaff(page, "/automations", "text=Automations");
  await page.waitForTimeout(800);
  await shot(page, `${theme}-09-automations-ledger`);

  await page.getByRole("button", { name: /new rule/i }).first().click();
  await page.waitForTimeout(500);
  const subject = page.getByLabel("Rule subject");
  await subject.click();
  await page.getByRole("option", { name: /Board — fires on events with no ticket/ }).click();
  await page.waitForTimeout(400);
  await shot(page, `${theme}-10-rule-editor-board-subject`);
  await page.keyboard.press("Escape");

  // --- D: the consent screen with trigger + read-node blocks ------------------
  if (stagedId !== null) {
    await gotoStaff(page, "/settings/plugins", "text=Plugins");
    await page.getByRole("button", { name: "Approvals" }).click();
    await page.waitForTimeout(800);
    const row = page.getByTestId(`staged-row-${stagedId}`);
    if (await row.isVisible().catch(() => false)) {
      await row.getByRole("button", { name: "Review consent" }).click();
      await page.getByText("Automation triggers").first().waitFor({ timeout: 25_000 }).catch(() => {});
      await page.waitForTimeout(600);
      await shot(page, `${theme}-11-consent-trigger-read-surfaces`);
    } else {
      console.log(`  [skip] staged row ${stagedId} not visible (already decided?)`);
    }
  }

  await ctx.close();
}

// --- mobile + reduced-motion pass -------------------------------------------
{
  const ctx = await browser.newContext({
    colorScheme: "light",
    viewport: { width: 375, height: 780 },
  });
  const page = await ctx.newPage();
  await loginWithRetry(page);
  await gotoStaff(page, "/settings/integrations", "text=Event types");
  await shot(page, `mobile-01-integrations-375`);
  await gotoStaff(page, "/automations", "text=Automations");
  await shot(page, `mobile-02-automations-375`);
  await ctx.close();
}

await browser.close();
console.log(`[capture] done — ${shots} captures in ${OUT}`);
console.log(`[capture] staged source ${sourceId} / rule ${captureRuleId} left for the reviewer's reference`);
