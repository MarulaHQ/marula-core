// p46-draft-verification-captures.mjs — Wave 64 row 5, the visual check the
// data cannot make.
//
// The row asks for the rendered draft to be looked at, in both themes, and
// judged by a second model against the pack's density, token, typography and
// radius law — because a card the layout squashes is a layout defect, not a
// data one. This harness produces the images and the batch packaging. It makes
// no judgement: the verdict belongs to the reviewer that reads them, and a
// harness that wrote one would be inventing evidence.
//
// It boots nothing. It expects `scripts/e2e-stack.sh up` plus the built web
// bundle, and reads every port from `e2e/.e2e-stack.env` (p37 doctrine).
//
// The authoring endpoint is a FIXTURE here, not a live model call: this lane
// has no provider, and a capture that waited on one would be a capture of a
// refusal. `page.route` answers `POST /api/v1/reports/author` with the exact
// envelope the server produces for three states — clean, repaired, and handed
// back with a surviving finding — so the row under the camera is the real
// component rendering real server-shaped data. The manifest says so next to
// every batch, so nobody reads these as a model transcript.
//
// Usage:
//   node e2e/p46-draft-verification-captures.mjs
//   LANE=design/qa-p46-draft-verification node e2e/p46-draft-verification-captures.mjs
import { chromium } from "playwright";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const LANE = process.env.LANE ?? "design/qa-p46-draft-verification";
const OUT = resolve(LANE, "captures");
mkdirSync(OUT, { recursive: true });

const envPath = resolve("e2e/.e2e-stack.env");
if (!existsSync(envPath)) {
  throw new Error(
    "e2e/.e2e-stack.env is missing. Run scripts/e2e-stack.sh up first (lane boot " +
      "routes through that script; never set the ports by hand).",
  );
}
const env = Object.fromEntries(
  readFileSync(envPath, "utf8")
    .split("\n")
    .filter((line) => line.includes("="))
    .map((line) => [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)]),
);
const WEB = process.env.E2E_WEB_URL ?? env.E2E_WEB_URL ?? "http://localhost:3000";
const API = (process.env.E2E_API_URL ?? env.E2E_API_URL ?? "http://localhost:8080").replace(
  /\/+$/,
  "",
);

/* The pack's recorded desktop size (internal mockup pack manifest; the same
 * 1600x994 the Wave 47 capture harness shoots at), plus the common laptop. */
const VIEWPORTS = [
  { id: "pack", width: 1600, height: 994 },
  { id: "laptop", width: 1280, height: 800 },
];

const STAFF_EMAIL = "owner@demo.local";
const STAFF_PASSWORD = "demo-password-123";

const SPEC = {
  view: "tickets",
  filters: [],
  group_by: ["priority"],
  measures: [{ agg: "count" }],
  viz: "bar",
};
const CARD = {
  size: { span: "half", height: "regular" },
  window_days: 30,
  refresh: { every_minutes: 60, reach_days: 0 },
};
const READS =
  "A chart over tickets grouped by priority, showing a count of rows. It spans the " +
  "last 30 days, refreshes every hour, and sits half wide and regular tall.";

const CHECKED = [
  "the spec and card pass the same boundary the save and run routes use",
  "a chart hint has a dimension to put on its axis",
  "the window is no wider than one refresh reaches",
  "a chart has rows, a table has headers, a stat has a number",
  "no card's filters resolve to zero rows",
];

const DRAFT = {
  name: "Tickets by priority",
  spec: SPEC,
  card: CARD,
  shape: "chart",
  reads: READS,
};

const DIFF = [
  { op: "add", path: "view", after: "tickets" },
  { op: "add", path: "group_by/priority", after: "priority" },
  { op: "add", path: "card/window_days", after: 30 },
];

const BASE = {
  saved: false,
  diff: DIFF,
  tool_calls: [
    { name: "get_view_columns", argument: "tickets", outcome: "ok" },
    { name: "verify_report_draft", argument: "tickets", outcome: "clean" },
  ],
  usage: { prompt_tokens: 180, completion_tokens: 96, steps: 3 },
  model_info: { provider: "fixture", model: "fixture-model", resolution: "fixture" },
  catalogue: { views: 5, shapes: 3, rules: 3, prompt_chars: 5393 },
};

/* The three states the row can render, each the envelope the server sends. */
const STATES = {
  ready: {
    ...BASE,
    valid: true,
    draft: DRAFT,
    issues: [],
    verification: {
      ran: true,
      checked: CHECKED,
      attempts: [{ round: 0, findings: [] }],
      repaired: [],
      findings: [],
    },
  },
  repaired: {
    ...BASE,
    valid: true,
    draft: DRAFT,
    issues: [],
    verification: {
      ran: true,
      checked: CHECKED,
      attempts: [
        {
          round: 0,
          findings: [
            { kind: "no_rows", message: "The chart has no rows at this window." },
          ],
        },
        { round: 1, findings: [] },
      ],
      repaired: [
        { kind: "no_rows", message: "The chart has no rows at this window." },
      ],
      findings: [],
    },
  },
  needs_fixes: {
    ...BASE,
    valid: false,
    draft: DRAFT,
    issues: [
      {
        code: "no_rows",
        path: "card[3]",
        message: "Card 3's chart has no rows at this window.",
      },
    ],
    verification: {
      ran: true,
      checked: CHECKED,
      attempts: [
        {
          round: 0,
          findings: [
            { kind: "no_rows", card_index: 2, message: "Card 3's chart has no rows at this window." },
          ],
        },
        {
          round: 1,
          findings: [
            { kind: "no_rows", card_index: 2, message: "Card 3's chart has no rows at this window." },
          ],
        },
        {
          round: 2,
          findings: [
            { kind: "no_rows", card_index: 2, message: "Card 3's chart has no rows at this window." },
          ],
        },
      ],
      repaired: [],
      findings: [
        { kind: "no_rows", card_index: 2, message: "Card 3's chart has no rows at this window." },
      ],
    },
  },
};

const shots = [];
const problems = [];
const log = (line) => console.log(line);
const warn = (line) => {
  problems.push(line);
  console.log(`  ! ${line}`);
};

async function staffToken() {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: STAFF_EMAIL, password: STAFF_PASSWORD }),
  });
  if (!res.ok) throw new Error(`staff login failed (${res.status})`);
  return res.json();
}

async function staffContext(browser, { theme, width, height, reducedMotion }) {
  const ctx = await browser.newContext({
    viewport: { width, height },
    colorScheme: theme,
    reducedMotion,
  });
  const tokens = await staffToken();
  const login = await ctx.request.post(`${API}/api/v1/auth/login`, {
    data: { email: STAFF_EMAIL, password: STAFF_PASSWORD },
  });
  if (!login.ok()) throw new Error(`context login failed (${login.status()})`);
  await ctx.request.put(`${API}/api/v1/settings/user`, {
    data: { key: "tours.disabled", value: true },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  await ctx.addInitScript(
    ([raw, exp, themeValue]) => {
      window.localStorage.setItem("marula-tokens", raw);
      window.localStorage.setItem("marula-tokens-exp", exp);
      window.localStorage.setItem("marula-theme", themeValue);
      window.localStorage.setItem("marula-theme-preset", "marula");
    },
    [
      JSON.stringify(tokens),
      String(Date.now() + tokens.expires_in * 1000 - 30_000),
      theme === "dark" ? "dark" : "light",
    ],
  );
  return ctx;
}

/** Renders the draft row for one fixture state and waits for it to settle. */
async function renderDraft(page, state) {
  await page.route("**/api/v1/reports/author", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(STATES[state]),
    }),
  );
  await page.goto(`${WEB}/reports/author`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.getByLabel("What should this report answer?").fill(
    "Tickets by priority as a bar chart, last thirty days.",
  );
  await page.getByRole("button", { name: /Generate a draft/ }).click();
  await page.waitForSelector('[data-testid="report-draft-row"]', { timeout: 30_000 });
  // The row animates in at 120ms; wait past it so a mid-rise frame is never
  // the evidence, and so a reduced-motion run proves the row is static.
  await page.waitForTimeout(250);
  // Expand the class list in the verification block: the reviewer must see what
  // was checked, not only that something was.
  const summary = page.getByText("What was checked", { exact: true });
  if (await summary.count()) {
    await summary.first().click();
    await page.waitForTimeout(100);
  }
  return page.locator('[data-testid="report-draft-row"]');
}

function record(shot) {
  shots.push(shot);
  log(`  ✓ ${shot.file} (${shot.state} / ${shot.theme} / ${shot.viewport}${shot.reducedMotion ? " / reduced-motion" : ""})`);
}

async function main() {
  const browser = await chromium.launch();
  const plan = [];
  for (const viewport of VIEWPORTS) {
    for (const state of ["ready", "repaired", "needs_fixes"]) {
      for (const theme of ["light", "dark"]) {
        plan.push({ viewport, state, theme, reducedMotion: "no-preference", kind: "row" });
      }
    }
  }
  // Reduced motion is proven for the two states whose motion differs: the
  // clean row (entrance only) and the exhausted one (entrance plus the
  // findings list).
  for (const state of ["ready", "needs_fixes"]) {
    for (const theme of ["light", "dark"]) {
      plan.push({
        viewport: VIEWPORTS[0],
        state,
        theme,
        reducedMotion: "reduce",
        kind: "row",
      });
    }
  }
  // Two page-level shots, so the row is judged in the composition it sits in
  // rather than floating on its own.
  for (const state of ["needs_fixes", "ready"]) {
    plan.push({
      viewport: VIEWPORTS[0],
      state,
      theme: "light",
      reducedMotion: "no-preference",
      kind: "page",
    });
  }

  for (const item of plan) {
    const tag = [item.state, item.theme, item.viewport.id, item.reducedMotion, item.kind].join(
      "-",
    );
    const file = `${tag}.png`;
    const ctx = await staffContext(browser, {
      theme: item.theme,
      width: item.viewport.width,
      height: item.viewport.height,
      reducedMotion: item.reducedMotion,
    });
    try {
      const page = await ctx.newPage();
      const locator = await renderDraft(page, item.state);
      if (item.kind === "page") {
        await page.screenshot({ path: resolve(OUT, file), fullPage: true });
      } else {
        await locator.screenshot({ path: resolve(OUT, file) });
      }
      record({
        file,
        state: item.state,
        theme: item.theme,
        viewport: item.viewport.id,
        width: item.viewport.width,
        height: item.viewport.height,
        reducedMotion: item.reducedMotion === "reduce",
        kind: item.kind,
      });
      await page.close();
    } catch (error) {
      warn(`${tag}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      await ctx.close();
    }
  }
  await browser.close();

  /* The batching rule is binding: a review request never carries more than 30
   * images, and this lane reviews in batches of at most 8. The manifest does
   * the split so the reviewer consumes them in order without re-deriving it. */
  const batches = [];
  for (let i = 0; i < shots.length; i += 8) {
    batches.push({
      index: batches.length + 1,
      images: shots.slice(i, i + 8).map((shot) => shot.file),
      ask: [
        "Does the card sit on the 12-column grid at the recorded span and height, " +
          "or does the layout squash it?",
        "Do the surfaces, borders and type sizes come from the product's token set " +
          "and density scale, with no off-palette colour and no shadow where the pack " +
          "uses a border?",
        "Is the type hierarchy readable at the recorded size: title, meta line, " +
          "section heading, body, machine class in mono — and does any line wrap " +
          "into a wall?",
        "Is the verification block legible as evidence (what was checked, what was " +
          "repaired, what failed) rather than decoration?",
        "Is the 16px card radius and the item radius consistent with the pack, and " +
          "does anything clip at the card edge?",
        "In the dark shot, does every text tone keep its contrast against its own " +
          "surface?",
        "In the reduced-motion shot, is the row fully composed with nothing mid-rise?",
      ],
    });
  }

  const manifest = {
    lane: LANE,
    harness: "e2e/p46-draft-verification-captures.mjs",
    authoring_endpoint: "fixture (page.route) — no live model provider was called",
    viewports: VIEWPORTS,
    states: Object.keys(STATES),
    shots,
    batches,
    problems,
    judged: false,
    judged_note:
      "No verdict is recorded by this harness. The images and the batch questions " +
      "are the input to a review; a judgement this run did not obtain is not evidence.",
  };
  writeFileSync(resolve(OUT, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
  log(`\n${shots.length} shots, ${batches.length} batches → ${resolve(OUT, "manifest.json")}`);
  if (problems.length > 0) {
    log(`${problems.length} capture problem(s) — read them before judging.`);
    process.exitCode = 1;
  }
}

await main();
