// Wave 41 lane 2b vision-review captures.
//
// Auth via the app's httpOnly-cookie flow: a UI login once per context, then a
// retry loop, because sign-in is the only credential path in this build
// (tokens live in memory + an httpOnly cookie).
//
// What this shoots, and why each one is on the list:
//
//   density-comfortable / -compact / -dense   The D7 ladder at all three steps.
//                                             A ladder that is only ever seen at
//                                             one step is a ladder nobody has
//                                             checked moves anything.
//   composer-internal / -reply               The note tint against the plain
//                                             surface. This is THE comparison
//                                             the spec asks for, and it is only
//                                             visible with both on one screen.
//   ticket-stepper                          The status stepper, which did not
//                                             exist before this lane.
//   assistant-recommend                     The assistant surface, the one
//                                             inverted card on a screen.
//
// D12 controls have no live surface yet (they are primitives plus a
// persistence seam; no route mounts them). They are shot from a QA-only route
// that renders the real components with fixture data, and that route is
// deleted after the capture pass. See the landing record.
import { chromium } from "playwright";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const OUT = join(root, "design/qa-p41-marula-theme/captures");
mkdirSync(OUT, { recursive: true });

const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
// The stack env writes a TRAILING SLASH on the API base, so a naive
// `${API}/api/v1/...` becomes a double slash and the router 404s.
const API = (process.env.E2E_API_URL ?? "http://localhost:28480").replace(/\/+$/, "");
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const ONLY = process.env.CAPTURE_ONLY ?? "";

const browser = await chromium.launch({ args: ["--no-proxy-server"] });

async function loginOnce() {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (!res.ok) throw new Error(`login failed: HTTP ${res.status}`);
  return (await res.json()).access_token;
}

// ONE browser context for the whole pass. The auth gate counts sign-ins per
// IP and the demo DB persists lockouts, so a fresh login per capture group
// locks the operator out of their own stack partway through.
async function session() {
  if (!globalThis.__ctx) {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => console.error("  [pageerror]", e.message));
    await page.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
    await page.fill('input[type="email"]', EMAIL);
    await page.fill('input[type="password"]', PASSWORD);
    await Promise.all([
      page.waitForURL((u) => !u.pathname.startsWith("/login"), { timeout: 60_000 }),
      page.click('button[type="submit"]'),
    ]);
    globalThis.__ctx = ctx;
    globalThis.__token = await loginOnce();
  }
  const page = await globalThis.__ctx.newPage();
  page.on("pageerror", (e) => console.error("  [pageerror]", e.message));
  // Establish the origin before anything reads or writes localStorage: a
  // fresh page is about:blank, and localStorage on about:blank is a
  // SecurityError, not an empty store.
  await page.goto(WEB, { waitUntil: "domcontentloaded" });
  return { ctx: globalThis.__ctx, page };
}

/** Set the account density from node with the bearer token, then force a full
 *  reload. A page-context fetch would be cross-origin to the API and would
 *  silently carry no cookie, which reads as "the key does nothing". */
async function setDensity(_page, step) {
  const res = await fetch(`${API}/api/v1/settings/user`, {
    method: "PUT",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${globalThis.__token}`,
    },
    body: JSON.stringify({ key: "appearance.density", value: step }),
  });
  if (!res.ok) throw new Error(`density ${step}: HTTP ${res.status} ${await res.text()}`);
}


async function setMode(page, mode) {
  await page.evaluate(
    (mode) => localStorage.setItem("marula-theme", mode),
    mode,
  );
}

const wanted = (name) => ONLY === "" || name.includes(ONLY);
const shots = [];

// ---- 1. The density ladder, both modes --------------------------------------
if (wanted("density") || wanted("appearance")) {
  for (const mode of ["light", "dark"]) {
    const { ctx, page } = await session();
    await setMode(page, mode);
    for (const step of ["comfortable", "compact", "dense"]) {
      await setDensity(page, step);
      await page.goto(`${WEB}/settings/appearance`, { waitUntil: "networkidle" });
      await page.evaluate((m) => localStorage.setItem("marula-theme", m), mode);
      await page.reload({ waitUntil: "networkidle" });
      await page.waitForSelector('[data-testid="appearance-density"]', { timeout: 30_000 });
      await page.waitForTimeout(700);
      const file = join(OUT, `2b-density-${step}-${mode}.png`);
      await page.locator('[data-testid="appearance-density"]').screenshot({ path: file });
      const attr = await page.evaluate(() => document.documentElement.dataset.density);
      const control = await page.evaluate(() =>
        getComputedStyle(document.documentElement).getPropertyValue("--m-density-control").trim(),
      );
      console.log(`  density ${step} / ${mode}: data-density=${attr} --m-density-control=${control}`);
      shots.push(file);
    }
  }
}

// ---- 2. The ticket workbench: the stepper, the composer, the assistant ------
if (wanted("ticket") || wanted("composer") || wanted("stepper") || wanted("assistant")) {
  for (const mode of ["light", "dark"]) {
    const { ctx, page } = await session();
    await setMode(page, mode);
    await setDensity(page, "comfortable");

    await page.goto(`${WEB}/tickets`, { waitUntil: "networkidle" });
    await page.waitForTimeout(700);
    const first = page.locator('a[href^="/tickets/"]').first();
    await first.click();
    await page.waitForURL(/\/tickets\/[0-9a-f-]{36}/, { timeout: 60_000 });
    await page.waitForTimeout(2200);

    // The composer, in both of its modes, on one screen if possible.
    const composer = page.locator('[data-testid="notes-composer"]');
    if (await composer.count()) {
      await composer.scrollIntoViewIfNeeded();
      const internal = join(OUT, `2b-composer-internal-${mode}.png`);
      await composer.screenshot({ path: internal });
      shots.push(internal);
      console.log(`  composer internal / ${mode}`);

      // Open the intent menu and pick the client-visible reply, which is what
      // moves the box to the plain surface and offers the Waiting move.
      const current = page.locator('[data-testid="note-kind-current"]');
      if (await current.count()) {
        await current.click();
        await page.waitForTimeout(700);
        // The menu renders its items in place under the trigger, so match on
        // the option's own text rather than an accessible name that may carry
        // the "▾" affordance too.
        const reply = page.locator("button", { hasText: /^Client-visible reply/ }).first();
        if (await reply.count()) {
          await reply.click();
          await page.waitForTimeout(500);
          const replyShot = join(OUT, `2b-composer-reply-${mode}.png`);
          await composer.screenshot({ path: replyShot });
          shots.push(replyShot);
          console.log(`  composer reply / ${mode}`);
        }
      }
    }

    // The stepper.
    const stepper = page.locator(".m-stepper").first();
    if (await stepper.count()) {
      const file = join(OUT, `2b-stepper-${mode}.png`);
      await stepper.screenshot({ path: file });
      shots.push(file);
      const current = await stepper.locator('[aria-current="step"]').textContent();
      console.log(`  stepper / ${mode}: current step = ${current?.trim()}`);
    } else {
      console.log(`  stepper / ${mode}: NOT FOUND`);
    }

    // The assistant surface. The panel is toggled in the workbench, so look
    // for the card rather than assuming the panel is open.
    const card = page.locator('[data-testid="wb-agent-recommend"]').first();
    if (await card.count()) {
      const file = join(OUT, `2b-assistant-${mode}.png`);
      await card.screenshot({ path: file });
      shots.push(file);
      console.log(`  assistant / ${mode}`);
    }

    // A wider shot of the whole workbench for context.
    const full = join(OUT, `2b-ticket-full-${mode}.png`);
    await page.screenshot({ path: full });
    shots.push(full);
  }
}

// ---- 3. The D12 controls, from the QA-only route ---------------------------
if (wanted("d12") || wanted("toolbar")) {
  for (const mode of ["light", "dark"]) {
    const { ctx, page } = await session();
    await setMode(page, mode);
    await setDensity(page, "comfortable");
    await page.goto(`${WEB}/qa-marula-2b`, { waitUntil: "networkidle" });
    await page.waitForTimeout(1200);

    const strip = page.locator('[data-testid="qa-d12-toolbar"]').first();
    if (await strip.count()) {
      const file = join(OUT, `2b-d12-toolbar-${mode}.png`);
      await strip.screenshot({ path: file });
      shots.push(file);
      console.log(`  d12 toolbar / ${mode}`);

      // The chooser open, because the list ordering is the whole claim.
      const trigger = page.locator('[data-testid="qa-d12-columns-trigger"]').first();
      if (await trigger.count()) {
        await trigger.click();
        await page.waitForTimeout(450);
        const panel = page.locator(".m-column-chooser-panel").first();
        const open = join(OUT, `2b-d12-chooser-open-${mode}.png`);
        await panel.screenshot({ path: open });
        shots.push(open);
        console.log(`  d12 chooser / ${mode}`);
        await page.keyboard.press("Escape");
      }

      // The compound filter builder, because a chip list is only half of it.
      // It opens from FilterBar's OWN trigger, not from the QA route's
      // add-a-chip button.
      const add = page.locator("button", { hasText: /Add filter/ }).first();
      if (await add.count()) {
        await add.click();
        await page.waitForTimeout(600);
        const builder = page.locator(".m-filter-builder").first();
        if (await builder.count()) {
          const openBuilder = join(OUT, `2b-d12-filter-builder-${mode}.png`);
          await builder.screenshot({ path: openBuilder });
          shots.push(openBuilder);
          console.log(`  d12 filter builder / ${mode}`);
        }
      } else {
        console.log(`  d12 add-filter trigger / ${mode}: NOT FOUND`);
      }
    } else {
      console.log(`  d12 / ${mode}: QA route not reachable`);
    }
  }
}

await browser.close();
console.log(`\ncaptured ${shots.length} files into ${OUT}`);
for (const f of shots) console.log("  " + f.replace(root, ""));

// Never leave the capture-only route behind.
try {
  rmSync(join(root, "apps/web/app/qa-marula-2b"), { recursive: true, force: true });
  console.log("\nremoved the QA-only capture route");
} catch (e) {
  console.log("\nQA route removal failed:", e.message);
}
