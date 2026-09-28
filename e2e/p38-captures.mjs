// p38 vision-review captures: popout surfaces, compact shells, wide-viewport set.
import { chromium } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { apiLogin, authedFetch } from "./helpers/api.ts";

const API = (process.env.E2E_API_URL ?? "http://localhost:28481").replace(/\/+$/, "");
const WEB = process.env.E2E_WEB_URL ?? "http://localhost:3001";
const OUT = new URL("../design/qa-p38-web-multi-window/captures/", import.meta.url).pathname;
mkdirSync(OUT, { recursive: true });

const tokens = await apiLogin("owner@demo.local", "demo-password-123");
const get = authedFetch(tokens.access_token);
const tickets = await get("/tickets?limit=50&board_id=6d657269-6469-616e-0000-00000000003c");
const ticket = tickets.filter((t) => t.category !== "closed")[0];

async function session(theme) {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await ctx.request.post(`${API}/api/v1/auth/login`, { data: { email: "owner@demo.local", password: "demo-password-123" } });
  const page = await ctx.newPage();
  await page.addInitScript((t) => {
    try { window.localStorage.setItem("meridian-theme", t); } catch {}
  }, theme);
  return { browser, ctx, page };
}

async function shot(page, name) {
  await page.screenshot({ path: `${OUT}${name}.png` });
  console.log("captured", name);
}

// Light: main + popout ticket detail (wide-viewport set at 2560x1080 too).
{
  const { browser, ctx, page } = await session("light");
  await page.goto(`${WEB}/tickets/${ticket.id}`);
  await page.getByTestId("popout-open").waitFor({ timeout: 30000 });
  await page.waitForTimeout(2500);
  await shot(page, "p38-main-ticket-light");
  const [popout] = await Promise.all([
    ctx.waitForEvent("page"),
    page.getByTestId("popout-open").click(),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await popout.getByTestId("popout-bar").waitFor({ timeout: 30000 });
  await popout.waitForTimeout(3000);
  await shot(popout, "p38-popout-ticket-light");
  await browser.close();
}
// Dark pair.
{
  const { browser, ctx, page } = await session("dark");
  await page.goto(`${WEB}/tickets/${ticket.id}`);
  await page.getByTestId("popout-open").waitFor({ timeout: 30000 });
  await page.waitForTimeout(2500);
  await shot(page, "p38-main-ticket-dark");
  const [popout] = await Promise.all([
    ctx.waitForEvent("page"),
    page.getByTestId("popout-open").click(),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await popout.getByTestId("popout-bar").waitFor({ timeout: 30000 });
  await popout.waitForTimeout(3000);
  await shot(popout, "p38-popout-ticket-dark");
  await browser.close();
}
// Dispatch board popout (light) + stock lookup popout (light).
{
  const { browser, ctx, page } = await session("light");
  await page.goto(`${WEB}/dispatch`);
  await page.waitForTimeout(3000);
  await shot(page, "p38-main-dispatch-light");
  // Popout via the palette-riding keyboard chord (alt+shift+p), per scope.
  const [popout] = await Promise.all([
    ctx.waitForEvent("page"),
    page.keyboard.press("Alt+Shift+KeyP"),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await popout.getByTestId("popout-bar").waitFor({ timeout: 30000 });
  await popout.waitForTimeout(2500);
  await shot(popout, "p38-popout-dispatch-light");
  await browser.close();
}
{
  const { browser, ctx, page } = await session("light");
  await page.goto(`${WEB}/stock`);
  await page.waitForTimeout(2500);
  const [popout] = await Promise.all([
    ctx.waitForEvent("page"),
    page.keyboard.press("Alt+Shift+KeyP"),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await popout.getByTestId("popout-bar").waitFor({ timeout: 30000 });
  await popout.waitForTimeout(2500);
  await shot(popout, "p38-popout-stock-light");
  await browser.close();
}
// Approvals popout (dark).
{
  const { browser, ctx, page } = await session("dark");
  await page.goto(`${WEB}/approvals`);
  await page.waitForTimeout(2500);
  const [popout] = await Promise.all([
    ctx.waitForEvent("page"),
    page.keyboard.press("Alt+Shift+KeyP"),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await popout.getByTestId("popout-bar").waitFor({ timeout: 30000 });
  await popout.waitForTimeout(2500);
  await shot(popout, "p38-popout-approvals-dark");
  await browser.close();
}
// Wide-viewport pair (2560x1080): main + popout ticket detail, light.
{
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 2560, height: 1080 } });
  await ctx.request.post(`${API}/api/v1/auth/login`, { data: { email: "owner@demo.local", password: "demo-password-123" } });
  const page = await ctx.newPage();
  await page.goto(`${WEB}/tickets/${ticket.id}`);
  await page.getByTestId("popout-open").waitFor({ timeout: 30000 });
  await page.waitForTimeout(2500);
  await shot(page, "p38-main-ticket-wide-2560");
  const [popout] = await Promise.all([
    ctx.waitForEvent("page"),
    page.getByTestId("popout-open").click(),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await popout.getByTestId("popout-bar").waitFor({ timeout: 30000 });
  await popout.waitForTimeout(3000);
  await shot(popout, "p38-popout-ticket-wide-2560");
  await browser.close();
}
console.log("done");
