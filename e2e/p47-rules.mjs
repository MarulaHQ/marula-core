// p47-rules.mjs — Wave 47 lane A13, `p47-vision-captures`.
//
// The §2 rules of `DESIGN-NOTES-tickets.md` that are behaviour, not layout, and
// the interaction states the pack draws. `p47-conformance.mjs` proves what is on
// screen at rest; this drives the surface and reads what happens:
//
//   §2.3  one filter state shared by views, chips and the builder — and a link
//         that reproduces it.        → FILTER-LINK
//   §2.4  columns are per user, persist, reorder and resize. → COLUMN-PERSIST
//   §2.5  export says what it will export.                    → EXPORT-STATEMENT
//   §2.6  selection replaces the column header, in place.     → SELECTION-HEADER
//   §4    density applies to controls and rows together.      → DENSITY-LADDER
//   §9    a control changes on hover and shows a focus ring.  → INTERACTION
//
// Usage: node e2e/p47-rules.mjs        (stack up, web bundle serving)
import { chromium } from "playwright";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const LANE = "design/qa-p47-vision-captures";
mkdirSync(LANE, { recursive: true });
const CANVAS = { width: 1600, height: 994 };
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

const envPath = resolve("e2e/.e2e-stack.env");
if (!existsSync(envPath)) throw new Error("e2e/.e2e-stack.env missing — boot the stack first");
const env = Object.fromEntries(
  readFileSync(envPath, "utf8")
    .split("\n")
    .filter((l) => l.includes("="))
    .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
);
const api = (env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const web = env.E2E_WEB_URL ?? "http://localhost:3000";

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: CANVAS });
await ctx.request.post(`${api}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
const tokens = await (await fetch(`${api}/api/v1/auth/login`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
})).json();
await ctx.addInitScript(
  ([raw, exp]) => {
    window.localStorage.setItem("marula-tokens", raw);
    window.localStorage.setItem("marula-tokens-exp", exp);
    window.localStorage.setItem("marula-theme", "light");
  },
  [JSON.stringify(tokens), String(Date.now() + tokens.expires_in * 1000 - 30_000)],
);

const put = (key, value) =>
  ctx.request.put(`${api}/api/v1/settings/user`, {
    data: { key, value },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
await put("appearance.data_surfaces", "{}");
await put("appearance.density", "comfortable");

const open = async () => {
  const page = await ctx.newPage();
  await page.goto(`${web}/tickets`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.waitForLoadState("networkidle", { timeout: 15_000 }).catch(() => {});
  await page.waitForTimeout(1200);
  return page;
};

const out = { generatedAt: new Date().toISOString(), checks: {} };

// --- §2.3 the filter builder, and whether the state has a link -----------------
{
  const page = await open();
  await page.locator("button", { hasText: "Filter" }).first().click().catch(() => {});
  await page.waitForTimeout(400);
  const dialog = page.locator('[role=dialog]').first();
  const text = (await dialog.innerText().catch(() => "")) || "";
  const url = page.url();
  out.checks["§2.3 filter builder"] = {
    builderOpened: await dialog.isVisible().catch(() => false),
    builderText: text.replace(/\s+/g, " ").slice(0, 220),
    offersCopyLink: /copy link/i.test(text),
    offersSaveAsView: /save as view/i.test(text),
    urlCarriesFilterState: /[?&](filter|f=|view=|q=)/.test(url),
  };
  await page.close();
}

// --- §2.4 columns: per user, persist, reorder, resize --------------------------
{
  const page = await open();
  const before = await page.locator("thead th").allInnerTexts().catch(() => []);
  await page.locator("button", { hasText: "Columns" }).first().click().catch(() => {});
  await page.waitForTimeout(400);
  const panelText = (await page.locator('[role=dialog], [role=menu], [role=listbox]').first().innerText().catch(() => "")) || "";
  // Toggle the first checkbox in the chooser (turn a hidden column on).
  const boxes = page.locator('[role=dialog] input[type=checkbox], [role=menu] input[type=checkbox], [role=dialog] [role=checkbox]');
  const n = await boxes.count().catch(() => 0);
  if (n > 0) await boxes.nth(n - 1).click().catch(() => {});
  await page.waitForTimeout(600);
  const afterToggle = await page.locator("thead th").allInnerTexts().catch(() => []);
  await page.close();
  // Reload: did the toggle persist for this user?
  const page2 = await open();
  const afterReload = await page2.locator("thead th").allInnerTexts().catch(() => []);
  await page2.close();
  out.checks["§2.4 columns"] = {
    before,
    afterToggle,
    afterReload,
    persistedAcrossReload: JSON.stringify(afterToggle) === JSON.stringify(afterReload),
    chooserText: panelText.replace(/\s+/g, " ").slice(0, 160),
    statesSavedPerDevice: /saved/i.test(panelText),
  };
}

// --- §2.5 export says what it will export -------------------------------------
{
  const page = await open();
  await page.locator("button", { hasText: "Export" }).first().click().catch(() => {});
  await page.waitForTimeout(400);
  const pop = page.locator('[role=dialog], [role=menu], [role=listbox]').first();
  const text = (await pop.innerText().catch(() => "")) || "";
  out.checks["§2.5 export"] = {
    popoverOpened: await pop.isVisible().catch(() => false),
    text: text.replace(/\s+/g, " ").slice(0, 260),
    statesRows: /\b\d+\b\s*(row|rows)/.test(text),
    statesColumns: /column/i.test(text),
    statesFiltersOrSort: /filter|sort|order/i.test(text),
  };
  await page.close();
}

// --- §2.6 selection replaces the column header, in place ----------------------
{
  const page = await open();
  const headerBefore = await page.locator("thead th").allInnerTexts().catch(() => []);
  const box = page.locator("tbody tr").first().locator('input[type=checkbox], [role=checkbox]').first();
  await box.click().catch(async () => {
    await page.locator("tbody tr").first().locator("button").first().click().catch(() => {});
  });
  await page.waitForTimeout(500);
  const headerAfter = await page.locator("thead th").allInnerTexts().catch(() => []);
  const bulkText = (await page.locator(".m-bulk-action-bar, [data-testid*=bulk]").first().innerText().catch(() => "")) || "";
  out.checks["§2.6 selection"] = {
    headerBefore,
    headerAfter,
    headerReplaced: JSON.stringify(headerBefore) !== JSON.stringify(headerAfter),
    bulkBarText: bulkText.replace(/\s+/g, " ").slice(0, 160),
  };
  await page.close();
}

// --- §4 density applies to controls and rows together -------------------------
{
  const measure = async (step) => {
    await put("appearance.density", step);
    const page = await open();
    const m = await page.evaluate(() => {
      const row = document.querySelector("tbody tr");
      const btn = document.querySelector("thead button, .m-toolbar-button, .m-data-toolbar button");
      const h = (el) => (el ? Math.round(el.getBoundingClientRect().height) : null);
      return {
        density: document.documentElement.getAttribute("data-density"),
        row: h(row),
        control: h(btn),
        th: h(document.querySelector("thead th")),
      };
    });
    await page.close();
    return m;
  };
  out.checks["§4 density ladder"] = {
    comfortable: await measure("comfortable"),
    compact: await measure("compact"),
    dense: await measure("dense"),
  };
  await put("appearance.density", "comfortable");
}

// --- §9 a control changes on hover and shows a focus ring ---------------------
{
  const page = await open();
  const btn = page.locator(".m-data-toolbar button, thead button").first();
  const base = await btn.evaluate((el) => {
    const s = getComputedStyle(el);
    return { background: s.backgroundColor, outline: s.outlineStyle, boxShadow: s.boxShadow };
  }).catch(() => null);
  await btn.hover().catch(() => {});
  await page.waitForTimeout(250);
  const hovered = await btn.evaluate((el) => {
    const s = getComputedStyle(el);
    return { background: s.backgroundColor, boxShadow: s.boxShadow };
  }).catch(() => null);
  await page.keyboard.press("Tab");
  await page.waitForTimeout(150);
  const focused = await page.evaluate(() => {
    const el = document.activeElement;
    if (!el) return null;
    const s = getComputedStyle(el);
    return { tag: el.tagName, outline: s.outlineStyle, outlineWidth: s.outlineWidth, boxShadow: s.boxShadow };
  });
  out.checks["§9 interaction"] = {
    base,
    hovered,
    hoverChanges: JSON.stringify(base) !== JSON.stringify(hovered),
    focused,
  };
  await page.close();
}

writeFileSync(resolve(LANE, "RULES.json"), JSON.stringify(out, null, 2));
console.log(JSON.stringify(out, null, 2));
console.log(`\n[rules] -> ${LANE}/RULES.json`);
await browser.close();
