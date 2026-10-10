// Wave 44 lane 5 (`p44-ui-money-sales`) — capture the money cluster for the
// vision review against the Marula surface pack.
//
// Purpose: a drawn mockup and a built page are different kinds of evidence.
// The pack's `web/approvals/` ten states are the SPEC; only a render shows
// whether a pill reads as a pill, whether the bulk bar replaces the header,
// and whether a colour says the right thing. This captures the seven money
// routes PLUS the interaction states the resting pages never show (bulk
// selection, reject dialog, record-payment dialog, invoice paid, empty
// queue) in both drawn themes.
//
// Non-purpose: not an E2E suite and not a regression gate — nothing here
// asserts; it writes PNGs. Read-only against the API: it logs in, GETs, and
// only interacts to bring a state to the screen. The one mutation (record
// payment) runs against a local dev stack with a demo seed, not production.
//
// SEED GAP: the demo seed writes no time-entry approval_status, so every seeded
// entry sits at `draft` and `/approvals` renders its empty state — none of the
// pack's three queue states can be reached. Before running this battery on a
// fresh stack, push entries into the queue the way a tech does:
//   POST /api/v1/time-entries/{id}/submit  ({} as the body; it needs JSON)
// for entries NOT owned by the capture seat, because the pack's own-entry rule
// keeps the approver's own time out of the bulk pass. Twelve is enough.
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const WEB = process.env.CAPTURE_WEB ?? "http://localhost:3013";
const OUT = process.env.CAPTURE_OUT ?? "design/qa-p44-ui-money-sales/captures";
const EMAIL = process.env.CAPTURE_EMAIL ?? "owner@demo.local";
const PASSWORD = process.env.CAPTURE_PASSWORD ?? "demo-password-123";

/** The seven routes, with a file stem each. */
const ROUTES = [
  { stem: "approvals", path: "/approvals" },
  { stem: "billing", path: "/billing" },
  { stem: "invoice-detail", path: "/billing/INVOICE_ID" },
  { stem: "credit-notes", path: "/billing/credit-notes" },
  { stem: "pipeline", path: "/sales/pipeline" },
  { stem: "quotes", path: "/sales/quotes" },
  { stem: "quote-detail", path: "/sales/quotes/QUOTE_ID" },
  { stem: "catalog", path: "/sales/catalog" },
];

/** The pack's drawn themes: the light neutral surface and the dark one. */
const THEMES = [
  { name: "neutral", scheme: "light" },
  { name: "dark", scheme: "dark" },
];

const DESKTOP = { width: 1600, height: 1000 };

async function login(page) {
  // networkidle, not domcontentloaded: the form is React-controlled, and a
  // click before hydration does nothing at all — which looks exactly like a
  // failed login.
  await page.goto(`${WEB}/login`, { waitUntil: "networkidle" });
  await page.waitForSelector('input[type="email"]', { state: "visible" });
  await page.fill('input[type="email"]', EMAIL);
  await page.fill('input[type="password"]', PASSWORD);
  await page.click('button[type="submit"]');
  // The app routes client-side, so there is no document navigation to wait
  // for — `waitForURL` would sit there until it timed out. The shell only
  // renders once the session lands, so waiting on it is the honest signal.
  await page.waitForFunction(() => !location.pathname.startsWith("/login"), null, {
    timeout: 30_000,
  });
  await page.waitForLoadState("networkidle");
}

/** The first invoice id, read from the ledger so the detail shot is real. */
async function firstInvoiceId(page) {
  await page.goto(`${WEB}/billing`, { waitUntil: "networkidle" });
  const href = await page
    .locator('a[href^="/billing/"]:not([href$="/credit-notes"])')
    .first()
    .getAttribute("href")
    .catch(() => null);
  return href === null ? null : href.replace("/billing/", "");
}

/** The first quote id, read off the quote ledger's row link (which is the way
 *  into the document, per canonical Q01). */
async function firstQuoteId(page) {
  await page.goto(`${WEB}/sales/quotes`, { waitUntil: "networkidle" });
  const href = await page
    .locator('a[href^="/sales/quotes/"]')
    .first()
    .getAttribute("href")
    .catch(() => null);
  return href === null ? null : href.replace("/sales/quotes/", "");
}

/** A route shot with the settle wait, so the shot is the resting state. */
async function shot(page, target, file) {
  await page.goto(`${WEB}${target}`, { waitUntil: "networkidle" });
  await page.waitForTimeout(700);
  await page.screenshot({ path: file, fullPage: false });
  console.log(`-> ${file}`);
}

/** Interaction states, both themes. Each: bring the state up, shoot it.
 *  `terminal` runs the one-way states (approve-everything, record payment)
 *  and must be set for the LAST pass only — they leave the demo data spent. */
async function states(context, themeName, invoiceId, terminal) {
  const page = await context.newPage();
  page.on("pageerror", (e) => console.error(`[${themeName}] page error:`, e.message));
  await login(page);
  const file = (s) => path.join(OUT, `${themeName}-${s}.png`);

  // 01 queue (resting) + 03 bulk pass: select a couple of rows.
  await page.goto(`${WEB}/approvals`, { waitUntil: "networkidle" });
  await page.waitForTimeout(700);
  await page.screenshot({ path: file("approvals"), fullPage: false });
  console.log(`-> ${file("approvals")}`);
  const rows = page.locator('[data-testid^="approval-row-"] input[type="checkbox"]:not([disabled])');
  if ((await rows.count()) >= 2) {
    await rows.nth(0).click();
    await rows.nth(1).click();
    await page.waitForTimeout(300);
    await page.screenshot({ path: file("approvals-bulk"), fullPage: false });
    console.log(`-> ${file("approvals-bulk")}`);
  } else {
    console.warn(`[${themeName}] fewer than 2 selectable rows — skipping approvals-bulk`);
  }

  // 05 reject dialog: from the bulk bar.
  if ((await page.locator('[data-testid="bulk-reject"]').count()) > 0) {
    await page.locator('[data-testid="bulk-reject"]').click();
    await page.waitForTimeout(400);
    await page.screenshot({ path: file("approvals-reject"), fullPage: false });
    console.log(`-> ${file("approvals-reject")}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(200);
  }

  // 04 review-flagged: the strip's Review all leaves only the flagged rows
  // and offers the way back.
  const reviewAll = page.locator('button:has-text("Review all")');
  if ((await reviewAll.count()) > 0) {
    await reviewAll.first().click();
    await page.waitForTimeout(500);
    await page.screenshot({ path: file("approvals-reviewed"), fullPage: false });
    console.log(`-> ${file("approvals-reviewed")}`);
    const showAll = page.locator('button:has-text("Show all")');
    if ((await showAll.count()) > 0) {
      await showAll.first().click();
      await page.waitForTimeout(500);
    }
  }

  // 07 empty queue: the bulk pass decided in context — tick every row (the
  // bulk bar's own checkbox deselects, so the full pass is per-row), approve
  // the lot, and shoot what a cleared queue says.
  if (terminal) {
    const allRows = page.locator('[data-testid^="approval-row-"] input[type="checkbox"]:not([disabled])');
    const rowCount = await allRows.count();
    for (let i = 0; i < rowCount; i++) {
      const box = allRows.nth(i);
      if (!(await box.isChecked())) await box.click();
    }
    await page.waitForTimeout(300);
    const approveAll = page.locator('[data-testid="bulk-approve"]');
    if ((await approveAll.count()) > 0) {
      const label = (await approveAll.first().innerText()).trim();
      await approveAll.first().click();
      await page.waitForTimeout(1500);
      await page.screenshot({ path: file("approvals-empty"), fullPage: false });
      console.log(`-> ${file("approvals-empty")} (${label})`);
    } else {
      console.warn(`[${themeName}] ${rowCount} rows ticked but no bulk bar — skipping empty`);
    }
  }

  // 08/09/10 invoice detail + record-payment dialog + the paid result.
  if (invoiceId !== null) {
    await page.goto(`${WEB}/billing/${invoiceId}`, { waitUntil: "networkidle" });
    await page.waitForTimeout(700);
    await page.screenshot({ path: file("invoice-detail"), fullPage: false });
    console.log(`-> ${file("invoice-detail")}`);
    const rec = page.locator('[data-testid="record-payment"]');
    if ((await rec.count()) > 0 && (await rec.first().isEnabled())) {
      await rec.first().click();
      await page.waitForTimeout(400);
      await page.screenshot({ path: file("invoice-record-payment"), fullPage: false });
      console.log(`-> ${file("invoice-record-payment")}`);
      // 10: the dialog's amount opens holding the balance — record it whole
      // and shoot the invoice the pack draws paid.
      const save = page.locator('[data-testid="record-payment-save"]');
      if (terminal && (await save.count()) > 0 && (await save.first().isEnabled())) {
        await save.first().click();
        await page.waitForTimeout(1500);
        await page.screenshot({ path: file("invoice-paid"), fullPage: false });
        console.log(`-> ${file("invoice-paid")}`);
      }
    } else {
      console.warn(`[${themeName}] Record payment not enabled on this invoice — skipping`);
    }
  } else {
    console.warn(`[${themeName}] no invoice to shoot — skipping invoice states`);
  }
  await page.close();
}

async function main() {
  await mkdir(OUT, { recursive: true });
  const browser = await chromium.launch();
  const written = [];
  // Resolve the invoice id once, before the themes: both passes shoot the
  // same record, so the two themes' detail shots are of the same invoice.
  let invoiceId = null;
  let quoteId = null;
  {
    const context = await browser.newContext({
      viewport: DESKTOP,
      colorScheme: "light",
      deviceScaleFactor: 2,
    });
    const page = await context.newPage();
    await login(page);
    invoiceId = await firstInvoiceId(page);
    console.log(`invoice id for detail shots: ${invoiceId ?? "NONE"}`);
    quoteId = await firstQuoteId(page);
    console.log(`quote id for detail shots: ${quoteId ?? "NONE"}`);
    await context.close();
  }

  for (const [index, theme] of THEMES.entries()) {
    const context = await browser.newContext({
      viewport: DESKTOP,
      colorScheme: theme.scheme,
      deviceScaleFactor: 2,
    });
    // The one-way states (approve everything, record the payment) run in the
    // final pass only: they spend the demo data for every later shot.
    await states(context, theme.name, invoiceId, index === THEMES.length - 1);
    await context.close();
  }

  // The six remaining resting routes, both themes.
  for (const theme of THEMES) {
    const context = await browser.newContext({
      viewport: DESKTOP,
      colorScheme: theme.scheme,
      deviceScaleFactor: 2,
    });
    const page = await context.newPage();
    page.on("pageerror", (e) => console.error(`[${theme.name}] page error:`, e.message));
    await login(page);
    for (const route of ROUTES) {
      if (["approvals", "invoice-detail"].includes(route.stem)) continue;
      const raw =
        route.path === "/billing/INVOICE_ID"
          ? (invoiceId === null ? null : `/billing/${invoiceId}`)
          : route.path === "/sales/quotes/QUOTE_ID"
            ? (quoteId === null ? null : `/sales/quotes/${quoteId}`)
            : route.path;
      if (raw === null) {
        console.warn(`[${theme.name}] no record to shoot — skipping ${route.stem}`);
        continue;
      }
      await shot(page, raw, path.join(OUT, `${theme.name}-${route.stem}.png`));
      written.push(raw);
    }
    await context.close();
  }

  await browser.close();
  console.log(`\nmoney capture battery done (${written.length} routes + states) -> ${OUT}`);
}

await main();
