// p24 vision-review captures. Auth via the app's httpOnly-cookie flow: a
// UI login *once* per context, then a retry loop — sign-in is the only
// credential path in this build (tokens live in memory + httpOnly cookie).
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

const root =
  "/Users/iordn/Documents/GitHub/meridian-open-psa-core/.kilo/worktrees/p24-theming-system";
const OUT = join(root, "design/qa-p24-theming-system/captures");
mkdirSync(OUT, { recursive: true });

const BASE = process.env.P24_WEB_URL ?? "http://127.0.0.1:15435";
const API = process.env.P24_API_URL ?? "http://127.0.0.1:15433";
// Demo credentials the embedded test app seeds (see rebuild-local-test-app).
const EMAIL = process.env.P24_EMAIL ?? "owner@demo.local";
const PASSWORD = process.env.P24_PASSWORD ?? "demo-password-123";

// The embedded test DB's durable per-IP auth gates persist across app
// restarts; a single short refresh lockout from context churn otherwise
// kills a whole capture pass. Operator-owned captures clear it per context.
const PG_PORT = process.env.P24_PG_PORT ?? "15434";
function clearAuthGates() {
  try {
    execFileSync("/usr/bin/env", [
      `PGPASSWORD=postgres`, "psql", "-h", "127.0.0.1", "-p", PG_PORT,
      "-U", "postgres", "-d", "meridian", "-c", "DELETE FROM auth_lockouts",
    ], { stdio: "ignore", env: { ...process.env, PGPASSWORD: "postgres" } });
  } catch { /* gate table may not exist on older bundles */ }
}

const browser = await chromium.launch({ args: ["--no-proxy-server"] });
const PRESETS = ["none", "tangerine", "slate", "high_contrast"];
// Optional: only (re)capture labels matching this substring — misses can be
// retried without redoing a full pass.
const ONLY = process.env.P24_ONLY ?? "";

const wanted = [];
const PRESET_LABELS = { tangerine: "Tangerine", slate: "Slate", high_contrast: "High contrast" };
for (const mode of ["light", "dark"]) {
  for (const preset of PRESETS) {
    const label = preset === "none" ? `${mode}-baseline` : `${mode}-${preset}`;
    if (ONLY && !label.includes(ONLY)) continue;
    wanted.push(label);
  }
}
const failures = [];
for (const label of wanted) {
  const preset = label === "light-baseline" || label === "dark-baseline"
    ? "none"
    : label.replace(/^(light|dark)-/, "");
  const mode = label.startsWith("light") ? "light" : "dark";
  try {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    clearAuthGates();
    const page = await ctx.newPage();
    await page.addInitScript(
      ([themeKey, presetKey, theme, presetName, apiBase]) => {
        window.__MARULA_API_URL__ = apiBase;
        localStorage.setItem(themeKey, theme);
        if (presetName && presetName !== "null")
          localStorage.setItem(presetKey, presetName);
      },
      ["meridian-theme", "meridian-theme-preset", mode, preset, API],
    );
    // Sign in (with retry — the only credential path this build owns).
    for (let attempt = 0; attempt < 4; attempt += 1) {
      await page.goto(`${BASE}/login`, { waitUntil: "networkidle" });
      await page.locator('input[type="email"], input[name*="email" i]').first().fill("owner@demo.local");
      await page.locator('input[type="password"]').first().fill("demo-password-123");
      await page.getByRole("button", { name: /sign in/i }).click();
      await page
        .waitForURL((u) => !String(u).includes("/login"), { timeout: 15000 })
        .catch(() => {});
      if (!page.url().includes("/login")) break;
      await page.waitForTimeout(1500);
    }
    if (page.url().includes("/login")) throw new Error(`${label}: login failed`);
    // The account's SERVER settings own the preset after hydration
    // (ThemePresetSync resolves user override → org theme), so each preset
    // leg clicks the real override radio on the appearance page and waits
    // for the save notice. Baseline legs re-select the org_default card.
    // The appearance page hosts the override radios; drive the real UI.
    await page.goto(`${BASE}/settings/appearance`, { waitUntil: "networkidle" });
    await page.waitForTimeout(1200);
    if (page.url().includes("/login")) throw new Error(`${label}: appearance redirected to login`);
    const overrideGroup = page.locator('[role="radiogroup"][aria-label="Theme override"]');
    await overrideGroup.waitFor({ state: "visible", timeout: 20000 });
    await page.locator('[role="radiogroup"][aria-label="Theme override"] [role="radio"]')
      .filter({ hasText: preset === "none" ? "Org default" : PRESET_LABELS[preset] })
      .click();
    await page
      .locator('text=/Saved to your account\\.|rejected/i')
      .first()
      .waitFor({ timeout: 15000 }).catch(() => {});
    // Reload: ThemePresetSync runs on authed mount and applies the fresh
    // server-resolved preset; the full navigation forces a session-start
    // sync instead of trusting the stale in-page state.
    await page.reload({ waitUntil: "networkidle" });
    await page.waitForTimeout(1500);
    for (let attempt = 0; attempt < 6; attempt += 1) {
      if (!page.url().includes("/login")) break;
      await page.goto(`${BASE}/settings/appearance`, { waitUntil: "networkidle" }).catch(() => {});
      await page.waitForTimeout(1200);
    }
    if (page.url().includes("/login")) throw new Error(`${label}: appearance redirected to login`);
    await page.screenshot({ path: join(OUT, `appearance-${label}.png`), fullPage: true });
    await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
    await page.waitForTimeout(1500);
    await page.screenshot({ path: join(OUT, `app-${label}.png`) });
    ctx.close();
  } catch (err) {
    failures.push(String(err.message ?? err));
    console.error("CAPTURE FAILED:", err.message ?? err);
  }
}
await browser.close();
if (failures.length > 0) {
  console.error("misses:", failures.join(" | "));
  process.exitCode = 1;
} else {
  console.log("captures done");
}
