// (sm) leg evidence. The kit Button has no consumer in apps/web, so its sm
// cannot appear in an app capture. Measure it directly from the compiled CSS
// (inject an element carrying the exact kit sm classes) and, beside it, the
// LIVE small button `.m-btn-sm`, across all three densities and both themes.
import { chromium } from "playwright";
const WEB = "http://localhost:3000", API = "http://localhost:28480";
const EMAIL = "owner@demo.local", PASSWORD = "demo-password-123";
const req = await (await import("playwright")).request.newContext();
const lr = await req.post(`${API}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
const { access_token: token } = await lr.json();
const t = await (await req.fetch(`${API}/api/v1/tickets?limit=1`, { headers: { authorization: `Bearer ${token}` } })).json();
const TICKET = (Array.isArray(t) ? t : t.items)[0].id;

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
  await page.goto(`${WEB}/tickets`, { waitUntil: "domcontentloaded" }).catch(() => {});
  await page.waitForTimeout(1200);
  for (const d of ["comfortable", "compact", "dense"]) {
    const r = await page.evaluate((d) => {
      document.documentElement.setAttribute("data-density", d);
      // inject the exact kit sm class string
      const el = document.createElement("button");
      el.id = "kit-sm-probe";
      el.className = "inline-flex shrink-0 items-center h-[calc(var(--mc-ctl)-8px)] min-h-[32px] rounded-ctl px-[14px] text-[13px]";
      el.textContent = "kit sm";
      document.body.appendChild(el);
      const live = [...document.querySelectorAll(".m-btn-sm")].map((b) => Math.round(b.getBoundingClientRect().height));
      const ctl = getComputedStyle(document.documentElement).getPropertyValue("--mc-ctl").trim();
      const h = Math.round(el.getBoundingClientRect().height);
      el.remove();
      return { density: document.documentElement.getAttribute("data-density"), ctl, kitSm: h, liveSm: [...new Set(live)].sort((a, b) => a - b), liveSmN: live.length };
    }, d);
    console.log(`${theme} ${r.density}: --mc-ctl=${r.ctl} kitButton.sm=${r.kitSm}px  live .m-btn-sm heights=${JSON.stringify(r.liveSm)} (n=${r.liveSmN})`);
  }
  await ctx.close();
}
await browser.close();
