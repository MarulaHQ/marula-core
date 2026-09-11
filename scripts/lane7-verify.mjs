// Lane-7 vision verification driver (Wave 12 `p12-straggler-ui`).
//
// Drives the two straggler web surfaces against the live demo stack
// (local Postgres + core-api dev seed + the built web app) and drops
// light+dark captures at 768/1024/1440 for review:
//   1. Settings → Taxonomy — type + category/subcategory tree editors.
//   2. Ticket board move — status-bar affordance, target board/status +
//      reset_sla, and the typed `board_move_status_conflict` retry.
//
// One browser session: login once, then toggle the theme through the
// persisted key (`meridian-theme`) and re-navigate per capture.
// Run: node scripts/lane7-verify.mjs
import { chromium } from "playwright";

const BASE = "http://localhost:3000";
const API = "http://localhost:8080";
const OUT = "design/qa-straggler";
const BOARD_A = "6d657269-6469-616e-0000-00000000003c";
const SUF = String(Date.now() % 10000);

const results = [];
const step = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${detail ? ` — ${detail}` : ""}`);
};

async function login(p) {
  for (let attempt = 1; attempt <= 3; attempt++) {
    await p.goto(`${BASE}/login`, { waitUntil: "domcontentloaded" });
    await p.fill('input[type="email"], input[name="email"]', "owner@demo.local");
    await p.fill('input[type="password"], input[name="password"]', "demo-password-123");
    await p.click('button[type="submit"]');
    const landed = await p
      .waitForURL(/\/today/, { timeout: 20000 })
      .then(() => true)
      .catch(() => false);
    if (landed) {
      // The login page stores the token pair but not the expiry hint, so a
      // full navigation triggers a proactive refresh race. Seed the hint the
      // way the app's own refresh path does; this is harness-only.
      await p.evaluate(() =>
        localStorage.setItem("meridian-tokens-exp", String(Date.now() + 600000)),
      );
      return;
    }
    console.log(`login attempt ${attempt} failed (url=${p.url()})`);
  }
  throw new Error("login failed");
}

async function ownerToken() {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: "owner@demo.local", password: "demo-password-123" }),
  });
  return (await res.json()).access_token;
}
const token = await ownerToken();
const apiGet = async (path) =>
  (await fetch(`${API}/api/v1${path}`, { headers: { authorization: `Bearer ${token}` } })).json();

let ticket = null;
for (const t of (await apiGet("/tickets?limit=100")) ?? []) {
  if (t.board_id === BOARD_A && t.category === "open" && t.merged_into_ticket_id == null) {
    ticket = t;
    break;
  }
}
step("open demo ticket resolved", ticket !== null, ticket ? `${ticket.number} ${ticket.id.slice(0, 8)}` : "none");
if (ticket === null) process.exit(1);

const boards = await apiGet("/boards");
const boardC = boards.find((b) => b.name === "Closed Only QA");
step("closed-only conflict board present", boardC != null, boardC?.id ?? "missing");

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 });
const page = await ctx.newPage();
await login(page);

const shots = [];
async function shot(name, fullPage = false) {
  await page.screenshot({ path: `${OUT}/${name}.png`, fullPage });
  shots.push(name);
  console.log("shot:", name);
}
async function setTheme(theme) {
  await page.evaluate((t) => localStorage.setItem("meridian-theme", t), theme);
}
async function overflowAt768(label) {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  step(`${label} 768 no horizontal overflow`, overflow <= 0, `${overflow}px`);
}

// ---------------------------------------------------------------------------
// 1. Settings → Taxonomy — light+dark at 1440/1024/768
// ---------------------------------------------------------------------------
for (const theme of ["light", "dark"]) {
  await setTheme(theme);
  for (const width of [1440, 1024, 768]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`${BASE}/settings/taxonomy`, { waitUntil: "domcontentloaded" });
    await page.getByTestId("new-type").waitFor({ timeout: 25000 });
    await page.getByText("Incident", { exact: true }).first().waitFor({ timeout: 15000 });
    await shot(`settings-taxonomy-${theme}-${width}`, true);
    if (width === 768) await overflowAt768(`taxonomy ${theme}`);
  }
}

// Interaction evidence (light, 1440): create → retire a type; reparent dialog.
await setTheme("light");
await page.setViewportSize({ width: 1440, height: 900 });
await page.goto(`${BASE}/settings/taxonomy`, { waitUntil: "domcontentloaded" });
await page.getByTestId("new-type").waitFor({ timeout: 25000 });
await page.getByText("Incident", { exact: true }).first().waitFor();

await page.getByTestId("new-type").click();
await page.locator("#tax-name").waitFor();
await page.fill("#tax-name", `QA Straggler ${SUF}`);
await page.fill("#tax-description", "Created by the Wave 12 lane-7 verification pass");
await shot("settings-taxonomy-create-dialog");
await page.getByTestId("save-taxonomy-node").click();
await page.getByText(`QA Straggler ${SUF}`).first().waitFor({ timeout: 12000 });
step("taxonomy type created in-browser", true, `QA Straggler ${SUF}`);
await shot("settings-taxonomy-created");

const row = page.locator("div.flex").filter({ hasText: `QA Straggler ${SUF}` }).first();
await row.getByRole("button", { name: "Retire" }).first().click();
await page.getByRole("heading", { name: "Retire node" }).waitFor({ timeout: 8000 });
await shot("settings-taxonomy-retire-dialog");
await page.getByTestId("confirm-retire").click();
await page.getByTestId("taxonomy-notice").waitFor({ timeout: 12000 });
step(
  "taxonomy soft-retire notice",
  (await page.getByTestId("taxonomy-notice").innerText()).includes("retired"),
);
await shot("settings-taxonomy-retired");

const move = page.getByRole("button", { name: "Move" }).first();
if (await move.count()) {
  await move.click();
  await page.getByRole("heading", { name: "Move subcategory" }).waitFor({ timeout: 8000 });
  await shot("settings-taxonomy-reparent-dialog");
  await page.keyboard.press("Escape");
  step("taxonomy reparent dialog", true);
}

// ---------------------------------------------------------------------------
// 2. Ticket board move — status bar + dialog + typed conflict
// ---------------------------------------------------------------------------
for (const theme of ["light", "dark"]) {
  await setTheme(theme);
  for (const width of [1440, 1024, 768]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`${BASE}/tickets/${ticket.id}`, { waitUntil: "domcontentloaded" });
    await page.getByTestId("ticket-status-bar").waitFor({ timeout: 25000 });
    await shot(`ticket-status-bar-${theme}-${width}`);
    if (width === 768) await overflowAt768(`ticket ${theme}`);
  }

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${BASE}/tickets/${ticket.id}`, { waitUntil: "domcontentloaded" });
  await page.getByTestId("ticket-status-bar").waitFor({ timeout: 25000 });
  await page.getByTestId("ticket-status-change").click();
  await page.getByTestId("ticket-move-board").waitFor({ timeout: 8000 });
  await shot(`ticket-status-menu-${theme}`);
  await page.getByTestId("ticket-move-board").click();
  await page.getByRole("heading", { name: "Move to another board" }).waitFor({ timeout: 8000 });
  await shot(`board-move-dialog-${theme}`);

  if (theme === "light" && boardC) {
    await page
      .locator(`[data-testid="move-board-target"] option[value="${boardC.id}"]`)
      .waitFor({ state: "attached", timeout: 8000 });
    await page.getByTestId("move-board-target").selectOption(boardC.id);
    await page.getByTestId("move-board-status").waitFor({ timeout: 8000 });
    await page.waitForTimeout(400);
    await shot("board-move-dialog-conflict-board");
    await page.getByTestId("confirm-move-board").click();
    await page.getByTestId("board-move-conflict").waitFor({ timeout: 12000 });
    step("typed board_move_status_conflict rendered inline", true);
    await shot("board-move-conflict");

    await page.getByTestId("board-move-conflict-closed").first().click();
    await page.getByTestId("confirm-move-board").click();
    await page
      .getByTestId("board-move-conflict")
      .waitFor({ state: "detached", timeout: 12000 })
      .catch(() => {});
    await page.waitForTimeout(1000);
    const t = await ownerToken();
    const moved = await (
      await fetch(`${API}/api/v1/tickets/${ticket.id}`, { headers: { authorization: `Bearer ${t}` } })
    ).json();
    step("retry with explicit status_id moved the board", moved.board_id === boardC.id, moved.board_id);
    await shot("board-move-completed");
  } else {
    // Pick whatever target board the dialog actually offers (excludes the
    // ticket's live board), then show the status picker.
    const select = page.getByTestId("move-board-target");
    await select.locator("option").first().waitFor({ state: "attached", timeout: 8000 });
    const values = await select
      .locator("option")
      .evaluateAll((os) => os.map((o) => o.value).filter(Boolean));
    if (values.length > 0) {
      await select.selectOption(values[0]);
      await page.getByTestId("move-board-status").waitFor({ timeout: 8000 });
      await page.waitForTimeout(400);
      await shot(`board-move-dialog-target-${theme}`);
    }
    await page.keyboard.press("Escape");
  }
}

// details-tab fallback (light + dark)
for (const theme of ["light", "dark"]) {
  await setTheme(theme);
  await page.goto(`${BASE}/tickets/${ticket.id}`, { waitUntil: "domcontentloaded" });
  await page.getByTestId("ticket-status-bar").waitFor({ timeout: 25000 });
  await page.getByTestId("detail-tab-details").click();
  await page.getByTestId("details-move-board").waitFor({ timeout: 8000 });
  await shot(`board-move-details-fallback-${theme}`);
  await page.getByTestId("details-move-board").click();
  await page.getByRole("heading", { name: "Move to another board" }).waitFor({ timeout: 8000 });
  await shot(`board-move-details-dialog-${theme}`);
  await page.keyboard.press("Escape");
}

await ctx.close();
await browser.close();
console.log(JSON.stringify({ shots, results }, null, 1));
