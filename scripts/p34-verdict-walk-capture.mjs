// p34-spec-verdict-sweep — the walk-the-app capture leg.
//
// Walks the SHIPPED app against the seeded demo org and photographs every
// surface the verdict matrix leans on: dispatch board (ghost placement
// mid-drag), dispatch planning (phase allocations + forecast + the
// `curated` work-next queue), projects Gantt/burn, approvals, today,
// tickets, billing + verification, settings email, calendar sync,
// stock, dashboards, and the client portal. Both themes for the
// verdict-critical surfaces; light only for the rest.
//
// The five highest-value captures come from LIVE api data: the script
// signs in through the real form (never injected storage), reseeds a
// small amount of demo-org state through the real API (2 work-next
// entries, 1 dated phase + 1 phase allocation on the seeded project —
// scratch db only), then photographs. Nothing outside this run's
// scratch database is touched.
//
// Total captures ≈ 35. The vision review consuming them batches ≤8 per
// call and never more than 30 images in one provider request.
import { chromium } from "playwright";
import { mkdir, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { resolve } from "node:path";

const OUT = resolve(
  process.argv[2] ?? "design/qa-p34-spec-verdict-sweep/captures",
);
const WEB = process.env.E2E_WEB_URL ?? "http://localhost:3004";
const API = (process.env.E2E_API_URL ?? "http://localhost:28480/").replace(
  /\/+$/,
  "",
);
const STAFF_EMAIL = process.env.E2E_DEMO_EMAIL ?? "owner@demo.local";
const STAFF_PASSWORD = process.env.E2E_DEMO_PASSWORD ?? "demo-password-123";
const PORTAL_EMAIL = process.env.E2E_PORTAL_EMAIL ?? "portal@acme.demo";
const PORTAL_PASSWORD = process.env.E2E_DEMO_PASSWORD ?? "demo-password-123";
// E2E_WALK_ONLY=portal re-runs just the portal leg (e.g. after an auth
// fix); unset means the full walk.
const ONLY = process.env.E2E_WALK_ONLY ?? "";

async function apiLogin() {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: STAFF_EMAIL, password: STAFF_PASSWORD }),
  });
  if (!res.ok) throw new Error(`staff api login failed: ${res.status}`);
  return res.json();
}

const bearerOf = (access_token) => ({
  authorization: `Bearer ${access_token}`,
});
async function apiGet(token, path) {
  const res = await fetch(`${API}${path}`, { headers: bearerOf(token) });
  if (!res.ok) throw new Error(`GET ${path} failed: ${res.status}`);
  return res.json();
}
async function listify(token, path) {
  const body = await apiGet(token, path);
  return Array.isArray(body) ? body : (body.items ?? []);
}

// Reseed just enough org state for the surfaces to show rows: two
// work-next entries for the signed-in owner, one undated milestone phase
// plus one dated phase, and one 40%-windowed allocation onto the dated
// phase. Every call is fault-tolerant: a failure logs and the capture
// proceeds (an honest empty state is itself walk evidence).
async function bootstrapDemoState(token) {
  const me = await apiGet(token, "/api/v1/me");
  const userId = me.user_id ?? me.id;
  const tickets = await listify(token, "/api/v1/tickets?limit=5");
  const put = async (path, body) => {
    const res = await fetch(`${API}${path}`, {
      method: "PUT",
      headers: { ...bearerOf(token), "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`PUT ${path} failed ${res.status}`);
    return res.json();
  };
  let created = [];
  if (tickets.length >= 2) {
    for (const t of tickets.slice(0, 2)) {
      await put("/api/v1/dispatch/work-next", {
        user_id: userId,
        ticket_id: t.id,
        position: 0,
      }).catch((e) => log(`bootstrap work-next: ${e.message}`));
      created.push(t.id);
    }
  }
  const projects = await listify(token, "/api/v1/projects?limit=5");
  let projectId = projects[0]?.id ?? null;
  if (projectId) {
    const phases = await listify(token, `/api/v1/projects/${projectId}/phases`);
    let dated = phases.find((p) => p.start_date && p.end_date) ?? null;
    if (!dated) {
      const today = new Date().toISOString().slice(0, 10);
      const later = new Date(Date.now() + 14 * 86_400_000)
        .toISOString()
        .slice(0, 10);
      const res = await fetch(
        `${API}/api/v1/projects/${projectId}/phases`,
        {
          method: "POST",
          headers: { ...bearerOf(token), "content-type": "application/json" },
          body: JSON.stringify({
            name: "Discovery (dated)",
            sort_order: (phases.length ?? 0) + 1,
            start_date: today,
            end_date: later,
          }),
        },
      );
      if (res.ok) dated = await res.json();
      else log(`bootstrap phase POST: ${res.status} — using as-is`);
    }
    if (dated) {
      const start = dated.start_date;
      const end = dated.end_date;
      await put("/api/v1/dispatch/planning/allocations", {
        phase_id: dated.id,
        user_id: userId,
        share_percent: 40,
        start_date: start,
        end_date: end,
        note: "verdict-walk capture fixture",
      }).catch((e) => log(`bootstrap allocation: ${e.message}`));
    } else {
      log("no phase came out of bootstrap — allocation panel captures empty");
    }
  } else {
    log("no seeded project — gantt/allocations capture empty states");
  }
  return { userId, projectId };
}

const problems = [];

function log(msg) {
  console.log(`  ${msg}`);
  problems.push(msg);
}

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

const bootstrap = await apiLogin().then((t) =>
  bootstrapDemoState(t.access_token).then((b) => ({ ...b, token: t.access_token })),
);
log(`demo state ready (user ${bootstrap.userId}, project ${bootstrap.projectId})`);

const browser = await chromium.launch();
const manifest = [];

async function staffLoop() {
  for (const scheme of ["light", "dark"]) {
    const context = await browser.newContext({
      viewport: { width: 1440, height: 1000 },
      colorScheme: scheme,
      deviceScaleFactor: 2,
    });
    const page = await context.newPage();
    const consoleErrors = [];
    page.on("console", (m) => {
      if (m.type() === "error") consoleErrors.push(m.text().slice(0, 200));
    });
    page.on("pageerror", (e) =>
      consoleErrors.push(`pageerror: ${String(e).slice(0, 200)}`),
    );

    await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
    await page.fill('input[type="email"]', STAFF_EMAIL);
    await page.fill('input[type="password"]', STAFF_PASSWORD);
    await page.click('button[type="submit"]');
    try {
      await page.waitForFunction(
        () => !document.querySelector('[data-testid="login-card"]'),
        { timeout: 30_000 },
      );
    } catch {
      await page.screenshot({
        path: resolve(OUT, `LOGIN-FAILED-${scheme}.png`),
        fullPage: true,
      });
      throw new Error(`login did not complete (${scheme})`);
    }
    await page.waitForTimeout(800);

    const shot = async (name, path, opts = {}) => {
      // domcontentloaded + a settle wait: the workspace pages hold the SSE
      // stream open, and networkidle never fires on it.
      await page.goto(`${WEB}${path}`, {
        waitUntil: "domcontentloaded",
        timeout: 60_000,
      });
      await page.waitForTimeout(1500);
      if (opts.scrollToHeading) {
        await page.evaluate((heading) => {
          const el = [...document.querySelectorAll("h1, h2, h3")].find((n) =>
            (n.textContent ?? "").includes(heading),
          );
          el?.scrollIntoView({ block: "center" });
        }, opts.scrollToHeading);
        await page.waitForTimeout(500);
      }
      const file = resolve(OUT, `${name}.png`);
      await page.screenshot({ path: file, fullPage: opts.fullPage ?? false });
      manifest.push({ name, scheme, file, consoleErrors: [...consoleErrors] });
      consoleErrors.length = 0;
      console.log(`  captured ${name} (${scheme})`);
    };

    await shot(`today-${scheme}`, "/today");
    await shot(`tickets-list-${scheme}`, "/tickets");
    if (bootstrap.ticketId)
      await shot(`ticket-detail-${scheme}`, `/tickets/${bootstrap.ticketId}`);

    // Dispatch board + the mid-drag ghost. The drag is cancelled with
    // Escape so no seeded appointment moves.
    await shot(`dispatch-board-${scheme}`, "/dispatch", { fullPage: true });
    try {
      const card = page.locator("[data-q]").first();
      const lane = page.locator("[data-tech]").first();
      const cardBox = await card.boundingBox();
      const laneBox = await lane.boundingBox();
      if (cardBox && laneBox) {
        await page.mouse.move(cardBox.x + cardBox.width / 2, cardBox.y + cardBox.height / 2);
        await page.mouse.down();
        await page.mouse.move(laneBox.x + laneBox.width / 2, laneBox.y + laneBox.height / 2, { steps: 24 });
        await page.waitForTimeout(240);
        const file = resolve(OUT, `dispatch-ghost-${scheme}.png`);
        await page.screenshot({ path: file });
        manifest.push({ name: `dispatch-ghost-${scheme}`, scheme, file, consoleErrors: [...consoleErrors] });
        consoleErrors.length = 0;
        await page.keyboard.press("Escape");
        await page.mouse.up();
        console.log(`  captured dispatch-ghost-${scheme} (cancelled)`);
      } else {
        log(`no drag targets found on the board (${scheme})`);
      }
    } catch (e) {
      log(`ghost capture failed (${scheme}): ${String(e).slice(0, 160)}`);
      await page.keyboard.press("Escape").catch(() => {});
      await page.mouse.up().catch(() => {});
    }

    await shot(`planning-top-${scheme}`, "/dispatch/planning");
    await shot(`planning-full-${scheme}`, "/dispatch/planning", { fullPage: true });
    await shot(`approvals-${scheme}`, "/approvals");

    await shot(`billing-${scheme}`, "/billing");
    if (bootstrap.invoiceId)
      await shot(`billing-invoice-${scheme}`, `/billing/${bootstrap.invoiceId}`);
    await shot(`billing-verification-${scheme}`, "/billing/verification");

    await shot(`settings-email-${scheme}`, "/settings/email");
    await shot(`calendar-sync-${scheme}`, "/calendar", { fullPage: true });
    await shot(`stock-${scheme}`, "/stock");
    await shot(`dashboards-${scheme}`, "/dashboards");

    await context.close();
  }
}

async function portalLoop() {
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    colorScheme: "light",
    deviceScaleFactor: 2,
  });
  const page = await context.newPage();
  await page.addInitScript(() => {
    window.localStorage.setItem("marula-theme", "light");
  });
  const consoleErrors = [];
  page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${String(e).slice(0, 200)}`));

  // The portal card opens on the magic-link form; the password form sits
  // behind the `portal-use-password` toggle. Wait for hydration before
  // clicking, and for the password field before filling — both forms
  // share the `#portal-email` id.
  await page.goto(`${WEB}/portal/login`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.waitForSelector('[data-testid="portal-use-password"]', { timeout: 30_000 });
  const file = resolve(OUT, "portal-login.png");
  await page.screenshot({ path: file, fullPage: true });
  manifest.push({ name: "portal-login", scheme: "light", file, consoleErrors: [...consoleErrors] });
  console.log("  captured portal-login (light)");
  await context.close();

  // Signed-in portal pages: the portal session is an HttpOnly cookie
  // (the app actively scrubs `marula-portal-tokens` out of localStorage,
  // so injecting that key is a silent no-op), so the login runs through
  // this context's own request client — the session cookie lands in the
  // context jar and every portal fetch carries it, exactly like a real
  // signed-in browser.
  const pctx = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    colorScheme: "light",
    deviceScaleFactor: 2,
  });
  const pLogin = await pctx.request.post(`${API}/api/v1/portal/login`, {
    data: { email: PORTAL_EMAIL, password: PORTAL_PASSWORD },
  });
  if (!pLogin.ok()) throw new Error(`portal api login failed: ${pLogin.status()}`);

  const ppage = await pctx.newPage();
  const pErrors = [];
  ppage.on("pageerror", (e) => pErrors.push(`pageerror: ${String(e).slice(0, 200)}`));
  await ppage.addInitScript(() => {
    window.localStorage.setItem("marula-theme", "light");
  });

  const pshot = async (name, path) => {
    await ppage.goto(`${WEB}${path}`, {
      waitUntil: "domcontentloaded",
      timeout: 60_000,
    });
    await ppage.waitForTimeout(1500);
    // A portal page that renders the sign-in gate means the session was
    // not accepted — record it loudly instead of shipping a silent miss.
    if (await ppage.locator('[data-testid="portal-login"]').count()) {
      problems.push(`portal ${name} rendered the sign-in gate (session not accepted)`);
    }
    const f = resolve(OUT, `${name}.png`);
    await ppage.screenshot({ path: f, fullPage: name === "portal-kb" });
    manifest.push({ name, scheme: "light", file: f, consoleErrors: [...pErrors] });
    pErrors.length = 0;
    console.log(`  captured ${name} (light)`);
  };

  await pshot("portal-tickets", "/portal/tickets");
  await pshot("portal-invoices", "/portal/invoices");
  if (bootstrap.portalInvoiceId)
    await pshot("portal-invoice", `/portal/invoices/${bootstrap.portalInvoiceId}`);
  await pshot("portal-kb", "/portal/kb");

  await pctx.close();
}

// Resolve the remaining ids now that bootstrap wrote dispatch state.
try {
  const tickets = await listify(bootstrap.token, "/api/v1/tickets?limit=5");
  bootstrap.ticketId = tickets[0]?.id ?? null;
} catch (e) {
  log(`ticket id resolve: ${e.message}`);
}
try {
  const invoices = await listify(bootstrap.token, "/api/v1/invoices?limit=5");
  bootstrap.invoiceId = invoices[0]?.id ?? null;
} catch (e) {
  log(`invoice id resolve: ${e.message}`);
}
try {
  const portalRes = await fetch(`${API}/api/v1/portal/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: PORTAL_EMAIL, password: PORTAL_PASSWORD }),
  });
  if (portalRes.ok) {
    const pbody = await portalRes.json();
    const pinv = await fetch(`${API}/api/v1/portal/invoices`, {
      headers: bearerOf(pbody.access_token),
    });
    if (pinv.ok) {
      const rows = await pinv.json();
      bootstrap.portalInvoiceId = (Array.isArray(rows) ? rows : rows.invoices ?? [])[0]?.id ?? null;
    }
  } else {
    log(`portal api login ${portalRes.status}`);
  }
} catch (e) {
  log(`portal api login: ${e.message}`);
}

if (ONLY !== "portal") await staffLoop();
try {
  await portalLoop();
} catch (e) {
  log(`portal loop failed: ${String(e).slice(0, 200)}`);
}

await browser.close();
await writeFile(resolve(OUT, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(`\n[vision] ${manifest.length} captures -> ${OUT}`);
for (const p of problems) console.log(`  ⚠ ${p}`);
