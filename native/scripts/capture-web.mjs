/**
 * Capture the app's web render at phone and tablet widths, light and dark.
 *
 * The foundation host has no Xcode, so simulator captures are blocked (see
 * native/README.md operator gaps). Expo web renders the SAME React tree with
 * the same tokens, so these captures carry the layout, palette, and copy
 * review — every native-behavior claim still needs a device.
 *
 * Usage (Expo web must be on the stack's CORS web port):
 *   bash scripts/e2e-stack.sh up
 *   . e2e/.e2e-stack.env
 *   EXPO_PUBLIC_API_URL=http://localhost:$E2E_API_PORT \
 *   EXPO_PUBLIC_WS_URL=$E2E_WS_URL \
 *   pnpm --filter @marula-core/native exec expo start --web --port $E2E_WEB_PORT
 *   pnpm --filter @marula-core/native exec playwright test ... (or node this file)
 *
 * This file is plain Node (playwright is a root devDependency).
 */

import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const outDir = path.resolve(here, "../../design/qa-p33-rn-foundation/captures");
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
          localStorage.setItem("meridian.session.v1", sessionJson);
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

      // Today
      await go("/");
      await page.waitForSelector('[data-testid="today-screen"]', { timeout: 45000 });
      await page.waitForTimeout(1500);
      await shot("today");

      // Board + ticket detail
      await page.goto(`${WEB_URL}/board`, { waitUntil: "domcontentloaded" });
      await page.waitForTimeout(1500);
      await shot("board");
      const row = page.locator('[data-testid^="ticket-row-"]').first();
      if ((await row.count()) > 0) {
        await row.click();
        await page.waitForSelector('[data-testid="note-input"]', { timeout: 20000 });
        await page.waitForTimeout(1200);
        await shot("ticket");
      }

      // Scan: deterministic unknown payload
      await go("/scan");
      await page.waitForSelector('[data-testid="scan-screen"]', { timeout: 20000 });
      await page.fill('[data-testid="scan-input"]', "NOPE-NOPE-000");
      await page.click('[data-testid="scan-resolve"]');
      await page.waitForSelector('[data-testid="scan-resolution"]', { timeout: 20000 });
      await page.waitForTimeout(800);
      await shot("scan");

      // Notifications and tools
      await go("/notifications");
      await page.waitForSelector('[data-testid="notifications-screen"]', { timeout: 20000 });
      await page.waitForTimeout(1000);
      await shot("notifications");

      await go("/dev/queue");
      await page.waitForSelector('[data-testid="queue-screen"]', { timeout: 20000 });
      await page.waitForTimeout(600);
      await shot("queue-empty");

      // More
      await go("/more");
      await page.waitForSelector('[data-testid="more-screen"]', { timeout: 20000 });
      await page.waitForTimeout(800);
      await shot("more");

      // Offline: toggle the dev airplane switch, then navigate IN-APP so the
      // forced-offline state survives (a full reload resets it), queue a
      // note, and capture the banner + inspector with a live pending op.
      await page.click('[data-testid="more-airplane-toggle"]');
      await page.waitForTimeout(800);
      await shot("more-offline-forced");
      await page.click('[role="tab"][href="/board"]:visible');
      await page.waitForTimeout(1500);
      const offlineRow = page.locator('[data-testid^="ticket-row-"]').first();
      if ((await offlineRow.count()) > 0) {
        await offlineRow.click();
        await page.waitForSelector('[data-testid="note-input"]', { timeout: 20000 });
        await page.fill('[data-testid="note-input"]', "Captured offline note");
        await page.click('[data-testid="note-submit"]');
        await page.waitForTimeout(1500);
        await shot("ticket-offline-queued");
      }
      await page.click('[role="tab"][href="/more"]:visible');
      await page.waitForTimeout(1000);
      await shot("more-offline-queued");
      await page.getByText("Pending changes").click();
      await page.waitForSelector('[data-testid="queue-screen"]', { timeout: 20000 });
      await page.waitForTimeout(800);
      await shot("queue-pending");
      // Back online; the queue drains and the banner clears. The inspector
      // is a root route (no tab bar), so history returns to More with the
      // forced-offline state intact.
      await page.goBack();
      await page.waitForTimeout(800);
      await page.click('[data-testid="more-airplane-toggle"]');
      await page.waitForTimeout(2500);
      await shot("more-back-online");

      await context.close();
    }
  }

  await browser.close();
  await writeFile(path.join(outDir, "INDEX.json"), JSON.stringify(shots, null, 2));
  console.log(`\n${shots.length} captures in ${outDir}`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
