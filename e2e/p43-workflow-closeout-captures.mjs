// Wave 43 lane 18 `p43-workflow-closeout-ui` — vision-review captures for the
// workflow close-out surfaces, drawn against the REAL e2e stack (never mocks):
//
//   A  /automations — the header (Rules/Canvas toggle, Runs, Webhook activity)
//      and the per-rule "Runs" link into that workflow's history.
//   B  /automations with the editor open — the registry-driven trigger picker,
//      grouped by trigger node, on both subjects (ticket, board).
//   C  /automations/runs — the ledger, populated, unfiltered.
//   D  /automations/runs?status=failed — the filter row narrowed.
//   E  /automations/runs?rule=<id> — a workflow that has never run (the
//      never-run empty state, which is not the same sentence as "no runs").
//   F  /automations/runs/<id> — the failed run: the blush banner, the trace
//      with its failed step selected, the two retry verbs.
//   G  /automations/runs/<id> — a succeeded run and a parked run (the gold
//      banner), for the other two banner shapes.
//   H  the retry outcome (whatever the API returns), plus /automations/runs
//      with a run id that does not exist.
//
// Captured light + dark, plus a 375px mobile pass.
//
// FIXTURE, STATED PLAINLY: the stack boots core-api and realtime, not the
// worker, so no event executes and the ledger would be empty — the surface
// could not be looked at. This script therefore writes run and step rows into
// the lane's own run database with `psql` before capturing. The rows are real
// rows in the real schema, read back through the real API; they are not the
// executor's output, and the review says so.
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p43-workflow-closeout-captures.mjs
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { chromium } from "playwright";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const DB = process.env.DATABASE_URL ?? "";
const OUT = "design/qa-p43-workflow-closeout-ui/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;

mkdirSync(OUT, { recursive: true });

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

function psql(sql) {
  return execFileSync("psql", [DB, "-v", "ON_ERROR_STOP=1", "-tAc", sql], {
    encoding: "utf8",
  }).trim();
}

// Two rules: one that has run, one that has never run (the never-run state).
const ruleRes = await auth("/automation-rules", {
  method: "POST",
  body: JSON.stringify({
    name: `Capture run rule (${STAMP})`,
    board_id: null,
    subject_kind: "ticket",
    trigger_event: "ticket.created",
    conditions: [{ type: "priority_is", priority: "p1" }],
    actions: [{ type: "set_priority", priority: "p2" }],
  }),
});
const ruleId = (await json(ruleRes))?.id ?? null;
const freshRes = await auth("/automation-rules", {
  method: "POST",
  body: JSON.stringify({
    name: `Capture never-run rule (${STAMP})`,
    board_id: null,
    subject_kind: "ticket",
    trigger_event: "ticket.status_changed",
    conditions: [],
    actions: [{ type: "add_note", body: "Hello" }],
  }),
});
const neverRunRuleId = (await json(freshRes))?.id ?? null;
console.log(`[capture] rule ${ruleId} · never-run rule ${neverRunRuleId}`);

// --- fixture runs --------------------------------------------------------------
const orgId = psql("SELECT org_id FROM automation_rules ORDER BY created_at LIMIT 1");
if (!orgId) throw new Error("no org in the demo database");

const q = (s) => `'${String(s).replace(/'/g, "''")}'`;
const at = (minutesAgo) => `now() - interval '${minutesAgo} minutes'`;
const ids = {
  ok: randomUUID(),
  failed: randomUUID(),
  parked: randomUUID(),
  running: randomUUID(),
  retry: randomUUID(),
};
const definition = JSON.stringify({ trigger_event: "ticket.created", conditions: [], actions: [] });

function runRow({ id, status, trigger, started, finished, error_code, error, resume_cursor, wake_on, wake_at, resumed_from, retry_of }) {
  return (
    `(${q(id)}, ${q(orgId)}, ${ruleId ? q(ruleId) : "NULL"}, ${q(`Capture run rule (${STAMP})`)}, 1, ` +
    `${q(definition)}::jsonb, '1', ${q(status)}, ${q(trigger)}, ` +
    `${resumed_from ? q(resumed_from) : "NULL"}, ${retry_of ? q(retry_of) : "NULL"}, ` +
    `${resume_cursor ? `${q(JSON.stringify(resume_cursor))}::jsonb` : "NULL"}, ` +
    `${wake_at ? `now() + interval '${wake_at}'` : "NULL"}, ${wake_on ? q(wake_on) : "NULL"}, ` +
    `${error_code ? q(error_code) : "NULL"}, ${error ? q(error) : "NULL"}, ` +
    `${started}, ${finished ?? "NULL"}, ${started}, ${started})`
  );
}

function stepRow({ run_id, step_index, node_id, node_label, status, input, output, error_code, error, started, finished, attempt = 1 }) {
  return (
    `(${q(randomUUID())}, ${q(orgId)}, ${q(run_id)}, ${step_index}, ${q(node_id)}, ${q(node_label)}, ${attempt}, NULL, ` +
    `${q(status)}, ${q(JSON.stringify(input))}::jsonb, ` +
    `${output === undefined ? "NULL" : `${q(JSON.stringify(output))}::jsonb`}, ` +
    `${error_code ? q(error_code) : "NULL"}, ${error ? q(error) : "NULL"}, ${started}, ${finished ?? "NULL"})`
  );
}

const runCols = "(id, org_id, rule_id, rule_name, rule_version, rule_definition, catalogue_version, status, trigger_kind, resumed_from, retry_of, resume_cursor, wake_at, wake_on, error_code, error, started_at, finished_at, created_at, updated_at)";
const stepCols = "(id, org_id, run_id, step_index, node_id, node_label, attempt, iteration, status, input, output, error_code, error, started_at, finished_at)";

const runs = [
  runRow({ id: ids.ok, status: "succeeded", trigger: "event", started: at(14), finished: at(14) }),
  runRow({
    id: ids.failed, status: "failed", trigger: "event", started: at(48), finished: at(48),
    error_code: "node_dispatch_failed",
    error: "The HTTP step was refused: the RMM endpoint reset the connection.",
  }),
  runRow({
    id: ids.retry, status: "waiting", trigger: "operator_retry", started: at(6),
    retry_of: ids.failed, resume_cursor: { start_at_step: 3, mode: "resume" },
    wake_on: "queue", wake_at: "5 minutes",
  }),
  runRow({
    id: ids.parked, status: "waiting", trigger: "event", started: at(120),
    resume_cursor: { start_at_step: 2 }, wake_on: "flow.delay", wake_at: "1 hour",
  }),
  runRow({ id: ids.running, status: "running", trigger: "manual", started: at(1) }),
];

const steps = [
  // the clean run
  stepRow({ run_id: ids.ok, step_index: 1, node_id: "trigger.ticket", node_label: "Ticket lifecycle", status: "succeeded", input: { eventType: "ticket.created" }, output: { ticketId: "9f2c1a40-6b21-4e77-8d10-2c5f0a1b7d33", subject: { number: 18432 } }, started: at(14), finished: at(14) }),
  stepRow({ run_id: ids.ok, step_index: 2, node_id: "action.set_priority", node_label: "Set priority", status: "succeeded", input: { priority: "p2" }, output: { applied: true, priority: "p2" }, started: at(14), finished: at(14) }),
  // the failed run: two applied, one broke, the tail never reached
  stepRow({ run_id: ids.failed, step_index: 1, node_id: "trigger.ticket", node_label: "Ticket lifecycle", status: "succeeded", input: { eventType: "ticket.created" }, output: { ticketId: "1c7b9e02-4a51-4b0f-9f8e-77a1c2d3e4f5" }, started: at(48), finished: at(48) }),
  stepRow({ run_id: ids.failed, step_index: 2, node_id: "field.equals", node_label: "Field equals", status: "succeeded", input: { field: "priority", value: "p1" }, output: { matched: true }, started: at(48), finished: at(48) }),
  stepRow({
    run_id: ids.failed, step_index: 3, node_id: "action.http", node_label: "Call an HTTP endpoint", status: "failed",
    input: { method: "POST", url: "https://rmm.example.local/api/refresh", timeout_ms: 5000 },
    error_code: "node_dispatch_failed",
    error: "The HTTP step was refused: the RMM endpoint reset the connection.",
    started: at(48), finished: at(48),
  }),
  stepRow({ run_id: ids.failed, step_index: 4, node_id: "flow.stop", node_label: "Stop the run", status: "skipped", input: {}, started: at(48), finished: at(48) }),
  // the parked run
  stepRow({ run_id: ids.parked, step_index: 1, node_id: "trigger.ticket", node_label: "Ticket lifecycle", status: "succeeded", input: { eventType: "ticket.created" }, output: { ticketId: "1c7b9e02-4a51-4b0f-9f8e-77a1c2d3e4f5" }, started: at(120), finished: at(120) }),
  stepRow({ run_id: ids.parked, step_index: 2, node_id: "flow.delay", node_label: "Wait", status: "waiting", input: { duration: "1h" }, started: at(120) }),
  // the running run
  stepRow({ run_id: ids.running, step_index: 1, node_id: "trigger.manual", node_label: "Manual run", status: "succeeded", input: {}, output: { subject: {} }, started: at(1), finished: at(1) }),
  stepRow({ run_id: ids.running, step_index: 2, node_id: "action.add_note", node_label: "Add a note", status: "running", input: { body: "Refreshing the VPN profile" }, started: at(1) }),
];

psql(`INSERT INTO workflow_runs ${runCols} VALUES ${runs.join(",")};`);
psql(`INSERT INTO workflow_run_steps ${stepCols} VALUES ${steps.join(",")};`);
console.log(`[capture] fixture rows: ${runs.length} runs, ${steps.length} steps`);

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
 * What the page actually rendered, read from computed style and rects. Nulls
 * are honest: a shot with no banner reports `banner: null` rather than a
 * default that reads like a pass. Every value here is measured, never assumed.
 */
async function measure(page) {
  return page.evaluate(() => {
    const css = getComputedStyle(document.documentElement);
    const tok = (k) => css.getPropertyValue(k).trim();
    const el = (s) => document.querySelector(s);
    const dims = (n) =>
      n
        ? {
            h: Math.round(n.getBoundingClientRect().height),
            w: Math.round(n.getBoundingClientRect().width),
          }
        : null;

    // WCAG relative-luminance contrast for two computed colours.
    const rgb = (s) => {
      const m = (s || "").match(/\d+(\.\d+)?/g);
      return m ? m.slice(0, 3).map(Number) : null;
    };
    const lum = (c) => {
      const [r, g, b] = c.map((v) => {
        const x = v / 255;
        return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    const contrast = (fg, bg) => {
      const a = rgb(fg);
      const b = rgb(bg);
      if (!a || !b) return null;
      const la = lum(a);
      const lb = lum(b);
      const hi = Math.max(la, lb);
      const lo = Math.min(la, lb);
      return Math.round(((hi + 0.05) / (lo + 0.05)) * 100) / 100;
    };

    const row = el('[data-testid^="workflow-run-row-"]');
    const header = el('[data-testid="workflow-runs-table"] thead th');
    const cell = row?.querySelector("td") ?? null;
    const pill = row?.querySelector('[data-slot="pill"]') ?? el('[data-slot="pill"]');
    const pillStyle = pill ? getComputedStyle(pill) : null;
    const banner = el('[data-testid="run-banner"]');
    const bannerStyle = banner ? getComputedStyle(banner) : null;
    const label = el(".m-select-label");
    const labelStyle = label ? getComputedStyle(label) : null;
    const stepRow = el('[data-testid^="run-step-row-"]');
    const footer = el(".m-ledger-footer, .m-surface-footer") ?? null;
    const showing = Array.from(document.querySelectorAll("span")).find((s) =>
      /^Showing\s/.test(s.textContent?.trim() ?? ""),
    );

    return {
      url: location.pathname + location.search,
      densityAttr: document.documentElement.getAttribute("data-density"),
      scheme: getComputedStyle(document.documentElement).colorScheme,
      tokens: {
        border: tok("--m-border"),
        surface: tok("--m-surface"),
        textMuted: tok("--m-text-muted"),
        statusDueBg: tok("--m-status-due-bg"),
        statusDueFg: tok("--m-status-due-fg"),
        goldSoft: tok("--m-gold-soft"),
        goldText: tok("--m-gold-text"),
        bodyFont: css.getPropertyValue("font-family").trim().slice(0, 60),
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
            dotSize: pill.querySelector("span")?.getBoundingClientRect().width ?? null,
            contrastTextOnSurface: contrast(pillStyle.color, getComputedStyle(document.body).backgroundColor),
          }
        : null,
      banner: bannerStyle
        ? {
            text: banner.textContent?.replace(/\s+/g, " ").trim().slice(0, 90) ?? null,
            background: bannerStyle.backgroundColor,
            color: bannerStyle.color,
            contrast: contrast(bannerStyle.color, bannerStyle.backgroundColor),
            role: banner.getAttribute("role"),
          }
        : null,
      selectLabel: labelStyle
        ? {
            text: label.textContent?.trim() ?? null,
            fontSize: labelStyle.fontSize,
            fontWeight: labelStyle.fontWeight,
            letterSpacing: labelStyle.letterSpacing,
            textTransform: labelStyle.textTransform,
            color: labelStyle.color,
          }
        : null,
      stepRow: stepRow ? { height: dims(stepRow)?.h ?? null } : null,
      footer: {
        text: showing?.textContent?.replace(/\s+/g, " ").trim() ?? null,
        present: footer !== null,
      },
      h1: el("h1")?.textContent?.trim() ?? null,
    };
  });
}

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

for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({
    colorScheme: theme,
    viewport: { width: 1440, height: 1000 },
    reducedMotion: theme === "dark" ? "reduce" : "no-preference",
  });
  const page = await ctx.newPage();
  page.on("pageerror", (err) => console.error(`[pageerror ${theme}]`, err.message));
  await loginWithRetry(page);
  await dismissTour(page);

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
      // board subject: the ticket-less envelope list, and its deliveries link
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

  if (neverRunRuleId) {
    await gotoStaff(page, `/automations/runs?rule=${neverRunRuleId}`, "text=Workflow runs");
    await page.waitForTimeout(900);
    await shot(page, `${theme}-06-run-ledger-never-run`);
  }

  // --- F: the failed run — banner, trace, retry ------------------------------
  await gotoStaff(page, `/automations/runs/${ids.failed}`, "text=Workflow runs");
  await page.waitForTimeout(1200);
  await shot(page, `${theme}-07-run-detail-failed`);

  // --- G: the other two banner shapes ---------------------------------------
  await gotoStaff(page, `/automations/runs/${ids.parked}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-08-run-detail-parked`);
  await gotoStaff(page, `/automations/runs/${ids.ok}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  await shot(page, `${theme}-09-run-detail-succeeded`);

  // --- H: not found, and the retry's real outcome ----------------------------
  await gotoStaff(page, "/automations/runs/00000000-0000-0000-0000-000000000000", "text=Run not found");
  await page.waitForTimeout(600);
  await shot(page, `${theme}-10-run-not-found`);

  await gotoStaff(page, `/automations/runs/${ids.failed}`, "text=Workflow runs");
  await page.waitForTimeout(900);
  const retry = page.getByTestId("run-retry-resume");
  if (await retry.isVisible().catch(() => false)) {
    await retry.click();
    await page.waitForTimeout(2000);
    await shot(page, `${theme}-11-run-retry-outcome`);
  } else {
    console.log(`  [miss] no retry button on the failed run for ${theme}`);
  }

  await ctx.close();
}

// --- mobile pass ---------------------------------------------------------------
{
  const ctx = await browser.newContext({ colorScheme: "light", viewport: { width: 375, height: 780 } });
  const page = await ctx.newPage();
  await loginWithRetry(page);
  await dismissTour(page);
  await gotoStaff(page, "/automations/runs", "text=Workflow runs");
  await shot(page, "mobile-01-run-ledger-375");
  await gotoStaff(page, `/automations/runs/${ids.failed}`, "text=Workflow runs");
  await shot(page, "mobile-02-run-detail-375");
  await ctx.close();
}

await browser.close();
writeFileSync(
  `${OUT}/captures/measure.json`,
  JSON.stringify(
    {
      generated: new Date().toISOString(),
      viewport: { desktop: "1440x1000", mobile: "375x780" },
      fixture_runs: { ok: ids.ok, failed: ids.failed, parked: ids.parked, running: ids.running, retry: ids.retry },
      shots: measurements,
    },
    null,
    2,
  ),
);
console.log(`[capture] done — ${shots} captures in ${OUT}`);
console.log(`[capture] measured ${Object.keys(measurements).length} shots → ${OUT}/captures/measure.json`);
console.log(`[capture] run ids: failed=${ids.failed} parked=${ids.parked} ok=${ids.ok}`);
