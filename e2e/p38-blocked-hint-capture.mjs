// p38 popup-blocker hint capture: simulate a blocked window.open in the MAIN
// window, then screenshot the honest inline hint under the PopoutButton.
import { chromium } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { apiLogin, authedFetch } from "./helpers/api.ts";

const API = (process.env.E2E_API_URL ?? "http://localhost:28481").replace(/\/+$/, "");
const WEB = process.env.E2E_WEB_URL ?? "http://localhost:3001";
const OUT = new URL("../design/qa-p38-web-multi-window/captures/", import.meta.url).pathname;
mkdirSync(OUT, { recursive: true });

const tokens = await apiLogin("owner@demo.local", "demo-password-123");
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.request.post(`${API}/api/v1/auth/login`, { data: { email: "owner@demo.local", password: "demo-password-123" } });
const get = authedFetch(tokens.access_token);
const tickets = await get("/tickets?limit=50&board_id=6d657269-6469-616e-0000-00000000003c");
const ticket = tickets.filter((t) => t.category !== "closed")[0];

const page = await ctx.newPage();
await page.goto(`${WEB}/tickets/${ticket.id}`);
await page.getByTestId("popout-open").waitFor({ timeout: 30000 });
// Each context is allowed ONE window.open popup; consuming it makes every
// later window.open return null — exactly the popup-blocker posture.
await page.evaluate(() => {
  // Consume the one user-gesture popup Playwright allows, then force every
  // later window.open to return null — the popup-blocker posture.
  const warmed = window.open("about:blank");
  try { warmed?.close(); } catch {}
  window.open = () => null;
});
await page.getByTestId("popout-open").click();
await page.getByTestId("popout-blocked-hint").waitFor({ timeout: 10000 });
await page.waitForTimeout(400);
await page.screenshot({ path: `${OUT}p38-popout-blocked-hint.png` });
console.log("captured p38-popout-blocked-hint");
await browser.close();
console.log("done");
