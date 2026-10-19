/**
 * Capture the technician day surface at phone and tablet widths, light and
 * dark. Same posture as the foundation script: Expo web renders the SAME
 * React tree with the same tokens, so these captures carry the layout,
 * palette, and copy review; native gestures still need a device.
 *
 * Usage (Expo web must be on the stack's CORS web port):
 *   bash scripts/e2e-stack.sh up
 *   . e2e/.e2e-stack.env
 *   EXPO_PUBLIC_API_URL=http://localhost:$E2E_API_PORT \
 *   pnpm --filter @marula-core/native exec expo start --web --port $E2E_WEB_PORT
 *   node native/scripts/capture-web-tech-day.mjs
 *
 * Output: design/qa-p33-rn-tech-day/captures/<theme>-<device>-<name>.png
 */

import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const outDir = path.resolve(here, "../../design/qa-p33-rn-tech-day/captures");
const WEB_URL = process.env.CAPTURE_WEB_URL ?? "http://localhost:3002";
const API_URL = (process.env.CAPTURE_API_URL ?? "http://localhost:28480").replace(/\/$/, "");
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

const DEVICES = [
  { name: "phone", width: 390, height: 844 },
  { name: "tablet", width: 1024, height: 1366 },
];
const THEMES = ["light", "dark"];

async function login() {
  const response = await fetch(`${API_URL}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (!response.ok) throw new Error(`login failed: ${response.status}`);
  return response.json();
}

async function main() {
  await mkdir(outDir, { recursive: true });
  const tokens = await login();
  const session = {
    accessToken: tokens.access_token,
    refreshToken: tokens.refresh_token,
    sessionId: tokens.session_id,
  };
  const browser = await chromium.launch();
  const shots = [];

  for (const device of DEVICES) {
    for (const theme of THEMES) {
      const context = await browser.newContext({
        viewport: { width: device.width, height: device.height },
        deviceScaleFactor: 1,
        colorScheme: theme,
      });
      await context.addInitScript(
        ([sessionJson, themeMode]) => {
          localStorage.setItem("marula.session.v1", sessionJson);
          localStorage.setItem("@marula-core/prefs/theme.mode", themeMode);
        },
        [JSON.stringify(session), theme],
      );
      const page = await context.newPage();
      const shot = async (name) => {
        const file = path.join(outDir, `${theme}-${device.name}-${name}.png`);
        await page.screenshot({ path: file, animations: "disabled", timeout: 60_000 });
        shots.push(path.basename(file));
        console.log(`captured ${path.basename(file)}`);
      };
      const go = async (route) => {
        await page.goto(`${WEB_URL}${route}`, { waitUntil: "domcontentloaded" });
        await page.waitForTimeout(1200);
      };

      // Today: the day rail, on-call strip, travel section.
      await go("/");
      await page.waitForSelector('[data-testid="today-screen"]', { timeout: 45_000 });
      await page.waitForTimeout(1800);
      await shot("today");

      // The travel form, opened from the section header.
      if ((await page.locator('[data-testid="travel-log"]').count()) > 0) {
        await page.click('[data-testid="travel-log"]');
        await page.waitForSelector('[data-testid="travel-form"]', { timeout: 15_000 });
        await page.waitForTimeout(600);
        await shot("travel-form");
        await page.click('[data-testid="travel-cancel"]');
        await page.waitForTimeout(400);
      }

      // Ticket detail: clock strip, time entries, notes, composer.
      await go("/board");
      await page.waitForTimeout(1500);
      const row = page.locator('[data-testid^="ticket-row-"]').first();
      if ((await row.count()) > 0) {
        await row.click();
        await page.waitForSelector('[data-testid="note-input"]', { timeout: 20_000 });
        await page.waitForTimeout(1500);
        await shot("ticket-detail");

        // Stage the sample photo through the injector, then capture the chip.
        if ((await page.locator('[data-testid="note-attach-sample"]').count()) > 0) {
          await page.click('[data-testid="note-attach-sample"]');
          await page.waitForTimeout(2500);
          await shot("ticket-photo-staged");
        }
      }

      // Notifications: the footer kind pill and the ticket affordance.
      await go("/notifications");
      await page.waitForSelector('[data-testid="notifications-screen"]', { timeout: 20_000 });
      await page.waitForTimeout(1000);
      await shot("notifications");

      // Accept a booking last, so the pristine schedule is already captured.
      await go("/");
      await page.waitForTimeout(1200);
      const accept = page.locator('[data-testid^="appointment-action-"]').first();
      if ((await accept.count()) > 0) {
        await accept.click();
        await page.waitForTimeout(1200);
        await shot("today-accepted");
      }

      await context.close();
    }
  }

  await browser.close();
  console.log(`\n${shots.length} captures in ${path.relative(process.cwd(), outDir)}`);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
