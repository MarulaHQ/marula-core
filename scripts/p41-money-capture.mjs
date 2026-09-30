// Wave 41 lane 8 (`p41-ui-money`) — capture the money cluster for the
// vision review against the Marula surface pack.
//
// Purpose: a drawn mockup and a built page are different kinds of evidence.
// The pack's `what_to_notice` text and source HTML are a SPEC; only a render
// shows whether a pill reads as a pill, whether the gutter is too tight, and
// whether a colour says the right thing. This captures the seven money routes
// in both drawn themes so they can be compared against
// brand/marula-mockups/marula-surface-pack/web/{approvals,billing,...}.
//
// Non-purpose: not an E2E suite and not a regression gate — nothing here
// asserts; it writes PNGs and a contact sheet, and a human (or a vision
// pass) looks at them. Read-only against the API: it logs in and GETs.
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const WEB = process.env.CAPTURE_WEB ?? "http://localhost:3013";
const OUT = process.env.CAPTURE_OUT ?? "design/qa-p41-ui-money/captures";
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

async function main() {
  await mkdir(OUT, { recursive: true });
  // The API this capture points at is a neighbour lane's stack, whose
  // CORS_ORIGIN is that lane's own web port — not this worktree's. The
  // browser is throwaway and this is a local read-only capture, so the
  // origin check is disabled for the session rather than mutating another
  // lane's configuration. Nothing here is shipped.
  const browser = await chromium.launch({
    args: ["--disable-web-security", "--disable-features=IsolateOrigins,site-per-process"],
  });
  const written = [];
  // Resolved once, from the light pass: the dark pass reuses it rather than
  // re-reading the ledger (and rather than tripping over its own scope).
  let invoiceId = null;

  for (const theme of THEMES) {
    const context = await browser.newContext({
      viewport: DESKTOP,
      colorScheme: theme.scheme,
      deviceScaleFactor: 2,
    });
    const page = await context.newPage();
    page.on("pageerror", (e) => console.error(`[${theme.name}] page error:`, e.message));
    await login(page);

    if (invoiceId === null) invoiceId = await firstInvoiceId(page);
    for (const route of ROUTES) {
      const target = route.path === "/billing/INVOICE_ID" && invoiceId !== null
        ? `/billing/${invoiceId}`
        : route.path === "/billing/INVOICE_ID"
          ? null
          : route.path;
      if (target === null) {
        console.warn(`[${theme.name}] no invoice to shoot — skipping ${route.stem}`);
        continue;
      }
      await page.goto(`${WEB}${target}`, { waitUntil: "networkidle" });
      // Let the density + entrance motion settle so the shot is the resting state.
      await page.waitForTimeout(700);
      const file = path.join(OUT, `${theme.name}-${route.stem}.png`);
      await page.screenshot({ path: file, fullPage: false });
      written.push(file);
      console.log(`[${theme.name}] ${target} -> ${file}`);
    }
    await context.close();
  }

  await browser.close();
  console.log(`\n${written.length} captures written to ${OUT}`);
}

await main();
