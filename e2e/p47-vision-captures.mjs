// p47-vision-captures.mjs — Wave 47 lane A13, `p47-vision-captures`.
//
// The capture debt the B-track left behind. B1 (`design/qa-p47-demo-up/`), B2
// (`design/qa-p47-byok-runbook/`) and B3 (`design/qa-p47-demo-data/`) all landed
// with their data, api and boot paths proven and no screenshot evidence, so
// their landed records describe surfaces nobody has looked at.
//
// This harness boots nothing. It expects `scripts/e2e-stack.sh up` plus the
// built web bundle, and reads every port from `e2e/.e2e-stack.env` — never set
// one by hand (p37 doctrine).
//
// What one run produces:
//
//   1. CANONICAL SHOTS — every surface the row names, both themes and every
//      portal login, shot at the pack's recorded desktop size (1600x994, from
//      `design/mockups/marula-app-mockup-images/manifest.json` and mirrored by
//      `e2e/p41-audit-captures.mjs:254`). These are the images the vision
//      review reads against the pack.
//   2. THE RESPONSIVE MATRIX — every surface measured at five desktop sizes in
//      both themes, with mechanical checks rather than an eyeball: horizontal
//      page scroll, content overhanging the right edge, the nav rail
//      overlapping the content column, a missing `main`, an empty surface. A
//      size that breaks is named in the matrix and the summary table.
//   3. EXTREME SHOTS — the narrowest and widest desktop sizes for a subset of
//      the highest-value surfaces, so the extremes are looked at, not only
//      measured.
//
// The row's batching rule is binding: an evaluation request never carries more
// than 30 images, and this lane reviews in batches of at most 8, recording a
// verdict line per batch in `REVIEW.md` as it goes. The manifest this run
// writes groups the images into those batches so the review consumes them in
// order without re-deriving the split.
//
// Usage:
//   node e2e/p47-vision-captures.mjs              # everything
//   SHOTS=canonical node e2e/p47-vision-captures.mjs
//   SHOTS=matrix node e2e/p47-vision-captures.mjs
//   SHOTS=extreme node e2e/p47-vision-captures.mjs
//   ONLY=<capture-id>,<capture-id> node e2e/p47-vision-captures.mjs
import { chromium } from "playwright";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

/* The evidence directory. Defaults to the audit's own lane; a follow-up lane
   that re-runs the matrix sets LANE so it writes its own manifest and shots
   instead of overwriting the audit it is verifying against. */
const LANE = process.env.LANE ?? "design/qa-p47-vision-captures";
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
const API = (process.env.E2E_API_URL ?? env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");

/* ---------------------------------------------------------------------------
 * Desktops. `pack` is the reference size every canonical shot is taken at.
 * The rest are the real sizes a laptop, a scaled display and an ultrawide
 * actually are, so "does it appear properly" is answered against more than
 * one column width.
 * ---------------------------------------------------------------------- */
const VIEWPORTS = [
  { id: "compact", width: 1280, height: 800 },
  { id: "laptop", width: 1440, height: 900 },
  { id: "pack", width: 1600, height: 994 },
  { id: "desktop", width: 1920, height: 1080 },
  { id: "ultrawide", width: 2560, height: 1080 },
];
const PACK = VIEWPORTS.find((v) => v.id === "pack");
const EXTREMES = VIEWPORTS.filter((v) => v.id === "compact" || v.id === "ultrawide");

const STAFF_EMAIL = "owner@demo.local";
const STAFF_PASSWORD = "demo-password-123";

/* The three client seats lane B3 seeded, one per client
 * (`crates/core-db/src/demo.rs:528-550`), all on the shared demo password. */
const PORTALS = [
  { id: "acme", email: "portal@acme.demo", client: "Acme Manufacturing" },
  { id: "northwind", email: "portal@northwind.demo", client: "Northwind Legal" },
  { id: "globex", email: "portal@globex.demo", client: "Globex Foods" },
];
const PORTAL_PASSWORD = "demo-password-123";

const manifest = { canonical: [], extreme: [], matrix: [], problems: [], batches: [] };
const problems = [];
const consoleErrors = [];
const only = process.env.ONLY ? new Set(process.env.ONLY.split(",").map((s) => s.trim())) : null;
const scope = process.env.SHOTS ?? "all";
const wants = (what) => scope === "all" || scope === what;

const log = (line) => console.log(line);
const warn = (line) => {
  problems.push(line);
  console.log(`  ! ${line}`);
};

/* ---------------------------------------------------------------------------
 * The surfaces. Each entry is one surface the row names, plus the anchor that
 * proves the page actually rendered (a capture of an error boundary or an
 * empty shell is worse than no capture). Flat templates keep it honest: no
 * entry branches on what it finds.
 * ---------------------------------------------------------------------- */
const STAFF_SURFACES = [
  {
    id: "byok-checklist",
    path: "/settings/ai",
    anchor: '[data-testid="settings-ai-page"]',
    why: "the BYOK checklist an operator works before a prospect arrives (B2)",
  },
  {
    id: "tickets-list",
    path: "/tickets",
    anchor: '[aria-label="Ticket views"]',
    why: "the queue, carrying the P1-P4 SLA countdowns (B3 item 2)",
  },
  {
    id: "approvals-queue",
    path: "/approvals",
    anchor: '[data-testid^="approval-row-"]',
    why: "the manager queue with the submitted entry (B3 item 3)",
  },
  {
    id: "billing-ledger",
    path: "/billing",
    anchor: '[data-testid="billing-candidates"]',
    why: "the ledger's four invoice states: draft, issued, paid, overdue (B3 item 1)",
  },
  {
    id: "assets-warranty",
    path: "/assets",
    anchor: '[data-testid^="assets-open-"]',
    why: "contrasting warranty states and the internal-org asset (B3 item 7)",
  },
  {
    id: "kb-articles",
    path: "/kb",
    anchor: '[data-testid="kb-article-table"]',
    why: "the KB articles a client's own staff read (B3 item 6)",
  },
  {
    id: "email-inbox",
    path: "/settings/email",
    anchor: '[data-testid="mail-org-greeting"], [data-testid="mail-not-configured"]',
    why: "the email inbox and outbound queue",
  },
  {
    id: "secrets-manager",
    path: "/kb/secrets",
    anchor: '[data-testid="kb-secrets-table"]',
    why: "the secrets manager",
  },
  {
    id: "csat",
    path: "/settings/surveys",
    anchor: '[data-testid="csat-min-interval"]',
    why: "CSAT configuration, the survey a resolved ticket mails out",
  },
  {
    id: "purchasing",
    path: "/purchasing",
    anchor: '[data-testid="purchasing-new"]',
    why: "the purchasing completion legs",
  },
];

/* The ticket workspace, shot twice: the Files tab is the only human proof the
 * attachment storage seam works (B3's byte-identical download is a sha256
 * comparison, not something a person can see). */
const TICKET_FILES_SURFACE = {
  id: "ticket-files-attachment",
  act: async (page) => {
    await page.getByTestId("detail-tab-files").click();
  },
  anchor: '[data-testid="detail-panel-files"]',
  why: "the attachment on the ticket, the visible half of B3's download proof",
};
const TICKET_ACTIVITY_SURFACE = {
  id: "ticket-detail",
  anchor: '[data-testid="ticket-status-bar"]',
  why: "the ticket workspace, where the day's work is read",
};

/* Portal surfaces, per client. The ticket list is the B3 acceptance item that
 * needed a seat per client, so all three are shot. */
const PORTAL_SURFACES = [
  { id: "portal-tickets", path: "/portal/tickets", anchor: '[data-testid="portal-tickets-open"]', perClient: true, why: "the per-client ticket list, from each of the three logins (B3 item 5)" },
  { id: "portal-quotes", path: "/portal/quotes", anchor: "h1.portal-title", clients: ["acme", "northwind", "globex"], why: "the sent quote awaiting the client's decision (B3 item 4)" },
  { id: "portal-invoices", path: "/portal/invoices", anchor: '[data-testid="portal-owes"], .portal-row', clients: ["acme", "globex"], why: "the invoice states a client sees, including the overdue one" },
];

const THEMES = [
  { id: "light", portal: "sand" },
  { id: "dark", portal: "dark" },
];

/* ---------------------------------------------------------------------------
 * Auth. Staff ride the same path the E2E suite does: log in through the
 * context's cookie jar and seed the token pair into localStorage, which is
 * what the app's AuthProvider reads on bootstrap. Portal signs in through its
 * own in-page card, which is lane A1's fix and the reason half these surfaces
 * are reachable at all.
 * ---------------------------------------------------------------------- */
async function staffToken() {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: STAFF_EMAIL, password: STAFF_PASSWORD }),
  });
  if (!res.ok) throw new Error(`staff login failed (${res.status})`);
  return res.json();
}

async function staffContext(browser, { theme, width, height }) {
  const ctx = await browser.newContext({ viewport: { width, height }, colorScheme: theme });
  const tokens = await staffToken();
  // Cookie jar, so the app's rotation path owns a real session.
  const login = await ctx.request.post(`${API}/api/v1/auth/login`, {
    data: { email: STAFF_EMAIL, password: STAFF_PASSWORD },
  });
  if (!login.ok()) throw new Error(`context login failed (${login.status()})`);
  // Tours are per-user server state the stack script resets every boot, so the
  // first surface visited would otherwise carry the modal. Same switch the E2E
  // harness throws (e2e/helpers/harness.ts:73).
  await ctx.request.put(`${API}/api/v1/settings/user`, {
    data: { key: "tours.disabled", value: true },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  await seedTokens(ctx, tokens, theme);
  return { ctx, tokens };
}

/** The token pair plus both theme keys go in through an init script the
 *  context applies to every page it opens. */
async function seedTokens(ctx, tokens, theme) {
  await ctx.addInitScript(
    ([key, expKey, themeKey, portalThemeKey, raw, exp, th]) => {
      window.localStorage.setItem(key, raw);
      window.localStorage.setItem(expKey, exp);
      window.localStorage.setItem(themeKey, th === "dark" ? "dark" : "light");
      window.localStorage.setItem(portalThemeKey, th === "dark" ? "dark" : "sand");
      window.localStorage.setItem("marula-theme-preset", "marula");
    },
    [
      "marula-tokens",
      "marula-tokens-exp",
      "marula-theme",
      "marula-portal-theme",
      JSON.stringify(tokens),
      String(Date.now() + tokens.expires_in * 1000 - 30_000),
      theme,
    ],
  );
}

/** A portal context signed in through the real card (lane A1's path). */
async function portalContext(browser, { email, theme, width, height }) {
  // `colorScheme` takes dark|light only; the portal's sand mode is the app's own
  // localStorage key, set below.
  const ctx = await browser.newContext({
    viewport: { width, height },
    colorScheme: theme === "dark" ? "dark" : "light",
  });
  await ctx.addInitScript((t) => {
    window.localStorage.setItem("marula-portal-theme", t);
    window.localStorage.setItem("marula-theme", t === "dark" ? "dark" : "light");
  }, theme);
  const page = await ctx.newPage();
  await page.goto(`${WEB}/portal/login`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.waitForSelector('[data-testid="portal-use-password"]', { timeout: 60_000 });
  await page.click('[data-testid="portal-use-password"]');
  await page.fill("#portal-email", email);
  await page.fill("#portal-password", PORTAL_PASSWORD);
  await page.click('[data-testid="portal-password-submit"]');
  await page.waitForSelector('[data-testid="portal-tickets-open"]', { timeout: 60_000 });
  // Close the sign-in page: while it is open its provider can still be
  // mid-restore, and a second page rotating the same refresh pair trips the
  // server's replay guard (a phantom gate). Recorded by lane A1.
  await page.close();
  return ctx;
}

/* ---------------------------------------------------------------------------
 * Rendering. The shell can re-navigate itself once after load (the layout's
 * link prefetch), so every capture settles before it shoots, and any overlay
 * (the docs tour, a dialog) is dismissed first.
 * ---------------------------------------------------------------------- */
async function settle(page, anchor) {
  if (anchor) {
    await page.waitForSelector(anchor, { timeout: 30_000 }).catch(() => {
      warn(`${anchor} did not render`);
    });
  }
  await page.waitForLoadState("networkidle", { timeout: 20_000 }).catch(() => {});
  for (const name of [/^done$/i, /^got it$/i, /^finish$/i, /^close$/i]) {
    const b = page.getByRole("button", { name }).first();
    if ((await b.count()) && (await b.isVisible().catch(() => false))) {
      await b.click({ timeout: 4000 }).catch(() => {});
    }
  }
  await page.keyboard.press("Escape").catch(() => {});
  await page.waitForTimeout(1200);
}

/** The mechanical checks. Every one is a thing a person would call "it did not
 *  appear properly", measured rather than guessed. */
async function measure(page) {
  return page.evaluate(() => {
    const de = document.documentElement;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const main = document.querySelector("main");
    const rail = document.querySelector('nav[aria-label="Main"]');
    const box = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height), r: Math.round(r.right), b: Math.round(r.bottom) };
    };
    // The widest visible element inside the content column that crosses the
    // right edge of the window: the signature of a surface laid out for a
    // wider screen than the one it is on.
    let overhang = 0;
    let overhangSel = "";
    if (main) {
      for (const el of main.querySelectorAll("*")) {
        const r = el.getBoundingClientRect();
        if (r.width < 2 || r.height < 2) continue;
        const over = r.right - vw;
        if (over > overhang) {
          overhang = Math.round(over);
          const testid = el.getAttribute("data-testid");
          overhangSel = `${el.tagName.toLowerCase()}${testid ? `[data-testid="${testid}"]` : el.className ? `.${String(el.className).split(/\s+/)[0]}` : ""}`;
        }
      }
    }
    const mb = box(main);
    const rb = box(rail);
    const overlapArea = (a, b) => {
      if (!a || !b) return 0;
      const w = Math.min(a.r, b.r) - Math.max(a.x, b.x);
      const h = Math.min(a.b, b.b) - Math.max(a.y, b.y);
      return w > 0 && h > 0 ? Math.round(w * h) : 0;
    };
    return {
      vw,
      vh,
      hScroll: de.scrollWidth - de.clientWidth,
      overhang,
      overhangSel,
      railOverlap: overlapArea(mb, rb),
      hasMain: main !== null,
      textLen: (document.body.innerText ?? "").trim().length,
    };
  });
}

function judge(metrics, where) {
  const bad = [];
  if (metrics.hScroll > 1) bad.push(`horizontal page scroll of ${metrics.hScroll}px`);
  if (metrics.overhang > 2) bad.push(`content overhangs the right edge by ${metrics.overhang}px (${metrics.overhangSel})`);
  if (metrics.railOverlap > 0) bad.push(`the nav rail overlaps the content by ${metrics.railOverlap}px²`);
  if (!metrics.hasMain) bad.push("no main region rendered");
  if (metrics.textLen < 40) bad.push(`the surface looks empty (${metrics.textLen} chars of text)`);
  for (const b of bad) warn(`${where}: ${b}`);
  return { where, ...metrics, bad };
}

/* ---------------------------------------------------------------------------
 * The run.
 * ---------------------------------------------------------------------- */
const browser = await chromium.launch();

/** The ticket the ticket-shaped surfaces are captured on. Resolved once, from
 *  the API, because the seed's first ticket is not the one carrying an
 *  attachment and a hardcoded id would go stale silently. */
let attachmentTicketId = null;
async function resolveTicketWithAttachment() {
  if (attachmentTicketId !== null) return attachmentTicketId;
  const tokens = await staffToken();
  const authed = async (path) => {
    const res = await fetch(`${API}/api/v1${path}`, {
      headers: { authorization: `Bearer ${tokens.access_token}` },
    });
    return res.ok ? res.json() : null;
  };
  const tickets = (await authed("/tickets?limit=100")) ?? [];
  const list = Array.isArray(tickets) ? tickets : (tickets.items ?? []);
  for (const t of list) {
    const attachments = await authed(`/tickets/${t.id}/attachments`);
    if (Array.isArray(attachments) && attachments.length > 0) {
      log(`[tickets] attachment ticket: ${t.number} (${attachments.length} attachment(s))`);
      attachmentTicketId = t.id;
      return attachmentTicketId;
    }
  }
  warn("no seeded ticket carries an attachment; the attachment capture cannot be taken");
  return null;
}

/* 1. Canonical shots — every staff surface, both themes, at the pack's size. */
if (wants("canonical") || wants("matrix")) {
  await resolveTicketWithAttachment();
}

if (wants("canonical")) {
  const surfaces = [...STAFF_SURFACES];
  if (attachmentTicketId !== null) {
    surfaces.push({ ...TICKET_FILES_SURFACE, path: `/tickets/${attachmentTicketId}` });
    surfaces.push({ ...TICKET_ACTIVITY_SURFACE, path: `/tickets/${attachmentTicketId}` });
  }

  for (const theme of THEMES) {
    log(`[canonical] staff, ${theme.id} theme, ${PACK.width}x${PACK.height}`);
    const { ctx } = await staffContext(browser, { theme: theme.id, ...PACK });
    for (const surface of surfaces) {
      const id = `canonical-${surface.id}-${theme.id}`;
      if (only && !only.has(id)) continue;
      const page = await ctx.newPage();
      page.on("pageerror", (e) => consoleErrors.push(`${id}: ${String(e).slice(0, 160)}`));
      page.on("console", (m) => {
        if (m.type() === "error") consoleErrors.push(`${id}: ${m.text().slice(0, 160)}`);
      });
      await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
      if (surface.act) await surface.act(page).catch((e) => warn(`${id}: act failed (${e})`));
      await settle(page, surface.anchor);
      const metrics = await measure(page);
      judge(metrics, `${surface.id} @ ${theme.id}/${PACK.width}`);
      const file = resolve(OUT, `${id}.png`);
      await page.screenshot({ path: file });
      manifest.canonical.push({ id, path: surface.path, theme: theme.id, why: surface.why, file, metrics, consoleErrors: [...consoleErrors] });
      consoleErrors.length = 0;
      log(`  captured ${id}`);
      await page.close();
    }
    await ctx.close();
  }

  /* 2. Portal canonical shots. */
  for (const theme of THEMES) {
    log(`[canonical] portal, ${theme.portal} theme, ${PACK.width}x${PACK.height}`);
    for (const portal of PORTALS) {
      // The sign-in card itself needs a context with no session; the rest need
      // one, so the card is shot first and then the surface contexts open.
      const surfacesHere = PORTAL_SURFACES.filter(
        (s) => s.perClient || (s.clients ?? []).includes(portal.id),
      );
      if (surfacesHere.length === 0) continue;
      const ctx = await portalContext(browser, { email: portal.email, theme: theme.portal, ...PACK });
      for (const surface of surfacesHere) {
        const id = `canonical-${surface.id}-${portal.id}-${theme.id}`;
        if (only && !only.has(id)) continue;
        const page = await ctx.newPage();
        await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
        await settle(page, surface.anchor);
        const metrics = await measure(page);
        judge(metrics, `${surface.id}/${portal.id} @ ${theme.id}/${PACK.width}`);
        const file = resolve(OUT, `${id}.png`);
        await page.screenshot({ path: file });
        manifest.canonical.push({
          id,
          path: surface.path,
          portal: portal.email,
          client: portal.client,
          theme: theme.id,
          why: surface.why,
          file,
          metrics,
          consoleErrors: [...consoleErrors],
        });
        consoleErrors.length = 0;
        log(`  captured ${id}`);
        await page.close();
      }
      await ctx.close();
    }
  }
}

/* 3. The responsive matrix — every surface, five desktop sizes, both themes.
 *    Mechanical, no images: a size that breaks is named. */
if (wants("matrix")) {
  const tokens = await staffToken();
  for (const theme of THEMES) {
    const { ctx } = await staffContext(browser, { theme: theme.id, ...PACK });
    const matrixSurfaces = [...STAFF_SURFACES];
    if (attachmentTicketId !== null) {
      matrixSurfaces.push({ ...TICKET_ACTIVITY_SURFACE, path: `/tickets/${attachmentTicketId}` });
    }
    for (const surface of matrixSurfaces) {
      const page = await ctx.newPage();
      await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
      await settle(page, surface.anchor);
      for (const vp of VIEWPORTS) {
        await page.setViewportSize({ width: vp.width, height: vp.height });
        await page.waitForTimeout(600);
        const metrics = await measure(page);
        const row = judge(metrics, `${surface.id} @ ${theme.id}/${vp.id} (${vp.width}x${vp.height})`);
        manifest.matrix.push({ surface: surface.id, theme: theme.id, viewport: vp.id, width: vp.width, height: vp.height, ...metrics, bad: row.bad });
      }
      await page.close();
    }
    // Portal rides the same sweep for the two surfaces its clients read most.
    for (const portal of PORTALS) {
      const ctxP = await portalContext(browser, { email: portal.email, theme: theme.portal, ...PACK });
      for (const surface of PORTAL_SURFACES.filter((s) => s.perClient)) {
        const page = await ctxP.newPage();
        await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
        await settle(page, surface.anchor);
        for (const vp of VIEWPORTS) {
          await page.setViewportSize({ width: vp.width, height: vp.height });
          await page.waitForTimeout(600);
          const metrics = await measure(page);
          const row = judge(metrics, `${surface.id}/${portal.id} @ ${theme.id}/${vp.id} (${vp.width}x${vp.height})`);
          manifest.matrix.push({ surface: `${surface.id}-${portal.id}`, theme: theme.id, viewport: vp.id, width: vp.width, height: vp.height, ...metrics, bad: row.bad });
        }
        await page.close();
      }
      await ctxP.close();
    }
    await ctx.close();
  }
}

/* 4. Extreme shots — the narrowest and widest desktops, for a subset, so the
 *    ends of the range are looked at and not only measured. */
if (wants("extreme")) {
  const SUBSEAT = new Set(["tickets-list", "approvals-queue", "billing-ledger", "assets-warranty"]);
  for (const theme of THEMES) {
    for (const vp of EXTREMES) {
      log(`[extreme] staff, ${theme.id} theme, ${vp.id} ${vp.width}x${vp.height}`);
      const { ctx } = await staffContext(browser, { theme: theme.id, width: vp.width, height: vp.height });
      for (const surface of STAFF_SURFACES.filter((s) => SUBSEAT.has(s.id))) {
        const id = `extreme-${surface.id}-${vp.id}-${theme.id}`;
        if (only && !only.has(id)) continue;
        const page = await ctx.newPage();
        await page.goto(`${WEB}${surface.path}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
        await settle(page, surface.anchor);
        const metrics = await measure(page);
        judge(metrics, `${surface.id} @ ${theme.id}/${vp.id}`);
        const file = resolve(OUT, `${id}.png`);
        await page.screenshot({ path: file });
        manifest.extreme.push({ id, path: surface.path, theme: theme.id, viewport: vp.id, why: surface.why, file, metrics });
        log(`  captured ${id}`);
        await page.close();
      }
      await ctx.close();
    }
  }
  // The client portal at both ends, one client: the coldest surface in the
  // product is also the one a prospect's own staff will use.
  for (const theme of THEMES) {
    for (const vp of EXTREMES) {
      const id = `extreme-portal-tickets-acme-${vp.id}-${theme.id}`;
      if (only && !only.has(id)) continue;
      const ctx = await portalContext(browser, { email: PORTALS[0].email, theme: theme.portal, width: vp.width, height: vp.height });
      const page = await ctx.newPage();
      await page.goto(`${WEB}/portal/tickets`, { waitUntil: "domcontentloaded", timeout: 60_000 });
      await settle(page, '[data-testid="portal-tickets-open"]');
      const metrics = await measure(page);
      judge(metrics, `portal-tickets @ ${theme.id}/${vp.id}`);
      const file = resolve(OUT, `${id}.png`);
      await page.screenshot({ path: file });
      manifest.extreme.push({ id, path: "/portal/tickets", theme: theme.id, viewport: vp.id, why: "the client portal at both ends of the desktop range", file, metrics });
      log(`  captured ${id}`);
      await page.close();
      await ctx.close();
    }
  }
}

await browser.close();

/* ---------------------------------------------------------------------------
 * Write the record. The batches are pre-split at the row's ≤8 ceiling so the
 * vision review consumes them in order and never builds a request that would
 * be rejected for carrying too many images.
 * ---------------------------------------------------------------------- */
const images = [...manifest.canonical, ...manifest.extreme].map((c) => ({ id: c.id, file: c.file, why: c.why, theme: c.theme, path: c.path }));
for (let i = 0; i < images.length; i += 8) {
  manifest.batches.push({ batch: manifest.batches.length + 1, images: images.slice(i, i + 8) });
}
manifest.problems = problems;
manifest.generatedAt = new Date().toISOString();
manifest.runId = env.MARULA_RUN_ID ?? null;
writeFileSync(resolve(LANE, "manifest.json"), JSON.stringify(manifest, null, 2));

const breaks = manifest.matrix.filter((m) => m.bad.length > 0);
log(`\n[captures] ${manifest.canonical.length} canonical + ${manifest.extreme.length} extreme shots -> ${OUT}`);
log(`[matrix]   ${manifest.matrix.length} surface/size/theme measurements, ${breaks.length} with a problem`);
log(`[batches]  ${manifest.batches.length} vision-review batches (max 8 images each)`);
for (const p of problems) log(`  ! ${p}`);
if (problems.length > 0) process.exitCode = 1;
