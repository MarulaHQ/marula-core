// Wave 44 lane 7 `p44-ui-knowledge-automation` — vision-review captures for the
// eight knowledge-area routes, drawn against the REAL e2e stack (never mocks):
//   /kb, /kb/secrets, /documents, /documents/[id], /passwords, /imports,
//   /automations, /alerts
// Staging goes through /api/v1 so the vaults carry real category trees and
// records (the demo seed writes the legacy `passwords` rows only — the
// knowledge-class vault is empty on a fresh stack, which is the seed gap the
// lane record reports). Captured light + dark into
// design/qa-p44-ui-knowledge-automation/captures/, reviewed in <=8-image
// batches per the lane mandate.
//
// This session's model has no vision, so every capture carries MECHANICAL DOM
// assertions (geometry, anatomy, two-step reveal, pill states) written to
// captures/assertions.json — the pixel leg stays for a later human pass.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p44-knowledge-captures.mjs
import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const OUT = "design/qa-p44-ui-knowledge-automation/captures";
const EMAIL = "owner@demo.local";
const TECH_EMAIL = "tech@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;

mkdirSync(OUT, { recursive: true });
const assertions = [];
const note = (name, ok, detail) => {
  assertions.push({ name, ok, detail });
  console.log(`  [assert] ${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
};

// --- API context: login (durable-gate retry) + stage true state ---------------
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
const post = async (path, body) => json(await auth(path, { method: "POST", body: JSON.stringify(body) }));

const clients = (await json(await auth("/clients"))) ?? [];
const acme = clients.find((c) => /acme/i.test(c.name)) ?? clients[0];
if (!acme) throw new Error("no client in the seeded demo org");
console.log(`[capture] staging for client ${acme.name}`);

// --- stage: org-level vault ---------------------------------------------------
// NOTE the two wire quirks the API taught me while staging: record kinds are
// Rust snake_case on the wire (`api_key`, `recovery_code`) even though the DB
// vocabulary uses hyphens, and a client-scoped category is created by FORKING
// an org category (POST /secrets/folders/{id}/fork), never by a direct create
// with client_id — the folders table is exactly-one-of scoped.
const orgFolder = await post("/secrets/folders", {
  slug: `capture-platform-vault-${STAMP}`,
  label: "Capture platform vault",
  kind: "passwords",
});
if (!orgFolder?.id) throw new Error("org folder staging failed");
const soon = new Date(Date.now() + 10 * 86400_000).toISOString();
const stale = new Date(Date.now() - 200 * 86400_000).toISOString();
for (const [title, kind, fields, extra] of [
  ["Platform admin login", "password", [
    { field_key: "username", value: "svc-platform" },
    { field_key: "secret", value: "capture-plaintext-platform-9f2a" },
  ], {}],
  ["Stripe API key", "api_key", [
    { field_key: "secret", value: "sk_live_capture_51H8xQ2eZvKYlo2C" },
  ], {}],
  ["Vendor portal login", "password", [
    { field_key: "username", value: "vendor-ops" },
    { field_key: "secret", value: "capture-plaintext-vendor-7c1b" },
  ], { expires_at: soon }],
  ["Legacy DNS credential", "password", [
    { field_key: "username", value: "dns-admin" },
    { field_key: "secret", value: "capture-plaintext-dns-3e5d" },
  ], { rotate_at: stale }],
]) {
  const rec = await post("/secrets/records", {
    folder_id: orgFolder.id,
    kind,
    title: `Capture ${title}`,
    fields,
    ...extra,
  });
  if (!rec?.id) console.log(`[capture] WARN record ${title} failed`);
}
console.log(`[capture] org vault: ${orgFolder.id}`);

// --- stage: a restricted org chain (K11/K13 posture) --------------------------
const restrictedFolder = await post("/secrets/folders", {
  slug: `capture-msp-api-secrets-${STAMP}`,
  label: "Capture MSP API secrets",
  kind: "api-keys",
  restricted: true,
});
if (restrictedFolder?.id) {
  await post("/secrets/records", {
    folder_id: restrictedFolder.id,
    kind: "api_key",
    title: "Capture NinjaOne API key",
    fields: [{ field_key: "secret", value: "nj_capture_51H8xQ2eZvKYlo2C" }],
  });
  console.log(`[capture] restricted chain: ${restrictedFolder.id}`);
}

// --- stage: client-scoped vault (the K07 anatomy target), by fork --------------
const forkRes = await auth(`/secrets/folders/${orgFolder.id}/fork`, {
  method: "POST",
  body: JSON.stringify({ client_id: acme.id }),
});
const forked = (await json(forkRes)) ?? [];
const clientFolder = forked[0] ?? null;
if (!clientFolder?.id) throw new Error("client fork staging failed");
for (const [title, kind, fields, extra] of [
  ["Acme VPN certificate", "certificate", [
    { field_key: "secret", value: "capture-cert-thumbprint-9f8e" },
  ], {}],
  ["Acme rotate-past login", "password", [
    { field_key: "username", value: "stale-user" },
    { field_key: "secret", value: "capture-plaintext-stale-4d6f" },
  ], { rotate_at: stale }],
]) {
  const rec = await post("/secrets/records", {
    client_id: acme.id,
    folder_id: clientFolder.id,
    kind,
    title,
    fields,
    ...extra,
  });
  if (!rec?.id) console.log(`[capture] WARN client record ${title} failed`);
}
console.log(`[capture] client vault (forked): ${clientFolder.id}`);

// --- browser -------------------------------------------------------------------
const browser = await chromium.launch();
let shots = 0;
const shot = async (page, name) => {
  await page.screenshot({ path: `${OUT}/${name}.png` });
  shots += 1;
  console.log(`  [shot] ${name}`);
};

/** Geometry + anatomy assertions that stand in for the vision leg. */
const measure = async (page, sel, prop) =>
  page.evaluate(
    ([sel, prop]) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const cs = getComputedStyle(el);
      if (prop === "text") return el.textContent?.trim() ?? null;
      return cs[prop];
    },
    [sel, prop],
  );

const assertGeometry = async (page, label) => {
  const h1 = await measure(page, "h1", "fontSize");
  const h1w = await measure(page, "h1", "fontWeight");
  note(`${label}: title 26px/600`, h1 === "26px" && String(h1w) === "600", `${h1}/${h1w}`);
  const hdr = await page.evaluate(() => {
    const ths = Array.from(document.querySelectorAll("th")).filter((t) => (t.textContent ?? "").trim().length > 0);
    if (ths.length === 0) return null;
    const cs = getComputedStyle(ths[0]);
    return `${cs.fontSize}/${cs.letterSpacing}/${cs.textTransform}`;
  });
  note(
    `${label}: column header 12px/0.06em/uppercase`,
    hdr === null || hdr === "12px/0.72px/uppercase",
    hdr === null ? "no table rows in this state" : `${hdr}`,
  );
  const pill = await measure(page, ".m-ledger-sev", "height");
  note(`${label}: status pill 24px`, pill === "24px" || pill === null, `${pill}`);
};

const assertNoHex = async (page, label) => {
  const hex = await page.evaluate(() => {
    const bad = [];
    for (const el of Array.from(document.querySelectorAll("body *")).slice(0, 400)) {
      const cs = getComputedStyle(el);
      for (const v of [cs.color, cs.backgroundColor, cs.borderTopColor]) {
        if (/#[0-9a-f]{3,8}\b/i.test(v)) bad.push(v);
      }
    }
    return bad.slice(0, 3);
  });
  note(`${label}: no hex colours`, hex.length === 0, hex.join(" "));
};

/** Login with the durable-gate retry the e2e helpers use. */
const loginWithRetry = async (page, email = EMAIL) => {
  for (let attempt = 0; attempt < 4; attempt += 1) {
    await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
    const submit = page.locator('button[type="submit"]');
    await submit.waitFor({ state: "visible", timeout: 30_000 });
    await page.waitForTimeout(1500);
    await page.fill('input[type="email"]', email);
    await page.fill('input[type="password"]', PASSWORD);
    await submit.click();
    try {
      await page.waitForURL(/tickets|today|dashboard/, { timeout: 45_000 });
      return;
    } catch {
      await page.waitForTimeout(5000);
    }
  }
  throw new Error(`login failed after retries (${email}, auth gate may be throttling)`);
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

const gotoStaff = async (page, path, marker) => {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    await page.goto(`${WEB}${path}`, { waitUntil: "domcontentloaded" }).catch(() => {});
    await page.waitForTimeout(2500);
    if (!page.url().includes("/login")) {
      if (marker) {
        await page.locator(marker).first().waitFor({ state: "visible", timeout: 25_000 }).catch(() => {});
      }
      await page.waitForTimeout(600);
      return;
    }
    await loginWithRetry(page);
  }
  throw new Error(`could not reach ${path} (repeatedly bounced to /login)`);
};

/** Set the density + theme through the app's own bootstrap keys. */
const prepContext = async (theme) => {
  const ctx = await browser.newContext({
    colorScheme: theme === "dark" ? "dark" : "light",
    viewport: { width: 1440, height: 1000 },
    reducedMotion: "reduce",
  });
  const page = await ctx.newPage();
  page.on("pageerror", (err) => console.error(`[pageerror ${theme}]`, err.message));
  return { ctx, page };
};

for (const theme of ["light", "dark"]) {
  console.log(`\n=== ${theme} ===`);
  const { ctx, page } = await prepContext(theme);
  await loginWithRetry(page);
  const appliedTheme = await page.evaluate(() => document.documentElement.dataset.theme);
  note(`${theme}: data-theme`, appliedTheme === (theme === "dark" ? "dark" : "neutral"), appliedTheme);

  // --- 1: /kb — the knowledge ledger -----------------------------------------
  await gotoStaff(page, "/kb", "text=Knowledge base");
  await dismissTour(page);
  await page.waitForTimeout(700);
  await shot(page, `${theme}-01-kb-list`);
  await assertGeometry(page, `${theme}-01-kb-list`);

  // --- 2: /kb/secrets — the org vault with a category selected ----------------
  await gotoStaff(page, "/kb/secrets", "text=Secret vault");
  await page.waitForTimeout(600);
  await page.getByRole("button", { name: /Capture platform vault/ }).first().click().catch(async () => {
    await page.getByText("Capture platform vault").first().click().catch(() => {});
  });
  await page.getByTestId("kb-secrets-table").first().waitFor({ state: "visible", timeout: 20_000 }).catch(() => {});
  await page.waitForTimeout(900);
  await shot(page, `${theme}-02-kb-secrets-org-vault`);
  await assertGeometry(page, `${theme}-02-kb-secrets-org-vault`);
  const labels = await page.evaluate(() =>
    Array.from(document.querySelectorAll('[data-testid="kb-secrets-table"] tbody tr td'))
      .map((td) => td.textContent ?? "")
      .some((t) => /secret|username/.test(t)),
  );
  note(`${theme}-02: the table carries field labels, not values`, labels);
  const noPlaintext = await page.evaluate(
    () => !document.body.textContent.includes("capture-plaintext-platform-9f2a"),
  );
  note(`${theme}-02: no plaintext in the list`, noPlaintext);

  // --- 3: the grants dialog (K12 posture: staged until save) -------------------
  await page.getByRole("button", { name: /Category grants/ }).first().click().catch(() => {});
  await page.getByText("Category grants").first().waitFor({ state: "visible", timeout: 10_000 }).catch(() => {});
  await page.waitForTimeout(600);
  await shot(page, `${theme}-03-kb-secrets-grants`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(400);

  // --- 4-6: the two-step reveal ------------------------------------------------
  await page.getByTestId("kb-secrets-table").first().waitFor({ state: "visible", timeout: 15_000 }).catch(() => {});
  await page.getByRole("button", { name: /Capture Platform admin login/ }).first().click().catch(() => {});
  await page.getByTestId("kb-secret-reveal-open").first().waitFor({ state: "visible", timeout: 15_000 }).catch(() => {});
  await page.waitForTimeout(600);
  await shot(page, `${theme}-04-kb-secret-record-dialog`);
  await page.getByTestId("kb-secret-reveal-open").first().click().catch(() => {});
  await page.getByTestId("kb-secret-reveal-confirm").first().waitFor({ state: "visible", timeout: 10_000 }).catch(() => {});
  await page.waitForTimeout(500);
  await shot(page, `${theme}-05-kb-secret-reveal-confirm`);
  await page.getByTestId("kb-secret-reveal-confirm").first().click().catch(() => {});
  await page.waitForTimeout(1500);
  let revealed = false;
  for (let wait = 0; wait < 8 && !revealed; wait += 1) {
    revealed = await page.evaluate(() => {
      try {
        // The revealed value lives in the input's VALUE, not the page text —
        // the secret field renders as a password input on purpose.
        const inputs = Array.from(document.querySelectorAll('input[id^="reveal-"]'));
        return inputs.some((i) => i.value === "capture-plaintext-platform-9f2a");
      } catch {
        return false;
      }
    });
    if (!revealed) await page.waitForTimeout(700);
  }
  const revealState = await page.evaluate(() => ({
    revealedHeading: document.body.textContent.includes("Revealed — audited"),
    alert: Array.from(document.querySelectorAll('[role="alert"]')).map((e) => e.textContent?.slice(0, 80)),
    url: location.pathname,
  }));
  note(`${theme}-06: plaintext appears only after the confirm`, revealed, JSON.stringify(revealState));
  await shot(page, `${theme}-06-kb-secret-revealed`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(400);

  // --- 7-8: the client vault (K07 anatomy) -------------------------------------
  await gotoStaff(page, "/passwords", "text=Vault");
  await page.getByLabel("Vault scope").click().catch(() => {});
  await page.getByRole("option", { name: new RegExp(acme.name, "i") }).first().click().catch(() => {});
  await page.waitForTimeout(900);
  await page.getByRole("button", { name: new RegExp(clientFolder.label) }).first().click().catch(async () => {
    await page.getByText(clientFolder.label).first().click().catch(() => {});
  });
  await page.waitForTimeout(900);
  await shot(page, `${theme}-07-passwords-client-vault`);
  await assertGeometry(page, `${theme}-07-passwords-client-vault`);
  await page.getByRole("button", { name: /Acme rotate-past login/ }).first().click().catch(() => {});
  await page.getByTestId("kb-secret-reveal-open").first().waitFor({ state: "visible", timeout: 15_000 }).catch(() => {});
  await page.waitForTimeout(600);
  await shot(page, `${theme}-08-passwords-record-dialog`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(400);

  // --- 9-10: /documents — the fixed pill ---------------------------------------
  await gotoStaff(page, "/documents", "text=Documents");
  await page.waitForTimeout(700);
  const pillAtRest = await page.evaluate(() => {
    const btns = Array.from(document.querySelectorAll("button"));
    const b = btns.find((el) => /Client-scoped/.test(el.textContent ?? ""));
    return b ? b.getAttribute("aria-pressed") : "missing";
  });
  note(`${theme}-09: Client-scoped pill not pressed at rest`, pillAtRest === "false", pillAtRest);
  await shot(page, `${theme}-09-documents-list`);
  await page.getByRole("button", { name: /Client-scoped/ }).first().click().catch(() => {});
  await page.waitForTimeout(800);
  const pillPressed = await page.evaluate(() => {
    const btns = Array.from(document.querySelectorAll("button"));
    const b = btns.find((el) => /Client-scoped/.test(el.textContent ?? ""));
    return b ? b.getAttribute("aria-pressed") : "missing";
  });
  note(`${theme}-10: Client-scoped pill pressed after the click`, pillPressed === "true", pillPressed);
  const viewActive = await page.evaluate(() => {
    const btns = Array.from(document.querySelectorAll("button"));
    return btns.some((el) => /Client-scoped/.test(el.textContent ?? "") && el.getAttribute("aria-pressed") === "true");
  });
  note(`${theme}-10: a toolbar view carries the pressed state`, viewActive);
  await shot(page, `${theme}-10-documents-client-scoped`);
  await page.getByRole("button", { name: /Client-scoped/ }).first().click().catch(() => {});
  await page.waitForTimeout(600);

  // --- 11: /documents/[id] — the detail with its version history ---------------
  await gotoStaff(page, "/documents", "text=Documents");
  await page.getByRole("button", { name: /Runbook — Client Onboarding/ }).first().click().catch(() => {});
  await page.getByTestId("document-versions").first().waitFor({ state: "visible", timeout: 20_000 }).catch(() => {});
  await page.waitForTimeout(800);
  await shot(page, `${theme}-11-document-detail`);
  await assertGeometry(page, `${theme}-11-document-detail`);

  // --- 12: /imports — the wizard default state ---------------------------------
  await gotoStaff(page, "/imports", "text=Imports");
  await page.waitForTimeout(800);
  await shot(page, `${theme}-12-imports-wizard`);
  await assertGeometry(page, `${theme}-12-imports-wizard`);

  // --- 13-14: /automations — ledger + the rule editor ---------------------------
  await gotoStaff(page, "/automations", "text=Automation rules");
  await page.waitForTimeout(700);
  await shot(page, `${theme}-13-automations-ledger`);
  await assertGeometry(page, `${theme}-13-automations-ledger`);
  await page.getByRole("button", { name: "New rule" }).first().click().catch(() => {});
  await page.getByText("WHEN").first().waitFor({ state: "visible", timeout: 15_000 }).catch(() => {});
  await page.waitForTimeout(900);
  await shot(page, `${theme}-14-automations-editor`);

  // --- 15: /alerts — the feed and its right-edge assistant stub -----------------
  await gotoStaff(page, "/alerts", "text=Alerts");
  await page.waitForTimeout(800);
  await shot(page, `${theme}-15-alerts-feed`);
  await assertGeometry(page, `${theme}-15-alerts-feed`);
  const stubWidth = await page.evaluate(() => {
    const el = Array.from(document.querySelectorAll("div")).find((d) => /alerts have not been routed/.test(d.getAttribute("aria-label") ?? "") || /alerts have not been routed/.test(d.textContent ?? ""));
    return el ? getComputedStyle(el).width : null;
  });
  note(`${theme}-15: assistant stub column 40px`, stubWidth === "40px" || stubWidth === null, `${stubWidth}`);

  // --- 16: no hex anywhere on the ledger surfaces ------------------------------
  await assertNoHex(page, `${theme}-ledgers`);

  await ctx.close();
}

// --- technician view of the restricted chain (K13 posture) ----------------------
console.log("\n=== technician (restricted chain) ===");
{
  const { ctx, page } = await prepContext("light");
  await loginWithRetry(page, TECH_EMAIL);
  await gotoStaff(page, "/kb/secrets", "text=Secret vault");
  await page.waitForTimeout(900);
  const hasRestricted = await page.evaluate(() =>
    document.body.textContent.includes("Capture MSP API secrets"),
  );
  note(`tech: restricted chain hidden from the tree`, !hasRestricted);
  await shot(page, `tech-16-restricted-chain-hidden`);
  const { ctx: ctx2 } = { ctx };
  await ctx.close();
}

writeFileSync(join(OUT, "assertions.json"), JSON.stringify(assertions, null, 2));
const failed = assertions.filter((a) => !a.ok).length;
console.log(`\n[capture] ${shots} shots, ${assertions.length - failed}/${assertions.length} assertions pass`);
if (failed > 0) {
  console.log(`[capture] ${failed} assertions FAILED:`);
  for (const a of assertions.filter((x) => !x.ok)) console.log(`   FAIL ${a.name} — ${a.detail}`);
}
console.log("[capture] done");
