// Wave 43 lane 18 `p43-workflow-closeout-ui` — vision-review captures for the
// workflow close-out surfaces, drawn against the REAL e2e stack (never mocks):
//
//   A  /automations — the header (Rules/Canvas toggle, Runs, Webhook activity)
//      and the per-rule "Runs" link into that workflow's history.
//   B  /automations with the editor open — the registry-driven trigger picker,
//      grouped by trigger node, on both subjects (ticket, board).
//   C  /automations/runs — the ledger, populated with REAL runs.
//   D  /automations/runs?status=failed — the filter row narrowed.
//   E  /automations/runs?rule=<idle> — a workflow that has never run (the
//      never-run empty state, which is not the same sentence as "no runs").
//   F  /automations/runs/<id> — a failed run that KEEPS a prefix: the blush
//      banner, the trace with its failed step selected, the two retry verbs.
//   G  /automations/runs/<id> — a failed run that kept nothing ("No step
//      completed."), for the other banner sentence.
//   H  a parked run (the gold banner) and a succeeded run.
//   I  a retry-minted run — the "This run retries <id>" line.
//   J  /automations/runs/<unknown> — the not-found surface.
//   K  the retry outcome: the resume verb clicked against the failed run, so the
//      UI's typed 409 refusal renders.
//
// Captured light + dark, plus a 375px mobile pass.
//
// REAL RUNS, NOT FIXTURES (stated plainly): the sanctioned entry point
// (`scripts/e2e-stack.sh up`) boots core-api but NOT the workflow
// worker, so nothing executes and the ledger would be empty. This lane therefore
// boots the worker by hand against the same stack — the runbook block in
// REVIEW.md / GATES.md — and drives real executions through the public
// authoring path and the product's own retry route
// (`e2e/p43-workflow-closeout-drive.mjs`, whose report this script reads). Every
// row here is an execution the executor produced and the public run API
// returned. Nothing is inserted into the database.
//
// Run with the stack env exported, the worker up, and the bundle built:
//   set -a; . e2e/.e2e-stack.env; set +a
//   node e2e/p43-workflow-closeout-captures.mjs
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { chromium } from "playwright";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const OUT = "design/qa-p43-workflow-closeout-ui/captures";
const TRACES = "design/qa-p43-workflow-closeout-ui/traces";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

mkdirSync(OUT, { recursive: true });

const report = JSON.parse(readFileSync(`${TRACES}/drive-report.json`, "utf8"));
const run = (label) => {
  const r = report.runs[label];
  if (!r?.found) throw new Error(`drive report has no run for "${label}" — run the driver first`);
  return r.run_id;
};
const RUNS = {
  failedKept: run("mixed"),
  failedEmpty: run("bare"),
  parked: run("parked"),
  succeeded: run("green"),
  retryChild: run("retry_child"),
};
const NEVER_RUN_RULE = report.documents.idle.id;
console.log(
  `[capture] real runs: failed=${RUNS.failedKept} bare=${RUNS.failedEmpty} ` +
    `parked=${RUNS.parked} ok=${RUNS.succeeded} child=${RUNS.retryChild}`,
);

// --- API: login (durable-gate retry) ------------------------------------------
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

// --- browser -------------------------------------------------------------------
const browser = await chromium.launch();
let shots = 0;
const measurements = {};
const shot = async (page, name) => {
  await page.screenshot({ path: `${OUT}/${name}.png` });
  measurements[name] = await measure(page);
  shots += 1;
  console.log(`  [shot] ${name}`);
};

/**
 * What the page actually rendered, read from computed style and rects. Nulls are
 * honest: a shot with no banner reports `banner: null` rather than a default that
 * reads like a pass. Every value here is measured, never assumed.
 */
async function measure(page) {
  return page.evaluate(() => {
    const css = getComputedStyle(document.documentElement);
    const tok = (k) => css.getPropertyValue(k).trim();
    const el = (s) => document.querySelector(s);
    const dims = (n) =>
      n ? { h: Math.round(n.getBoundingClientRect().height), w: Math.round(n.getBoundingClientRect().width) } : null;

    const rgb = (s) => {
      const m = (s || "").match(/\d+(\.\d+)?/g);
      return m ? m.slice(0, 3).map(Number) : null;
    };
    const lum = (c) =>
      c
        .map((v) => {
          const x = v / 255;
          return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
        })
        .reduce((a, v, i) => a + v * [0.2126, 0.7152, 0.0722][i], 0);
    const contrast = (fg, bg) => {
      const a = rgb(fg);
      const b = rgb(bg);
      if (!a || !b) return null;
      const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
      return Math.round(((hi + 0.05) / (lo + 0.05)) * 100) / 100;
    };

    const row = el('[data-testid^="workflow-run-row-"]');
    const header = el('[data-testid="workflow-runs-table"] thead th');
    const cell = row?.querySelector("td") ?? null;
    const pill = row?.querySelector('[data-slot="pill"]') ?? el('[data-slot="pill"]');
    const pillStyle = pill ? getComputedStyle(pill) : null;
    const banner = el('[data-testid="run-banner"]');
    const bannerStyle = banner ? getComputedStyle(banner) : null;
    const refusal = el('[role="alert"] [data-slot="pill"], [role="alert"]');
    const label = el(".m-select-label");
    const labelStyle = label ? getComputedStyle(label) : null;
    const stepRow = el('[data-testid^="run-step-row-"]');
    const showing = Array.from(document.querySelectorAll("span")).find((s) =>
      /^Showing\s/.test(s.textContent?.trim() ?? ""),
    );
    const bodyBg = getComputedStyle(document.body).backgroundColor;

    return {
      url: location.pathname + location.search,
      densityAttr: document.documentElement.getAttribute("data-density"),
      scheme: getComputedStyle(document.documentElement).colorScheme,
      tokens: {
        border: tok("--m-border"),
        surface: tok("--m-surface"),
        surface2: tok("--m-surface-2"),
        textMuted: tok("--m-text-muted"),
        textSecondary: tok("--m-text-secondary"),
        statusDueBg: tok("--m-status-due-bg"),
        statusDueFg: tok("--m-status-due-fg"),
        goldSoft: tok("--m-gold-soft"),
        goldText: tok("--m-gold-text"),
        bodyFont: css.getPropertyValue("font-family").trim().slice(0, 40),
      },
      ledger: row
        ? {
            rowHeight: dims(row)?.h ?? null,
            headerHeight: dims(header)?.h ?? null,
            headerFont: header ? getComputedStyle(header).fontSize : null,
            cellFont: cell ? getComputedStyle(cell).fontSize : null,
            cellPadLeft: cell ? getComputedStyle(cell).paddingLeft : null,
            rows: document.querySelectorAll('[data-testid^="workflow-run-row-"]').length,
          }
        : null,
      statusPill: pillStyle
        ? {
            text: pill.textContent?.trim() ?? null,
            color: pillStyle.color,
            background: pillStyle.backgroundColor,
            fontSize: pillStyle.fontSize,
            contrastVsBody: contrast(pillStyle.color, bodyBg),
          }
        : null,
      banner: bannerStyle
        ? {
            text: banner.textContent?.replace(/\s+/g, " ").trim().slice(0, 100) ?? null,
            background: bannerStyle.backgroundColor,
            color: bannerStyle.color,
            contrast: contrast(bannerStyle.color, bannerStyle.backgroundColor),
            role: banner.getAttribute("role"),
          }
        : null,
      refusalRegion: refusal
        ? { text: refusal.textContent?.replace(/\s+/g, " ").trim().slice(0, 120) ?? null }
        : null,
      selectLabel: labelStyle
        ? {
            text: label.textContent?.trim() ?? null,
            fontSize: labelStyle.fontSize,
            fontWeight: labelStyle.fontWeight,
            letterSpacing: labelStyle.letterSpacing,
            textTransform: labelStyle.textTransform,
          }
        : null,
      stepRow: stepRow ? { height: dims(stepRow)?.h ?? null } : null,
      retryVerbs: Array.from(document.querySelectorAll('[data-testid^="run-retry-"]')).map((b) => ({
        testid: b.getAttribute("data-testid"),
        label: b.textContent?.replace(/\s+/g, " ").trim() ?? null,
        disabled: b.disabled,
      })),
      footer: { text: showing?.textContent?.replace(/\s+/g, " ").trim() ?? null },
      h1: el("h1")?.textContent?.trim() ?? null,
    };
  });
}

const loginWithRetry = async (page) => {
  for (let attempt = 0; attempt < 6; attempt += 1) {
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
      await page.waitForTimeout(6000);
    }
  }
  throw new Error("login failed after retries (auth gate may be throttling)");
};

const dismissTour = async (page) => {
  for (let step = 0; step < 8; step += 1) {
    const overlay = page.locator('[data-testid="tour-overlay"]');
    if (!(await overlay.isVisible().catch(() => false))) return;
    for (const name of [/don't show me tours/i, /^done$/i, /next/i]) {
      const target = page.getByRole("button", { name }).first();
      if (await target.isVisible().catch(() => false)) {
        await target.click({ timeout: 3000 }).catch(() => {});
        break;
      }
    }
    await page.waitForTimeout(300);
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
      await page.waitForTimeout(700);
      return;
    }
    await loginWithRetry(page);
  }
  throw new Error(`could not reach ${path} (repeatedly bounced to /login)`);
};

// ONE context, ONE login. The access token lives in this module's memory and
// the refresh secret rides a ROTATING httpOnly cookie, so a session cannot be
// shared across contexts (the second context presents a rotated-out cookie and
// bounces to /login), and logging in once per theme trips the auth gate. The
// theme is emulated on the page instead, and the viewport is resized for the
// mobile pass, so both modes and both widths run on one session.
const ctx = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
const page = await ctx.newPage();
page.on("pageerror", (err) => console.error("[pageerror]", err.message));
await loginWithRetry(page);
await dismissTour(page);

for (const theme of ["light", "dark"]) {
  await page.emulateMedia({
    colorScheme: theme,
    reducedMotion: theme === "dark" ? "reduce" : "no-preference",
  });

  // --- A: the rule builder with its header actions ---------------------------
  await gotoStaff(page, "/automations", "text=Automation rules");
  await shot(page, `${theme}-01-automations-header`);

  // --- B: the registry-driven trigger picker ---------------------------------
  const newRule = page.getByTestId("automation-new-rule");
  if (await newRule.isVisible().catch(() => false)) {
    await newRule.click();
    await page.waitForTimeout(700);
    const event = page.locator("#ar-event");
    if (await event.isVisible().catch(() => false)) {
      await event.click();
      await page.waitForTimeout(400);
      await shot(page, `${theme}-02-trigger-picker-ticket`);
      await page.keyboard.press("Escape");
      await page.waitForTimeout(200);
      await page.locator("#ar-subject").click();
      await page.waitForTimeout(300);
      await page.getByRole("option", { name: /^Board/ }).first().click().catch(() => {});
      await page.waitForTimeout(400);
      await event.click();
      await page.waitForTimeout(400);
      await shot(page, `${theme}-03-trigger-picker-board`);
      await page.keyboard.press("Escape");
    } else {
      console.log(`  [miss] trigger picker did not open for ${theme}`);
    }
    await page.keyboard.press("Escape");
    await page.waitForTimeout(400);
  }

  // --- C/D/E: the ledger, filtered, and a workflow that never ran ------------
  await gotoStaff(page, "/automations/runs", "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-04-run-ledger`);

  await gotoStaff(page, "/automations/runs?status=failed", "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-05-run-ledger-failed-only`);

  await gotoStaff(page, `/automations/runs?rule=${NEVER_RUN_RULE}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-06-run-ledger-never-run`);

  // --- F: the failed run that keeps a prefix — banner, trace, retry ----------
  await gotoStaff(page, `/automations/runs/${RUNS.failedKept}`, "text=Workflow runs");
  await page.waitForTimeout(1200);
  await shot(page, `${theme}-07-run-detail-failed-kept`);

  // --- G: the failed run that keeps nothing ----------------------------------
  await gotoStaff(page, `/automations/runs/${RUNS.failedEmpty}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-08-run-detail-failed-empty`);

  // --- H: parked (gold) and succeeded ----------------------------------------
  await gotoStaff(page, `/automations/runs/${RUNS.parked}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-09-run-detail-parked`);
  await gotoStaff(page, `/automations/runs/${RUNS.succeeded}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-10-run-detail-succeeded`);

  // --- I: the run a retry minted ---------------------------------------------
  await gotoStaff(page, `/automations/runs/${RUNS.retryChild}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-11-run-detail-retry-child`);

  // --- J: not found ----------------------------------------------------------
  await gotoStaff(page, "/automations/runs/00000000-0000-0000-0000-000000000000", "text=Run not found");
  await page.waitForTimeout(600);
  await shot(page, `${theme}-12-run-not-found`);

  // --- K: the retry refused, rendered ----------------------------------------
  await gotoStaff(page, `/automations/runs/${RUNS.failedKept}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  const retry = page.getByTestId("run-retry-resume");
  if (await retry.isVisible().catch(() => false)) {
    await retry.click();
    await page.waitForTimeout(2500);
    await shot(page, `${theme}-13-run-retry-refused`);
  } else {
    console.log(`  [miss] no retry button on the failed run for ${theme}`);
  }
}

// --- mobile pass ---------------------------------------------------------------
await page.setViewportSize({ width: 375, height: 780 });
await page.emulateMedia({ colorScheme: "light", reducedMotion: "no-preference" });
await gotoStaff(page, "/automations/runs", "text=Workflow runs");
await shot(page, "mobile-01-run-ledger-375");
await gotoStaff(page, `/automations/runs/${RUNS.failedKept}`, "text=Workflow runs");
await shot(page, "mobile-02-run-detail-375");
await ctx.close();

await browser.close();
writeFileSync(
  `${OUT}/measure.json`,
  JSON.stringify(
    {
      generated: new Date().toISOString(),
      viewport: { desktop: "1440x1000", mobile: "375x780" },
      source: "real executions — worker booted by hand (see REVIEW.md); no DB inserts",
      runs: RUNS,
      documents: { never_run_rule: NEVER_RUN_RULE },
      shots: measurements,
    },
    null,
    2,
  ),
);
console.log(`[capture] done — ${shots} captures in ${OUT}`);
console.log(`[capture] measured ${Object.keys(measurements).length} shots → ${OUT}/measure.json`);
