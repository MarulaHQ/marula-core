// Wave 47 lane B2 `p47-byok-runbook` — vision-review captures for the BYOK
// bring-up runbook's page: the real /settings/ai surface in both themes.
//
// The runbook (docs/site/how-to/admin-ai-bringup.md) is written against what
// this page actually renders, so the page is captured the way a demo
// operator sees it: signed in as the demo owner against the seeded org,
// master switch in its shipped default position.
//
// Captures (4 per theme, 8 total — one vision batch):
//   1. settings-ai-full-{theme}      — the whole page, top to bottom
//   2. settings-ai-master-{theme}    — the org master switch card
//   3. settings-ai-matrix-{theme}    — the deny-by-default surfaces matrix
//   4. settings-ai-slots-{theme}     — catalog + caps + credential slot
//
// Run against the live stack (the BYOK probe must pass first so the
// captures show a configured, working org):
//   set -a; . e2e/.e2e-stack.env; set +a
//   node e2e/p47-byok-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p47-byok-runbook/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

mkdirSync(OUT, { recursive: true });

const ctx0 = await (await import("playwright")).request.newContext();
const login = await ctx0.post(`${API}/api/v1/auth/login`, {
  data: { email: EMAIL, password: PASSWORD },
});
if (!login.ok()) throw new Error(`login failed: ${login.status()}`);
const tokens = await login.json();

const failures = [];
for (const theme of ["light", "dark"]) {
  const browser = await chromium.launch({ args: ["--no-proxy-server"] });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  try {
    const page = await ctx.newPage();
    const pageErrors = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    await page.addInitScript(
      ([apiBase, tokenPair, themeName]) => {
        window.__MARULA_API_URL__ = apiBase;
        window.localStorage.setItem("marula-tokens", tokenPair);
        window.localStorage.setItem("marula-tokens-exp", String(Date.now() + 3_600_000));
        window.localStorage.setItem("marula-theme", themeName);
      },
      [API, JSON.stringify(tokens), theme],
    );
    await page.goto(`${WEB}/settings/ai`, { waitUntil: "domcontentloaded" });
    await page.getByTestId("settings-ai-page").waitFor({ timeout: 30_000 });
    await page.waitForTimeout(600);

    // 1 — the whole page.
    await page.screenshot({ path: join(OUT, `settings-ai-full-${theme}.png`), fullPage: true });

    // 2 — the master switch card.
    const master = page.getByTestId("ai-master-card");
    await master.waitFor({ timeout: 10_000 });
    await master.screenshot({ path: join(OUT, `settings-ai-master-${theme}.png`) });

    // 3 — the surfaces matrix (scrolled into view; the card clips if off-screen).
    const matrix = page.getByTestId("surface-matrix-rows");
    await matrix.waitFor({ timeout: 10_000 });
    await matrix.scrollIntoViewIfNeeded();
    await page.waitForTimeout(250);
    await page.screenshot({
      path: join(OUT, `settings-ai-matrix-${theme}.png`),
      clip: await matrix.boundingBox().then((box) =>
        box === null ? undefined : { x: Math.max(box.x - 24, 0), y: Math.max(box.y - 96, 0), width: box.width + 48, height: box.height + 120 },
      ),
    });

    // 4 — the slot region: catalog, caps, credential slot.
    const slot = page.getByTestId("credential-slot");
    await slot.waitFor({ timeout: 10_000 });
    await slot.scrollIntoViewIfNeeded();
    await page.waitForTimeout(250);
    const slotBox = await slot.boundingBox();
    const catalog = page.getByTestId("catalog-add-row");
    if (slotBox !== null && (await catalog.count()) > 0) {
      const catalogBox = await catalog.boundingBox();
      if (catalogBox !== null) {
        await page.screenshot({
          path: join(OUT, `settings-ai-slots-${theme}.png`),
          clip: {
            x: Math.min(catalogBox.x, slotBox.x) - 24,
            y: Math.max(catalogBox.y - 48, 0),
            width: Math.max(catalogBox.width, slotBox.width) + 48,
            height: Math.max(slotBox.y + slotBox.height - catalogBox.y + 48, 320),
          },
        });
      }
    }

    if (pageErrors.length > 0) {
      failures.push(`${theme}: page errors — ${pageErrors.join(" | ")}`);
    }
    await page.close();
  } catch (err) {
    failures.push(`${theme}: ${err.message ?? err}`);
  }
  await ctx.close();
  await browser.close();
}

await ctx0.dispose();
if (failures.length > 0) {
  console.error("capture misses:", failures.join(" | "));
  process.exitCode = 1;
} else {
  console.log(`captures written to ${OUT}`);
}
