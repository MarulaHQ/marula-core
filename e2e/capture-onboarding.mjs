// Screenshot capture for the Wave 26 vision review (glm-5.3-flash).
// Light + dark, all onboarding surfaces, plus the skip-path journey.
// Screenshots land in design/qa-p26-onboarding-flows/recap/.
// Usage: node e2e/capture-onboarding.mjs
// (uses the seeded demo org; API on :8080, web on :3000)
import { chromium } from "playwright";
import fs from "node:fs";
import path from "path";
import { fileURLToPath } from "url";
import { readFileSync } from "fs";

// Per-run ports come from the booted stack (e2e/.e2e-stack.env, written by
// scripts/e2e-stack.sh); the historical defaults keep this script usable
// against a manual dev boot on :8080/:3000.
function stackEnv(key, fallback) {
  try {
    const line = readFileSync(
      new URL("../e2e/.e2e-stack.env", import.meta.url),
      "utf8",
    )
      .split("\n")
      .find((l) => l.startsWith(`${key}=`));
    return line ? line.slice(key.length + 1).trim() : fallback;
  } catch {
    return fallback;
  }
}

const API = stackEnv("E2E_API_URL", "http://localhost:8080").replace(/\/$/, "");
const WEB = stackEnv("E2E_WEB_URL", "http://localhost:3000");
const OUT = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "design",
  "qa-p26-onboarding-flows",
  "capture",
);

async function login(email) {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password: "demo-password-123" }),
  });
  if (!res.ok) throw new Error(`login failed ${res.status}`);
  return res.json();
}

// YYYY: demo owner storage keys match AuthProvider ('marula.auth' etc.);
// mirror the harness helper exactly by seeding what it reads.
const TOKENS_KEY = "marula-tokens";
const TOKENS_EXP_KEY = "marula-tokens-exp";
const THEME_KEY = "marula-theme";
const THEME_PRESET_KEY = "marula-theme-preset";

async function seedAuth(ctx, page, owner, mode) {
  // Context-level login first (Set-Cookie session lands in the cookie
  // jar, matching the e2e harness's loginContext).
  const res = await ctx.request.post(`${API}/api/v1/auth/login`, {
    data: { email: "owner@demo.local", password: "demo-password-123" },
  });
  if (!res.ok()) throw new Error(`context login failed (${res.status()})`);
  const tokenPair = await res.json();
  await page.addInitScript(
    ([tokensKey, expKey, themeKey, presetKey, raw, exp, theme, preset]) => {
      window.localStorage.setItem(tokensKey, raw);
      window.localStorage.setItem(expKey, exp);
      if (theme !== null) window.localStorage.setItem(themeKey, theme);
      if (preset !== null) window.localStorage.setItem(presetKey, preset);
    },
    [
      TOKENS_KEY,
      TOKENS_EXP_KEY,
      THEME_KEY,
      THEME_PRESET_KEY,
      JSON.stringify(tokenPair),
      String(Date.now() + tokenPair.expires_in * 1000 - 30_000),
      mode === "dark" ? "dark" : "light",
      null,
    ],
  );
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  const owner = await login("owner@demo.local");
  const shots = [];

  // reset org + user state so the FULL wizard is reachable
  await fetch(`${API}/api/v1/onboarding/reset/org`, {
    method: "POST",
    headers: { authorization: `Bearer ${owner.access_token}`, "content-type": "application/json" },
    body: JSON.stringify({}),
  }).catch(() => {});
  await fetch(`${API}/api/v1/onboarding/reset/user`, {
    method: "POST",
    headers: { authorization: `Bearer ${owner.access_token}`, "content-type": "application/json" },
    body: JSON.stringify({}),
  }).catch(() => {});
  // The welcome hero shows only while defaults are unresolved — keep it so.
  await fetch(`${API}/api/v1/onboarding/user/steps/defaults/skip`, {
    method: "POST",
    headers: { authorization: `Bearer ${owner.access_token}`, "content-type": "application/json" },
    body: JSON.stringify({ never: false }),
  }).catch(() => {});
  await fetch(`${API}/api/v1/settings/org`, {
    method: "PUT",
    headers: { authorization: `Bearer ${owner.access_token}`, "content-type": "application/json" },
    body: JSON.stringify({ key: "onboarding.org_banner_dismissed", value: false }),
  }).catch(() => {});
  // MODE IS A USER-SOCPE SETTING (ThemePresetSync applies it at session
  // start) — set it once up front; light pass uses "light".
  await fetch(`${API}/api/v1/settings/user`, {
    method: "PUT",
    headers: { authorization: `Bearer ${owner.access_token}`, "content-type": "application/json" },
    body: JSON.stringify({ key: "appearance.mode", value: "light" }),
  }).catch(() => {});

  for (const mode of ["light", "dark"]) {
    // server-authoritative mode for this group (session start applies it)
    await fetch(`${API}/api/v1/settings/user`, {
      method: "PUT",
      headers: { authorization: `Bearer ${owner.access_token}`, "content-type": "application/json" },
      body: JSON.stringify({ key: "appearance.mode", value: mode }),
    }).catch(() => {});
    const ensureMode = async () => {
      const target = mode === "dark" ? /switch to dark/i : /switch to light/i;
      const toggle = page.getByRole("button", { name: target });
      await toggle.click().catch(() => {});
      await page.waitForTimeout(250);
    };
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    await seedAuth(ctx, page, owner, mode);

    // 1 · /today with the resume banner (the header toggle is the
    // app's authoritative mode pipeline — click through it per surface).
    await page.goto(`${WEB}/today`);
    await ensureMode();
    const banner = page.getByTestId("onboarding-banner");
    try {
      await banner.waitFor({ state: "visible", timeout: 20_000 });
      await page.screenshot({ path: `${OUT}/${mode}-01-today-banner.png`, fullPage: false });
    } catch {
      await page.screenshot({ path: `${OUT}/${mode}-01-today-original.png`, fullPage: false });
    }

    // 2 · wizard first step (clients)
    await page.goto(`${WEB}/welcome`);
    await ensureMode();
    await page.getByTestId("wizard-step-clients").waitFor({ timeout: 20_000 }).catch(() => {});
    await page.screenshot({ path: `${OUT}/${mode}-02-wizard-first-step.png`, fullPage: false });

    // 3 · skip contracts visible in the invite form region
    await page.getByTestId("onboarding-rail-invites").click().catch(() => {});
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${OUT}/${mode}-03-wizard-invites.png`, fullPage: false });

    // 4 · feeder + AI skip controls
    await page.getByTestId("onboarding-rail-feeder").click().catch(() => {});
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${OUT}/${mode}-04-wizard-feeder.png`, fullPage: false });

    // 5 · the AI setup step (W72): the shipped rail is indexed
    // (`wizard-step-6` is the sixth org step), so drive it directly.
    await page.getByTestId("wizard-step-6").click().catch(() => {});
    await page.getByTestId("ai-setup").waitFor({ timeout: 10_000 }).catch(() => {});
    await page.waitForTimeout(400); // let the staggered card entrance settle
    await page.screenshot({ path: `${OUT}/${mode}-05-wizard-ai-setup.png`, fullPage: true });

    // 6 · user first-run flow
    await page.goto(`${WEB}/onboarding`);
    await ensureMode();
    await page.waitForTimeout(800);
    await page.screenshot({ path: `${OUT}/${mode}-06-user-onboarding.png` });
    await ctx.close();
  }

  await browser.close();
  console.log(`captured → ${OUT}`);
})();
