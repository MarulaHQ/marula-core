// Wave 43 track 2 lane 6 `p43-design-system-debt` — vision-review captures.
//
// Re-reviews the three changes this lane made, in both modes at the pack's
// recorded desktop viewport (1600x994, the size of the shipped mockup PNGs):
//   (A) `.m-btn-primary` now paints `--m-primary` (ink), not `--m-accent`
//   (D) the status danger pair aliases the kit breach pair; destructive
//       actions keep the solid `--destructive` red
//   (sm) the kit Button `sm` is density-aware (36/32/32) not a fixed 30px
//
// Comfortable captures (12 routes x 2 modes) plus a `dense` second pass on the
// two `sm` rows (rows 3 and 4 of REVIEW.md Part 2), plus the pack's own
// reference state copied in as row 15. 30 images, reviewed in <=8-image batches.
//
// Run with the stack env exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p43-design-system-debt-captures.mjs
import { chromium } from "playwright";
import { mkdirSync, copyFileSync, existsSync, writeFileSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:28480").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p43-design-system-debt/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

// Seeded rows (looked up from the stack at run time in a second pass, but the
// deterministic seed gives stable ids — fetched here to stay honest if reseeded).
const TIckET_FALLBACK = "6d657269-6469-616e-0000-000000000068"; // DEMO-1004
const CLIENT_FALLBACK = "6d657269-6469-616e-0000-000000000014"; // Acme Manufacturing

mkdirSync(OUT, { recursive: true });

const req = await (await import("playwright")).request.newContext();
const loginRes = await req.post(`${API}/api/v1/auth/login`, {
  data: { email: EMAIL, password: PASSWORD },
});
if (!loginRes.ok()) throw new Error(`api login failed: ${loginRes.status()}`);
const { access_token: token } = await loginRes.json();
const auth = (path, init = {}) =>
  req.fetch(`${API}/api/v1${path}`, {
    ...init,
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
      ...(init.headers ?? {}),
    },
  });

const firstId = (data, keys) => {
  const arr = Array.isArray(data) ? data : (data?.items ?? data?.[keys] ?? []);
  return arr?.[0]?.id ?? null;
};
const TICKET =
  firstId(await (await auth("/tickets?limit=1")).json(), "tickets") ?? TIckET_FALLBACK;
const CLIENT =
  firstId(await (await auth("/clients?limit=1")).json(), "clients") ?? CLIENT_FALLBACK;
console.log(`[captures] ticket=${TICKET} client=${CLIENT}`);

const loginWithRetry = async (page) => {
  for (let attempt = 0; attempt < 4; attempt += 1) {
    await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
    const submit = page.locator('button[type="submit"]');
    await submit.waitFor({ state: "visible", timeout: 30_000 });
    await page.waitForTimeout(1200);
    await page.fill('input[type="email"]', EMAIL);
    await page.fill('input[type="password"]', PASSWORD);
    await submit.click();
    try {
      await page.waitForURL(/tickets|today|dashboard|\/$/, { timeout: 45_000 });
      return;
    } catch {
      await page.waitForTimeout(5000);
    }
  }
  throw new Error("login failed after retries (auth gate may be throttling)");
};

const dismissTour = async (page) => {
  for (let step = 0; step < 10; step += 1) {
    const overlay = page.locator('[data-testid="tour-overlay"]');
    if (!(await overlay.isVisible().catch(() => false))) return;
    const stop = page.getByRole("button", { name: /don't show me tours/i }).first();
    const done = page.getByRole("button", { name: /^done$/i }).first();
    const next = page.getByRole("button", { name: /next/i }).first();
    for (const target of [stop, done, next]) {
      if (await target.isVisible().catch(() => false)) {
        await target.click({ timeout: 3000 }).catch(() => {});
        break;
      }
    }
    await page.waitForTimeout(500);
  }
};

const settle = async (page) => {
  await page.waitForLoadState("networkidle").catch(() => {});
  await page.waitForTimeout(1200);
};

// Capture a viewport shot via CDP: the app runs a JS-driven animation loop
// that never lets Playwright's stability wait settle, and CDP's
// Page.captureScreenshot does not wait for stability. Contexts also ask for
// reduced motion (the app honours prefers-reduced-motion) so the frames
// themselves are calm.
const cdpByPage = new WeakMap();
const shot = async (page, name) => {
  let client = cdpByPage.get(page);
  if (!client) {
    client = await page.context().newCDPSession(page);
    cdpByPage.set(page, client);
  }
  const { data } = await client.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(`${OUT}/${name}.png`, Buffer.from(data, "base64"));
};

const browser = await chromium.launch();
let shots = 0;
const ROUTES = [
  ["01-tickets", "/tickets"],
  ["02-ticket-detail", `/tickets/${TICKET}`],
  ["05-today", "/"],
  ["06-clients", "/clients"],
  ["07-client-detail", `/clients/${CLIENT}`],
  ["08-settings-appearance", "/settings/appearance"],
  ["09-settings-general", "/settings/general"],
  ["10-calendar", "/calendar"],
  ["11-dispatch", "/dispatch"],
  ["12-portal", "/portal"],
  ["14-alerts", "/alerts"],
];

for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({
    colorScheme: theme,
    reducedMotion: "reduce",
    viewport: { width: 1600, height: 994 },
  });
  // Resolve the device theme the way the app's bootstrap would; the seeded
  // account's mode is "system", so this is what it picks up.
  await ctx.addInitScript((t) => {
    try {
      localStorage.setItem("marula-theme", t);
    } catch {}
  }, theme);
  const page = await ctx.newPage();
  page.on("pageerror", (err) => console.error(`[pageerror ${theme}]`, err.message));

  // 13. sign-in — before auth, so the public route renders.
  await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
  await settle(page);
  await shot(page, `${theme}-13-login`);
  shots += 1;

  await loginWithRetry(page);
  await page.goto(`${WEB}/`);
  await page.waitForSelector('[data-testid="tour-overlay"]', { timeout: 12_000 }).catch(() => {});
  await dismissTour(page);

  for (const [name, route] of ROUTES) {
    await page.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded" });
    await settle(page);
    await shot(page, `${theme}-${name}`);
    shots += 1;
  }

  // Dense second pass on the sm rows (3 tickets list, 4 ticket detail).
  // The account's saved density is `comfortable`, so AppearanceSync re-applies
  // it on mount; set the attribute after settle() so the step actually sticks.
  for (const [name, route] of [
    ["03-tickets-dense", "/tickets"],
    ["04-ticket-detail-dense", `/tickets/${TICKET}`],
  ]) {
    await page.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded" });
    await settle(page);
    await page.evaluate(() =>
      document.documentElement.setAttribute("data-density", "dense"),
    );
    await page.waitForTimeout(400);
    await shot(page, `${theme}-${name}`);
    shots += 1;
  }

  await ctx.close();
}

// 15. the pack's own reference state, both modes (the target the rest is
// judged against). Copied so the evidence dir is self-contained.
const PACK = "design/mockups/marula-app-mockup-images";
const packRefs = [
  ["light-15-pack-reference.png", `${PACK}/mobile/light/states/01-board-list.png`],
  ["dark-15-pack-reference.png", `${PACK}/mobile/dark/states/01-board-list.png`],
];
for (const [dest, src] of packRefs) {
  if (existsSync(src)) {
    copyFileSync(src, `${OUT}/${dest}`);
    shots += 1;
  } else {
    console.error(`[captures] pack reference missing: ${src}`);
  }
}

await browser.close();
console.log(`[captures] wrote ${shots} images to ${OUT}`);
