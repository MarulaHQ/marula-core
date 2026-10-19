// p51-title-scale measurement harness.
//
// Measures the rendered page-title (first visible h1) font-size on the four
// surfaces this lane owns, against the running e2e stack, and prints one JSON
// line per probe. The surfaces:
//
//   /today          (staff, owner)  — pack draws the greeting at 28
//   /documents      (staff, owner)  — pack draws the Knowledge home at 28
//   /welcome        (staff, owner)  — pack draws the wizard card heading at 24
//   /onboarding     (staff, owner)  — same
//   /login          (signed out)    — pack draws the marketing headline at 34
//   /portal/csat    (client seat)   — pack draws the portal title at 26, in
//                                     the form, the thank-you and the
//                                     expired-link error alike.
//
// The csat probe stubs the single-use token endpoint so the form and the
// thank-you states render without seeding a survey; the error state is the
// real no-token path (which is the state the audit reached).
//
// Run: node e2e/p51-title-scale.mjs   (stack up + standalone web bundle up)
"use strict";

import { readFileSync } from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const STAFF_EMAIL = "owner@demo.local";
const STAFF_PASSWORD = "demo-password-123";
const PORTAL_EMAIL = "portal@acme.demo";

function stackEnv() {
  const out = {};
  try {
    const raw = readFileSync(path.join(HERE, ".e2e-stack.env"), "utf8");
    for (const line of raw.split("\n")) {
      const m = /^([A-Z0-9_]+)=(.*)$/.exec(line.trim());
      if (m) out[m[1]] = m[2];
    }
  } catch {
    /* no stack env — the caller must have exported E2E_API_URL/E2E_WEB_URL */
  }
  return out;
}

const ENV = stackEnv();
const API = process.env.E2E_API_URL ?? ENV.E2E_API_URL ?? "http://localhost:8080";
const WEB = process.env.E2E_WEB_URL ?? ENV.E2E_WEB_URL ?? "http://localhost:3000";

async function staffTokens() {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: STAFF_EMAIL, password: STAFF_PASSWORD }),
  });
  if (!res.ok) throw new Error(`staff login failed (${res.status})`);
  return res.json();
}

async function staffContext(browser) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const tokens = await staffTokens();
  const login = await ctx.request.post(`${API}/api/v1/auth/login`, {
    data: { email: STAFF_EMAIL, password: STAFF_PASSWORD },
  });
  if (!login.ok()) throw new Error(`context login failed (${login.status()})`);
  await ctx.request.put(`${API}/api/v1/settings/user`, {
    data: { key: "tours.disabled", value: true },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  await ctx.addInitScript(
    ([key, expKey, raw, exp]) => {
      window.localStorage.setItem(key, raw);
      window.localStorage.setItem(expKey, exp);
      window.localStorage.setItem("marula-theme", "light");
      window.localStorage.setItem("marula-theme-preset", "marula");
    },
    [
      "marula-tokens",
      "marula-tokens-exp",
      JSON.stringify(tokens),
      String(Date.now() + tokens.expires_in * 1000 - 30_000),
    ],
  );
  return ctx;
}

async function portalContext(browser) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  await page.goto(`${WEB}/portal/login`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.waitForSelector('[data-testid="portal-use-password"]', { timeout: 60_000 });
  await page.click('[data-testid="portal-use-password"]');
  await page.fill("#portal-email", PORTAL_EMAIL);
  await page.fill("#portal-password", STAFF_PASSWORD);
  await page.click('[data-testid="portal-password-submit"]');
  await page.waitForSelector('[data-testid="portal-tickets-open"]', { timeout: 60_000 });
  await page.close();
  return ctx;
}

const TITLE_PROBE = () => {
  const hs = [...document.querySelectorAll("h1")].filter(
    (h) => h.getBoundingClientRect().height > 0 && h.textContent.trim() !== "",
  );
  const h = hs[0];
  if (!h) return null;
  const st = getComputedStyle(h);
  return {
    text: h.textContent.replace(/\s+/g, " ").trim().slice(0, 60),
    size: parseFloat(st.fontSize),
    weight: st.fontWeight,
    cls: typeof h.className === "string" ? h.className : "",
  };
};

async function probe(page, route, { settleTestId } = {}) {
  await page.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  if (settleTestId) {
    await page.waitForSelector(`[data-testid="${settleTestId}"]`, { timeout: 30_000 }).catch(() => {});
  }
  await page.waitForTimeout(700);
  const title = await page.evaluate(TITLE_PROBE);
  return { route, url: page.url(), title };
}

async function main() {
  const browser = await chromium.launch();

  // A signed-out context for /login (the marketing headline).
  const anon = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const anonPage = await anon.newPage();
  console.log(
    JSON.stringify(await probe(anonPage, "/login", { settleTestId: "login-submit" })),
  );
  await anon.close();

  const staff = await staffContext(browser);
  const staffPage = await staff.newPage();
  for (const route of ["/today", "/documents", "/welcome", "/onboarding"]) {
    console.log(JSON.stringify(await probe(staffPage, route)));
  }
  await staff.close();

  // The portal feedback surface. The form and thank-you states are reached by
  // stubbing the single-use token endpoint; the error state is the real path.
  const portal = await portalContext(browser);
  const view = {
    ticket_id: "00000000-0000-0000-0000-0000000000aa",
    ticket_number: "18398",
    ticket_subject: "Printer on the second floor is offline",
    status: "valid",
    expires_at: new Date(Date.now() + 3600_000).toISOString(),
  };
  const csatPage = await portal.newPage();
  await csatPage.route("**/api/v1/portal/csat/mock-token", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(view) }),
  );
  await csatPage.route("**/api/v1/portal/csat/mock-token-done", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ...view, status: "responded" }),
    }),
  );
  console.log(JSON.stringify(await probe(csatPage, "/portal/csat")));
  console.log(JSON.stringify(await probe(csatPage, "/portal/csat?ticket=mock-token")));
  console.log(JSON.stringify(await probe(csatPage, "/portal/csat?ticket=mock-token-done")));
  await portal.close();

  await browser.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
