// Wave 34 lane 6 `p34-open-gaps-collector` — vision-review captures for the
// desktop update strip.
//
// The strip only appears in the Tauri shell (the desktop app loads the web
// app from a loopback origin and drives `check_desktop_update` over the
// app ACL). This script drives the REAL app in Chromium with `window
// __TAURI__` stubbed at the exact IPC boundary the shell exposes — same
// command names, same payload shape — so the rendered states are the real
// component's, not a mockup. The real-shell leg is captured separately
// during the Tauri gate run; please read REVIEW.md for which capture is
// which.
//
// States, light + dark (8 captures, one vision batch):
//   1. update-available — the strip with its two actions
//   2. update-installing — after "Restart & update" (download in flight)
//   3. update-failed    — install rejected, retry offered
//   4. no-update        — check answers "current": no strip at all
//
// Run against the live stack:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p34-updater-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p34-open-gaps-collector/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

mkdirSync(OUT, { recursive: true });

const ctx0 = await (await import("playwright")).request.newContext();
const login = await ctx0.post(`${API}/api/v1/auth/login`, {
  data: { email: EMAIL, password: PASSWORD },
});
if (!login.ok()) throw new Error(`login failed: ${login.status()}`);
const tokens = await login.json();

/** Install the shell stub at the IPC boundary, before app scripts run. */
async function stubShell(page, mode) {
  await page.addInitScript((mode) => {
    const UPDATE = {
      version: "0.2.0",
      currentVersion: "0.1.0",
      date: "2026-09-25T00:00:00Z",
      notes: "Day pack, faster ticket list, label fixes.",
    };
    window.__TAURI__ = {
      core: {
        invoke: (cmd) => {
          if (cmd === "check_desktop_update") {
            return Promise.resolve(mode === "no-update" ? null : UPDATE);
          }
          if (cmd === "install_desktop_update") {
            if (mode === "failed") {
              return Promise.reject(new Error("signature check failed"));
            }
            // Never resolves: the shell restarts the app after install.
            return new Promise(() => {});
          }
          return Promise.reject(new Error(`unexpected command ${cmd}`));
        },
      },
    };
  }, mode);
}

const failures = [];
for (const theme of ["light", "dark"]) {
  const browser = await chromium.launch({ args: ["--no-proxy-server"] });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  {
    let ok = false;
    for (let attempt = 0; attempt < 20 && !ok; attempt += 1) {
      const res = await ctx.request.post(`${API}/api/v1/auth/login`, {
        data: { email: EMAIL, password: PASSWORD },
      });
      ok = res.ok();
      if (!ok) await new Promise((resolve) => setTimeout(resolve, 3000));
    }
    if (!ok) throw new Error("context login failed");
  }
  try {
    // 1+2+3 — the three visible states.
    for (const mode of ["available", "installing", "failed"]) {
      const page = await ctx.newPage();
      await stubShell(page, mode);
      await page.addInitScript(
        ([apiBase, tokenPair, themeName]) => {
          window.__MARULA_API_URL__ = apiBase;
          window.localStorage.setItem("marula-tokens", tokenPair);
          window.localStorage.setItem("marula-tokens-exp", String(Date.now() + 3_600_000));
          window.localStorage.setItem("marula-theme", themeName);
        },
        [API, JSON.stringify(tokens), theme],
      );
      await page.goto(`${WEB}/`, { waitUntil: "domcontentloaded" });
      const strip = page.getByTestId("desktop-update-strip");
      await strip.waitFor({ timeout: 30_000 });
      if (mode === "installing" || mode === "failed") {
        await page.getByTestId("desktop-update-install").click();
      }
      if (mode === "installing") {
        await page.getByTestId("desktop-update-installing").waitFor({ timeout: 10_000 });
      }
      if (mode === "failed") {
        await page.getByTestId("desktop-update-failed").waitFor({ timeout: 10_000 });
      }
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(OUT, `update-${mode}-${theme}.png`) });
      await page.close();
    }

    // 4 — "current": the honest silent baseline (no strip).
    {
      const page = await ctx.newPage();
      await stubShell(page, "no-update");
      await page.addInitScript(
        ([apiBase, tokenPair, themeName]) => {
          window.__MARULA_API_URL__ = apiBase;
          window.localStorage.setItem("marula-tokens", tokenPair);
          window.localStorage.setItem("marula-tokens-exp", String(Date.now() + 3_600_000));
          window.localStorage.setItem("marula-theme", themeName);
        },
        [API, JSON.stringify(tokens), theme],
      );
      await page.goto(`${WEB}/`, { waitUntil: "domcontentloaded" });
      await page.waitForTimeout(2500);
      const strip = await page.$('[data-testid="desktop-update-strip"]');
      if (strip !== null) throw new Error("strip rendered on an up-to-date shell");
      await page.screenshot({ path: join(OUT, `update-none-${theme}.png`) });
      await page.close();
    }
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
