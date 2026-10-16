// Objective companion to the vision review. Scans every captured route for:
//   (A) every `.m-btn-primary` element's computed bg/fg + height
//   (D) any element painting the OLD danger literals (must be gone) vs the NEW
//       breach pair / destructive red
//   (sm) kit `Button size="sm"` heights at comfortable and dense
import { chromium } from "playwright";

const API = (process.env.E2E_API_URL ?? "http://localhost:28480").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

const req = await (await import("playwright")).request.newContext();
const lr = await req.post(`${API}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
const { access_token: token } = await lr.json();
const t = await (await req.fetch(`${API}/api/v1/tickets?limit=1`, { headers: { authorization: `Bearer ${token}` } })).json();
const TICKET = (Array.isArray(t) ? t : t.items)[0].id;

const ROUTES = [
  "/tickets", `/tickets/${TICKET}`, "/", "/clients", "/settings/appearance",
  "/settings/general", "/calendar", "/dispatch", "/portal", "/alerts",
];

// rgb() strings for the colours under test.
const C = {
  breachFillL: "rgb(251, 225, 222)", breachFgL: "rgb(143, 28, 19)",
  breachFillD: "rgb(74, 26, 24)", breachFgD: "rgb(249, 160, 151)",
  oldDangerFillL: "rgb(253, 232, 229)", oldDangerFgL: "rgb(180, 35, 24)",
  oldDangerFillD: "rgb(59, 26, 24)", oldDangerFgD: "rgb(251, 138, 128)",
  ink: "rgb(43, 42, 51)", gold: "rgb(232, 184, 58)",
};

const browser = await chromium.launch();
for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({ colorScheme: theme, reducedMotion: "reduce", viewport: { width: 1600, height: 994 } });
  await ctx.addInitScript((th) => { try { localStorage.setItem("marula-theme", th); } catch {} }, theme);
  const page = await ctx.newPage();
  for (let i = 0; i < 4; i++) {
    await page.goto(`${WEB}/login`).catch(() => {});
    await page.fill('input[type="email"]', EMAIL).catch(() => {});
    await page.fill('input[type="password"]', PASSWORD).catch(() => {});
    await page.click('button[type="submit"]').catch(() => {});
    try { await page.waitForURL(/tickets|today|\/$/, { timeout: 45000 }); break; } catch { await page.waitForTimeout(4000); }
  }
  for (const route of ROUTES) {
    await page.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded" }).catch(() => {});
    await page.waitForTimeout(900);
    const r = await page.evaluate((C) => {
      const all = [...document.querySelectorAll("*")];
      const prim = [];
      const oldHits = [];
      const newHits = [];
      for (const el of all) {
        const cs = getComputedStyle(el);
        const bg = cs.backgroundColor, fg = cs.color;
        const cls = typeof el.className === "string" ? el.className : "";
        if (cls.includes("m-btn-primary")) {
          prim.push({ text: (el.textContent || "").trim().slice(0, 24), bg, fg, h: Math.round(el.getBoundingClientRect().height) });
        }
        if (bg === C.oldDangerFillL || bg === C.oldDangerFillD || (fg === C.oldDangerFgL || fg === C.oldDangerFgD) && cls.includes("m-")) {
          oldHits.push({ cls: cls.slice(0, 60), bg, fg });
        }
        if (bg === C.breachFillL || bg === C.breachFillD) {
          newHits.push({ cls: cls.slice(0, 60), text: (el.textContent || "").trim().slice(0, 18), bg, fg });
        }
      }
      // kit Button sm: the sm classes contain min-h-[32px] + calc(var(--mc-ctl)-8px)
      const kitSm = [...document.querySelectorAll("button")].filter((b) =>
        (typeof b.className === "string") && b.className.includes("min-h-[32px]"));
      const kitSmH = [...new Set(kitSm.map((b) => Math.round(b.getBoundingClientRect().height)))].sort((a, b) => a - b);
      return {
        prim: prim.slice(0, 3), oldHits: oldHits.slice(0, 3), newHits: newHits.slice(0, 3),
        kitSmCount: kitSm.length, kitSmH,
      };
    }, C);
    const notable = r.prim.length || r.oldHits.length || r.newHits.length || r.kitSmCount;
    if (!notable) continue;
    console.log(`\n== ${theme} ${route} ==`);
    if (r.prim.length) console.log(`  m-btn-primary: ${JSON.stringify(r.prim)}`);
    if (r.kitSmCount) console.log(`  kit Button sm: n=${r.kitSmCount} heights=${JSON.stringify(r.kitSmH)}`);
    if (r.newHits.length) console.log(`  breach fill (new): ${JSON.stringify(r.newHits)}`);
    if (r.oldHits.length) console.log(`  OLD danger literal: ${JSON.stringify(r.oldHits)}`);
  }

  // Dense pass: measure kit Button sm heights on the two sm rows.
  for (const route of ["/tickets", `/tickets/${TICKET}`]) {
    await page.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded" }).catch(() => {});
    await page.waitForTimeout(900);
    await page.evaluate(() => document.documentElement.setAttribute("data-density", "dense"));
    await page.waitForTimeout(300);
    const h = await page.evaluate(() => {
      const kitSm = [...document.querySelectorAll("button")].filter((b) =>
        typeof b.className === "string" && b.className.includes("min-h-[32px]"));
      const prim = [...document.querySelectorAll(".m-btn-primary")].map((b) => Math.round(b.getBoundingClientRect().height));
      return {
        kitSm: [...new Set(kitSm.map((b) => Math.round(b.getBoundingClientRect().height)))].sort((a, b) => a - b),
        cssSm: [...new Set(prim)].sort((a, b) => a - b),
        density: document.documentElement.getAttribute("data-density"),
      };
    });
    console.log(`\n== ${theme} ${route} DENSE == density=${h.density} kit-sm-heights=${JSON.stringify(h.kitSm)} m-btn-primary-heights=${JSON.stringify(h.cssSm)}`);
  }
  await ctx.close();
}
await browser.close();
