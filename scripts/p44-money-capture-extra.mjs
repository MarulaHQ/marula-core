// Lane-local extra states: the two pack states the demo seed cannot reach on
// its own — the OVERDUE invoice (the seed's due dates are all in the future,
// so the blush ageing pill never paints) and the CLEARED queue (the owner's
// own two entries stay out of the owner's bulk pass, so only another approver
// can empty it). Both mutate local-dev state and are captured under their own
// filenames, never over a reviewed capture.
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const WEB = process.env.CAPTURE_WEB ?? "http://localhost:3013";
const API = process.env.CAPTURE_API ?? "http://localhost:28581";
const OUT = process.env.CAPTURE_OUT ?? "design/qa-p44-ui-money-sales/captures";
const INVOICE_ID = process.env.CAPTURE_INVOICE ?? "6d657269-6469-616e-0000-0000000000c9";

async function apiToken(email) {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password: "demo-password-123" }),
  });
  const body = await res.json();
  return body.access_token ?? body.token;
}

async function login(page, email) {
  await page.goto(`${WEB}/login`, { waitUntil: "networkidle" });
  await page.waitForSelector('input[type="email"]', { state: "visible" });
  await page.fill('input[type="email"]', email);
  await page.fill('input[type="password"]', "demo-password-123");
  await page.click('button[type="submit"]');
  await page.waitForFunction(() => !location.pathname.startsWith("/login"), null, {
    timeout: 30000,
  });
  await page.waitForLoadState("networkidle");
}

const THEMES = [
  { name: "neutral", scheme: "light" },
  { name: "dark", scheme: "dark" },
];

await mkdir(OUT, { recursive: true });
const browser = await chromium.launch();

// ---- 1. the overdue invoice (pack 08: blush pill with the days in words) ----
for (const theme of THEMES) {
  const context = await browser.newContext({
    viewport: { width: 1600, height: 1000 },
    colorScheme: theme.scheme,
    deviceScaleFactor: 2,
  });
  const page = await context.newPage();
  await login(page, "owner@demo.local");
  await page.goto(`${WEB}/billing`, { waitUntil: "networkidle" });
  await page.waitForTimeout(700);
  await page.screenshot({ path: path.join(OUT, `${theme.name}-billing-overdue.png`) });
  console.log(`-> ${theme.name}-billing-overdue.png`);
  await page.goto(`${WEB}/billing/${INVOICE_ID}`, { waitUntil: "networkidle" });
  await page.waitForTimeout(700);
  await page.screenshot({ path: path.join(OUT, `${theme.name}-invoice-detail-overdue.png`) });
  console.log(`-> ${theme.name}-invoice-detail-overdue.png`);
  await context.close();
}

// ---- 2. the cleared queue (pack 07): the dispatcher signs off the owner's own ----
const dispatcher = await apiToken("bookkeeper@demo.local");
const queue = await fetch(`${API}/api/v1/time-entries/approvals/queue?status=submitted`, {
  headers: { authorization: `Bearer ${dispatcher}` },
}).then((r) => r.json());
const items = Array.isArray(queue) ? queue : (queue.items ?? []);
const ids = items.map((i) => i.entry?.id ?? i.id);
for (const id of ids) {
  await fetch(`${API}/api/v1/time-entries/approvals/decide`, {
    method: "POST",
    headers: { authorization: `Bearer ${dispatcher}`, "content-type": "application/json" },
    body: JSON.stringify({ entry_ids: [id], decision: "approve" }),
  });
}
console.log(`bookkeeper approved ${ids.length} remaining entries`);
for (const theme of THEMES) {
  const context = await browser.newContext({
    viewport: { width: 1600, height: 1000 },
    colorScheme: theme.scheme,
    deviceScaleFactor: 2,
  });
  const page = await context.newPage();
  await login(page, "owner@demo.local");
  await page.goto(`${WEB}/approvals`, { waitUntil: "networkidle" });
  await page.waitForTimeout(700);
  await page.screenshot({ path: path.join(OUT, `${theme.name}-approvals-cleared.png`) });
  console.log(`-> ${theme.name}-approvals-cleared.png`);
  await context.close();
}

await browser.close();
console.log("extra states done");
