// Probe: main window -> popout; dump console + failed requests + body text.
import { chromium } from "@playwright/test";
import { apiLogin, authedFetch } from "./helpers/api.ts";

const API = (process.env.E2E_API_URL ?? "http://localhost:28481").replace(/\/+$/, "");
const WEB = process.env.E2E_WEB_URL ?? "http://localhost:3001";
console.log("probe api", API, "web", WEB);

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
const tokens = await apiLogin("owner@demo.local", "demo-password-123");
// The page bootstraps from the httpOnly session cookie — put it in the
// context jar (same posture as the harness fixture).
await ctx.request.post(`${API}/api/v1/auth/login`, { data: { email: "owner@demo.local", password: "demo-password-123" } });
const get = authedFetch(tokens.access_token);
const tickets = await get("/tickets?limit=50&board_id=6d657269-6469-616e-0000-00000000003c");
const ticket = tickets.filter((t) => t.category !== "closed")[0];
console.log("ticket", ticket?.id);

const page = await ctx.newPage();
page.on("console", (m) => { console.log("MAIN-C", m.type(), m.text().slice(0, 220)); });
page.on("response", (r) => { if (r.status() >= 400) console.log("MAIN-HTTP", r.status(), r.url().slice(0, 160)); });
await page.goto(`${WEB}/tickets/${ticket.id}`);
await page.getByTestId("popout-open").waitFor({ timeout: 20000 });

const [popout] = await Promise.all([
  ctx.waitForEvent("page"),
  page.getByTestId("popout-open").click(),
]);
popout.on("console", (m) => console.log("POP-C", m.type(), m.text().slice(0, 220)));
popout.on("requestfailed", (r) => console.log("POP-FAIL", r.url().slice(0, 120), r.failure()?.errorText));
popout.on("response", (r) => { if (r.status() >= 400) console.log("POP-HTTP", r.status(), r.url().slice(0, 140)); });
page.on("request", (r) => { if (r.url().includes("/comments") || r.url().includes("/events/stream")) console.log("MAIN-REQ", r.url().slice(0, 130)); });
await popout.waitForLoadState("domcontentloaded");
console.log("popout url", popout.url());
console.log("main window-id:", await page.evaluate(() => sessionStorage.getItem("marula-window-id")));
console.log("popout window-id:", await popout.evaluate(() => sessionStorage.getItem("marula-window-id")));
// Probe the cross-window legs from inside the MAIN window.
await page.evaluate(() => {
  const w = window;
  w.__probeMsgs = [];
  new BroadcastChannel("marula-windows").onmessage = (e) => w.__probeMsgs.push(JSON.stringify(e.data).slice(0, 200));
});
// Post a note in the popout (Notes tab).
await popout.getByTestId("detail-tab-notes").click();
await popout.getByTestId("notes-composer").waitFor({ timeout: 20000 });
const body = `PROBE note ${Date.now() % 100000}`;
await popout.getByTestId("notes-composer-body").fill(body);
await popout.getByTestId("note-post").click();
await popout.waitForTimeout(3000);
console.log("popout shows note:", await popout.getByText(body).first().isVisible().catch(() => false));
for (let i = 0; i < 5; i++) {
  await page.waitForTimeout(4000);
  const seen = await page.evaluate(() => window.__probeMsgs);
  const inMain = await page.getByText(body).first().isVisible().catch(() => false);
  console.log(`t+${(i + 1) * 4}s main shows note:`, inMain, "| bc msgs:", JSON.stringify(seen));
}
console.log("MAIN console errors collected above; main url:", page.url());
console.log("popout title-ish:", await popout.locator("main").innerText().then(t => t.slice(0, 400)).catch(String));
await browser.close();
