// Lane `p41-ui-operations` — vision-review captures.
//
// Drives the REAL app against the live e2e stack (never mocks) so the
// screenshots show true states. Covers the three surfaces this lane built or
// changed:
//
//   1. /tickets — the rebuilt ledger. The toolbar, the per-row SLA countdown,
//      the bulk-selection bar, the merged-row banner, and the honest empty
//      state.
//   2. /today — the on-call and schedule cards.
//   3. /tickets/[id] — the Details tab (custom fields + tags) and the Related
//      tab (explicit link tree + rollup).
//
// Scratch data is staged through /api/v1 rather than mocked, because the whole
// point of the SLA column is what the server actually says: a ticket with no
// SLA policy must read "—" and one with a due date inside the risk window must
// read as at risk.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p41-ui-operations-captures.mjs
import { chromium, request } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = process.env.CAPTURE_OUT ?? "design/qa-p41-ui-operations/captures";
const THEMES = (process.env.CAPTURE_THEMES ?? "light,dark").split(",");
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;

mkdirSync(OUT, { recursive: true });

const ctx0 = await request.newContext();
const login = await ctx0.post(`${API}/api/v1/auth/login`, {
  data: { email: EMAIL, password: PASSWORD },
});
if (!login.ok()) throw new Error(`login failed: ${login.status()}`);
const tokens = await login.json();

async function api(path, method = "GET", body) {
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

const me = await api(`/me`, "GET");
const boards = await api(`/boards`, "GET");
const boardId = boards[0].id;
const statuses = await api(`/boards/${boardId}/statuses`, "GET");

const byCategory = (c) => statuses.find((s) => s.category === c) ?? statuses[0];

// --- scratch tickets -------------------------------------------------------
// Three shapes the ledger has to render honestly:
//   AT_RISK  — a due date inside the 30-minute risk window (the warm tone)
//   UNDATED  — no due date and no SLA policy, which must read "—" not "0m"
//   PRIORITY — a P1, the only priority that takes red
const created = [];
for (const spec of [
  {
    title: `Ledger capture · uplink flapping ${STAMP}`,
    priority: "p1",
    category: "in_progress",
    due_in_min: 12,
  },
  {
    title: `Ledger capture · docs cleanup ${STAMP}`,
    priority: "p3",
    category: "waiting",
    due_in_min: null,
  },
  {
    title: `Ledger capture · quarterly patch ${STAMP}`,
    priority: "p2",
    category: "open",
    due_in_min: 60 * 30,
  },
]) {
  created.push(
    await api(`/tickets`, "POST", {
      title: spec.title,
      body: "Staged by the p41-ui-operations capture script.",
      priority: spec.priority,
      category_id: byCategory(spec.category).id,
      board_id: boardId,
      assignee_id: spec.category === "in_progress" ? me.user_id : null,
      due_at:
        spec.due_in_min === null
          ? null
          : new Date(Date.now() + spec.due_in_min * 60_000).toISOString(),
      idempotency_key: crypto.randomUUID(),
    }),
  );
}

// A tag on one ticket, so the Details tab's tag row has something to draw.
try {
  const tagRes = await api(`/tags`, "POST", { name: `capture-${STAMP}` });
  const tagId = tagRes?.tag?.id ?? tagRes?.id;
  if (tagId === undefined) throw new Error("tag create returned no id");
  await api(`/tickets/${created[0].id}/tags`, "POST", { tag_id: tagId });
} catch (e) {
  console.warn(`[capture] tag staging skipped: ${e.message}`);
}

// A merged ticket, so the banner has a real survivor to link to.
let mergedId = null;
try {
  const dup = await api(`/tickets`, "POST", {
    title: `Ledger capture · duplicate of uplink ${STAMP}`,
    body: "Merged by the capture script.",
    priority: "p4",
    category_id: byCategory("closed").id,
    board_id: boardId,
    idempotency_key: crypto.randomUUID(),
  });
  await api(`/tickets/${dup.id}/merge`, "POST", { survivor_id: created[0].id });
  mergedId = dup.id;
} catch (e) {
  console.warn(`[capture] merge staging skipped: ${e.message}`);
}

console.log(`[capture] staged ${created.length} tickets${mergedId ? " + 1 merged" : ""}`);

// --- capture ---------------------------------------------------------------

for (const theme of THEMES) {
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
      if (apiBase) window.__MARULA_API_URL__ = apiBase;
      window.localStorage.setItem("marula-tokens", tokenPair);
      window.localStorage.setItem("marula-tokens-exp", String(Date.now() + 3_600_000));
      window.localStorage.setItem("marula-theme", mode);
      window.sessionStorage.setItem("marula-tour-session", "1");
    },
    [API, JSON.stringify(tokens), theme],
  );

  const shot = async (name, locator) => {
    const file = `${OUT}/${theme}-${name}.png`;
    if (locator === undefined) await page.screenshot({ path: file, fullPage: true });
    else await locator.screenshot({ path: file });
    console.log(`[capture] ${file}`);
  };

  // 1. The ledger, default view.
  await page.goto(`${WEB}/tickets`, { waitUntil: "load" });
  await page.waitForSelector('[data-testid="ticket-row"]', { timeout: 20_000 }).catch(() => {});
  await page.waitForTimeout(2500); // let the capped SLA fan-out land
  await shot("01-tickets-ledger");

  // 2. Toolbar controls open (column chooser + a filter), proving the four
  //    data-surface controls are one strip and not a menu behind a kebab.
  const columnsBtn = page.getByRole("button", { name: /^Columns/ });
  if (await columnsBtn.count()) {
    await columnsBtn.first().click();
    await page.waitForTimeout(600);
    await shot("02-tickets-column-chooser");
    await page.keyboard.press("Escape");
    await page.waitForTimeout(300);
  }

  // 3. Bulk selection: the trailing bulk bar appears in the toolbar.
  const firstRowBox = page.locator('[data-testid="ticket-row"]').first().locator('button[role="checkbox"]');
  if (await firstRowBox.count()) {
    await firstRowBox.click();
    await page.waitForTimeout(500);
    await shot("03-tickets-bulk-selected");
    await firstRowBox.click();
    await page.waitForTimeout(300);
  }

  // 4. The merged-row banner.
  if (mergedId !== null) {
    const banner = page.getByTestId("merge-banner").first();
    if (await banner.count()) {
      await banner.scrollIntoViewIfNeeded();
      await page.waitForTimeout(400);
      await shot("04-tickets-merge-banner", page.locator("table").first());
    }
  }

  // 5. Today: the on-call and schedule cards.
  await page.goto(`${WEB}/today`, { waitUntil: "load" });
  await page.waitForTimeout(3000);
  await shot("05-today-rota");

  // 6. The workbench Details tab (custom fields + tags).
  await page.goto(`${WEB}/tickets/${created[0].id}`, { waitUntil: "load" });
  await page.waitForTimeout(2500);
  const detailsTab = page.getByTestId("detail-tab-details");
  if (await detailsTab.count()) {
    await detailsTab.first().click();
    await page.waitForTimeout(1200);
    await shot("06-workbench-details");
  }

  // 7. The workbench Related tab (link tree + rollup).
  const relatedTab = page.getByTestId("detail-tab-related");
  if (await relatedTab.count()) {
    await relatedTab.first().click();
    await page.waitForTimeout(1500);
    await shot("07-workbench-related");
  }

  // ---- interactive states: the ones a static first paint cannot show ----

  // 8a. Row HOVER. The ledger's row affordance, which no amount of static
  //     rendering can reveal because it only exists under a pointer.
  const hoverRow = page.locator('[data-testid="ticket-row"]').nth(1);
  if (await hoverRow.count()) {
    await hoverRow.hover();
    await page.waitForTimeout(350);
    await shot("09-tickets-row-hover", page.locator("table").first());
  }

  // 8b. The COLUMN CHOOSER open, with a column actually hidden — the state
  //     after a real choice, not just the popover on arrival.
  const colsBtn = page.getByRole("button", { name: /^Columns/ });
  if (await colsBtn.count()) {
    await colsBtn.first().click();
    await page.waitForTimeout(500);
    const assignee = page.getByRole("checkbox", { name: /Assignee/i }).first();
    if (await assignee.count()) {
      await assignee.click();
      await page.waitForTimeout(900);
      await shot("10-tickets-column-hidden", page.locator("table").first());
    }
    await page.keyboard.press("Escape");
    await page.waitForTimeout(400);
  }

  // 8c. A SAVED FILTER applied — the clause chip sitting in the toolbar and the
  //     ledger narrowed by it. This is the D12 state the whole surface exists
  //     to prove, and it has never been looked at.
  const addFilter = page.getByRole("button", { name: /Add filter/ });
  if (await addFilter.count()) {
    await addFilter.first().click();
    await page.waitForTimeout(400);
    const field = page.locator("select").filter({ hasText: "Priority" }).first();
    if (await field.count()) {
      await field.selectOption("p1").catch(() => {});
      await page.waitForTimeout(300);
    }
    const op = page.locator("select").nth(1);
    if (await op.count()) await op.selectOption({ label: /is any|equals|=/i }).catch(() => {});
    await page.waitForTimeout(300);
    const val = page.locator("input[type=text], input:not([type])").first();
    if (await val.count()) await val.fill("p1").catch(() => {});
    await page.waitForTimeout(1500);
    await shot("11-tickets-filter-applied", page.locator("table").first());
  }

  // 8d. The EMPTY state a bad filter produces — one sentence and one action.
  const removeChips = page.getByRole("button", { name: /Remove|×|✕/ });
  const chipCount = await removeChips.count();
  for (let i = 0; i < chipCount; i += 1) {
    await removeChips.first().click().catch(() => {});
    await page.waitForTimeout(400);
  }
  const addFilter2 = page.getByRole("button", { name: /Add filter/ });
  if (await addFilter2.count()) {
    await addFilter2.first().click();
    await page.waitForTimeout(400);
    const noMatch = page.locator("input[type=text], input:not([type])").first();
    if (await noMatch.count()) await noMatch.fill("zzzz-no-such-ticket").catch(() => {});
    await page.waitForTimeout(1500);
    await shot("12-tickets-empty-state");
  }

  // 9. A ticket with no due date and no policy — the honest "—" reading.
  await page.goto(`${WEB}/tickets`, { waitUntil: "load" });
  await page.waitForTimeout(2500);
  const undated = page.getByText(`Ledger capture · docs cleanup ${STAMP}`).first();
  if (await undated.count()) {
    await undated.scrollIntoViewIfNeeded();
    await page.waitForTimeout(400);
    await shot("08-tickets-underscore-reading", page.locator("table").first());
  }

  await browser.close();
  console.log(`[capture] ${theme} done`);
}