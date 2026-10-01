// Wave 41 lane 10 (`p41-ui-knowledge`) — state captures for the knowledge
// cluster, in BOTH themes.
//
// Purpose: hand the vision review a complete, honest picture of every state
// the brief names for this lane, so the review compares the same list the
// launch prompt wrote down:
//
//   kb            list · selection + bulk bar · publish-state row (portal on
//                 and portal-blocked) · publish visibility mismatch
//   kb/secrets    category rail · record ledger · reveal CONFIRMATION open
//   documents     list · type manager · invalid-JSON schema editor (blush)
//   documents/[id] record + version history ledger
//   passwords     vault ledger (categories and credentials views) · reveal
//                 CONFIRMATION open
//   imports       wizard step 1 (source) · step 2 (upload) · run history
//   automations   rule ledger · builder WHEN/IF/THEN · inline suggested
//                 condition (P3 — a mark in the row, never a panel)
//   alerts        feed with the COLLAPSED 40px assistant stub · acknowledged
//
// Non-purpose: no mocks and no fabricated rows. Every capture is driven
// against the live e2e stack and staged through /api/v1 first, so a shot shows
// a true state (the pack's rule) rather than a hand-built lookalike. States
// that need a destructive or paid action — publishing an article, rotating a
// secret — are staged with their real endpoints before the shot.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a
//   node e2e/p41-knowledge-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = process.env.CAPTURE_OUT ?? "design/qa-p41-ui-knowledge/captures";
const THEMES = (process.env.CAPTURE_THEMES ?? "light,dark").split(",");
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;

mkdirSync(OUT, { recursive: true });

const ctx0 = await (await import("playwright")).request.newContext();
const login = await ctx0.post(`${API}/api/v1/auth/login`, {
  data: { email: EMAIL, password: PASSWORD },
});
if (!login.ok()) throw new Error(`login failed: ${login.status()}`);
const tokens = await login.json();

async function api(path, method, body) {
  const res = await ctx0.fetch(`${API}/api/v1${path}`, {
    method,
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${tokens.access_token}`,
    },
    data: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok()) throw new Error(`${method} ${path} → ${res.status()}: ${await res.text()}`);
  return res.headers()["content-type"]?.includes("json") ? res.json() : undefined;
}

// --- scratch data -----------------------------------------------------------
const key = crypto.randomUUID();

const catClient = await api("/kb/categories", "POST", {
  name: `Capture runbook ${STAMP}`,
  description: "Client-facing how-tos",
  client_visible: true,
  position: 0,
  idempotency_key: key,
});
const catStaff = await api("/kb/categories", "POST", {
  name: `Capture internals ${STAMP}`,
  description: "Staff-only notes",
  client_visible: false,
  position: 1,
  idempotency_key: crypto.randomUUID(),
});

const articleLive = await api("/kb/articles", "POST", {
  title: `Capture reset the printer on an MFP ${STAMP}`,
  slug: `capture-printer-${STAMP}`,
  body: "Power cycle, then run the self-test.",
  status: "published",
  category_id: catClient.id,
  client_visible: true,
  idempotency_key: crypto.randomUUID(),
});
// The same shape but with a staff-only category: the honest "client-facing
// but not in the portal" row the brief asks the ledger to be able to say.
const articleBlocked = await api("/kb/articles", "POST", {
  title: `Capture rotate the service credential ${STAMP}`,
  slug: `capture-rotate-${STAMP}`,
  body: "Quarterly rotation, documented.",
  status: "published",
  category_id: catStaff.id,
  client_visible: true,
  idempotency_key: crypto.randomUUID(),
});
await api("/kb/articles", "POST", {
  title: `Capture draft: swap the toner drum ${STAMP}`,
  slug: `capture-draft-${STAMP}`,
  body: "Still being written.",
  status: "draft",
  category_id: catClient.id,
  client_visible: false,
  idempotency_key: crypto.randomUUID(),
});

const folder = await api("/secrets/folders", "POST", {
  slug: `capture-vault-${STAMP}`,
  label: `Capture vault ${STAMP}`,
  kind: "api_keys",
  parent_id: null,
  restricted: false,
  client_id: null,
});
await api("/secrets/records", "POST", {
  folder_id: folder.id,
  title: `Capture billing export key ${STAMP}`,
  kind: "api_key",
  fields: [
    { field_key: "secret", value: "sk-live-capture-0000000000" },
    { field_key: "username", value: "svc-billing" },
  ],
  expires_at: new Date(Date.now() + 45 * 86_400_000).toISOString(),
  client_id: null,
  idempotency_key: crypto.randomUUID(),
});
await api("/secrets/records", "POST", {
  folder_id: folder.id,
  title: `Capture expired monitor token ${STAMP}`,
  kind: "api_key",
  fields: [{ field_key: "secret", value: "tok-capture-expired" }],
  expires_at: new Date(Date.now() - 3 * 86_400_000).toISOString(),
  client_id: null,
  idempotency_key: crypto.randomUUID(),
});

const layout = await api("/documents/layouts", "POST", {
  name: `Capture runbook type ${STAMP}`,
  description: "Structured runbook fields",
  fields: [
    { key: "model", label: "Model", type: "text", position: 0 },
    { key: "minutes", label: "Est. minutes", type: "number", position: 1 },
  ],
  idempotency_key: crypto.randomUUID(),
});
const doc = await api("/documents", "POST", {
  title: `Capture printer reset runbook ${STAMP}`,
  body: "1. Power cycle.\n2. Run the self-test.\n3. Log the result.",
  client_id: null,
  layout_id: layout.id,
  fields: { model: "MFP-880", minutes: 12 },
  change_summary: "Initial version",
  idempotency_key: crypto.randomUUID(),
});
await api("/documents", "POST", {
  title: `Capture revised printer runbook ${STAMP}`,
  body: "1. Power cycle twice.\n2. Run the self-test.",
  client_id: null,
  layout_id: layout.id,
  fields: { model: "MFP-880", minutes: 15 },
  change_summary: "Added the second power cycle",
  idempotency_key: crypto.randomUUID(),
});

await api("/automation-rules", "POST", {
  name: `Capture escalate unassigned P1 ${STAMP}`,
  board_id: null,
  trigger_event: "ticket.created",
  conditions: [{ type: "priority_is", priority: "p1" }],
  actions: [{ type: "set_priority", priority: "p1" }],
});
// A rule with NO conditions on purpose: the row then carries the P3
// suggested-condition mark, which is the state the review must see.
// A rule with NO conditions on purpose: the row then carries the P3
// suggested-condition mark, which is the state the review must see.
await api("/automation-rules", "POST", {
  name: `Capture untriaged arrivals ${STAMP}`,
  board_id: null,
  trigger_event: "ticket.created",
  conditions: [],
  actions: [{ type: "set_priority", priority: "p3" }],
});

// The alert feed is shot from the seeded org: seed-demo creates one source,
// five alerts and two incident tickets, which is exactly the mix the feed has
// to cope with (open, acknowledged, routed and unrouted). Signing a fresh
// ingest here would only reproduce it with less truth in it.

let shotIndex = 0;
const failures = [];

for (const [themeIndex, theme] of THEMES.entries()) {
  const themeStamp = `${STAMP}-${themeIndex}`;
  const browser = await chromium.launch({ args: ["--no-proxy-server"] });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  {
    let ok = false;
    for (let attempt = 0; attempt < 20 && !ok; attempt += 1) {
      const res = await ctx.request.post(`${API}/api/v1/auth/login`, {
        data: { email: EMAIL, password: PASSWORD },
      });
      ok = res.ok();
      if (!ok) await new Promise((resolve) => setTimeout(resolve, 3000));
    }
    if (!ok) throw new Error("context login failed");
  }
  const page = await ctx.newPage();
  await page.addInitScript(
    ([apiBase, tokenPair, mode]) => {
      window.__MARULA_API_URL__ = apiBase;
      window.localStorage.setItem("marula-tokens", tokenPair);
      window.localStorage.setItem("marula-tokens-exp", String(Date.now() + 3_600_000));
      window.localStorage.setItem("marula-theme", mode);
      // Comfortable is Marula's own density step; capture it as such.
      window.localStorage.setItem("marula-density", "comfortable");
      window.sessionStorage.setItem("marula-tour-session", "1");
    },
    [API, JSON.stringify(tokens), theme],
  );

  async function shot(name, { fullPage = false } = {}) {
    shotIndex += 1;
    await page.waitForTimeout(400);
    await page.screenshot({
      path: `${OUT}/${String(shotIndex).padStart(2, "0")}-${name}-${theme}.png`,
      fullPage,
    });
  }

  try {
    // --- /kb: list with the portal-gate row ---------------------------
    await page.goto(`${WEB}/kb`, { waitUntil: "load" });
    await page.getByTestId("kb-article-table").waitFor({ timeout: 30_000 });
    await shot("kb-list");

    // Selection + the bulk bar that replaces the header row.
    await page.getByLabel(`Select ${articleLive.title}`).click();
    await page.getByLabel(`Select ${articleBlocked.title}`).click();
    await shot("kb-selection-bulk");

    // The publish state row: open the article editor.
    await page.getByRole("button", { name: articleLive.title }).click();
    await page.keyboard.press("Escape");
    await page.getByTestId("kb-new-article").click();
    const titleInput = page.getByTestId("kb-new-title");
    await titleInput.fill(`Capture publish-state walkthrough ${themeStamp}`);
    await page.getByTestId("kb-create-article").click();
    await page.getByTestId("kb-publish-row").waitFor({ timeout: 30_000 });
    await shot("kb-editor-publish-state");
    await page.keyboard.press("Escape");

    // --- /kb/secrets: rail, ledger, reveal confirmation ----------------
    await page.goto(`${WEB}/kb/secrets`, { waitUntil: "load" });
    await page.getByText(`Capture vault ${STAMP}`).first().click();
    await page.getByTestId("kb-secrets-table").waitFor({ timeout: 30_000 });
    await shot("kb-secrets-ledger");
    await page.getByRole("button", { name: `Capture billing export key ${STAMP}` }).click();
    await page.getByTestId("kb-secret-reveal-open").click();
    await page.getByTestId("kb-secret-reveal-confirm").waitFor({ timeout: 30_000 });
    await shot("kb-secrets-reveal-confirm");
    await page.keyboard.press("Escape");

    // --- /documents: list, type manager, invalid JSON -----------------
    await page.goto(`${WEB}/documents`, { waitUntil: "load" });
    await page.getByTestId("documents-table").waitFor({ timeout: 30_000 });
    await shot("documents-list");
    await page.getByTestId("documents-manage-types").click();
    await page.getByRole("dialog").waitFor({ timeout: 30_000 });
    await shot("documents-type-manager");
    const editButtons = page.getByRole("button", { name: "Edit" });
    await editButtons.last().click();
    const schema = page.locator("#layout-fields");
    await schema.fill('[{"key": "model", "label": "Model"');
    await page.getByTestId("documents-invalid-schema").waitFor({ timeout: 30_000 });
    await shot("documents-invalid-json-blush");
    await page.keyboard.press("Escape");

    // --- /documents/[id]: the record and its version ledger -----------
    await page.goto(`${WEB}/documents/${doc.id}`, { waitUntil: "load" });
    await page.getByTestId("document-versions").waitFor({ timeout: 30_000 });
    await shot("document-detail-versions");

    // --- /passwords: the vault, both views, reveal confirmation -------
    await page.goto(`${WEB}/passwords`, { waitUntil: "load" });
    await page.getByTestId("context-column").first().waitFor({ timeout: 30_000 });
    await shot("passwords-vault");
    await page.getByRole("tab", { name: /Credentials/ }).click();
    await page.getByTestId("vault-credentials-table").waitFor({ timeout: 30_000 });
    await shot("passwords-credentials-view");
    const reveal = page.locator('[data-testid^="vault-reveal-"]:not([data-testid="vault-reveal-confirm"])').first();
    await reveal.click();
    await page.getByTestId("vault-reveal-confirm").waitFor({ timeout: 30_000 });
    await shot("passwords-reveal-confirm");
    await page.keyboard.press("Escape");

    // --- /imports: the wizard's card stack, step by step -------------
    await page.goto(`${WEB}/imports`, { waitUntil: "load" });
    await page.getByRole("heading", { name: "Source" }).waitFor({ timeout: 30_000 });
    await shot("imports-step-source");
    await page.getByRole("button", { name: "Next · upload" }).click();
    await page.getByLabel("Paste CSV").fill(
      "external_key,name\nacme-corp,Northwind Traders\ncontoso-ltd,Contoso Ltd",
    );
    await shot("imports-step-upload");
    await page.getByTestId("imports-dry-run").click();
    await page.getByTestId("imports-report-step").waitFor({ timeout: 60_000 });
    await shot("imports-dry-run-report");
    await page.getByRole("button", { name: "Next · mapping" }).click();
    await shot("imports-step-mapping");
    await page.getByRole("button", { name: "Next · commit" }).click();
    await shot("imports-step-commit");

    // --- /automations: ledger, builder, the inline suggestion ---------
    await page.goto(`${WEB}/automations`, { waitUntil: "load" });
    await page.getByTestId("automation-rules-table").waitFor({ timeout: 30_000 });
    await shot("automations-rules-ledger");
    await page.getByTestId("automation-new-rule").click();
    await page.locator("#ar-name").fill(`Capture rule walkthrough ${STAMP}`);
    // The save stays disabled until the rule has a name and an action — the
    // capture is of the builder, so it is left unsaved on purpose.
    await shot("automations-builder-suggestion");
    await page.keyboard.press("Escape");

    // --- /alerts: the collapsed 40px stub, then an acknowledged row --
    await page.goto(`${WEB}/alerts`, { waitUntil: "load" });
    await page.getByTestId("assistant-stub").waitFor({ timeout: 30_000 });
    await shot("alerts-feed-assistant-stub");
    const ack = page.locator('[data-testid^="alert-ack-"]').first();
    if ((await ack.count()) > 0) {
      await ack.click();
      await page.getByTestId("alerts-table").waitFor({ timeout: 30_000 });
      await shot("alerts-acknowledged");
    }
  } catch (error) {
    failures.push({ theme, message: error instanceof Error ? error.message : String(error) });
  }

  await browser.close();
}

console.log(`captures written to ${OUT} (${shotIndex} shots)`);
if (failures.length > 0) {
  for (const failure of failures) {
    console.error(`capture failed [${failure.theme}]: ${failure.message}`);
  }
  process.exitCode = 1;
}
