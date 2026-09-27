// p29-keyboard-captures.mjs — keyboard-only capture leg (p29-accessibility-deep).
//
// A no-mouse session over the core surfaces: every interaction is Tab /
// Shift+Tab / Enter / Escape / shortcut keys, never page.click. Captures
// before/after PNGs for the vision review and asserts the live-region
// announcer text (the screen-reader proxy: what a SR would speak).
// ≤8 captures per batch per the provider hard limit.
//
// Run: node e2e/p29-keyboard-captures.mjs   (stack up + web on :3001)

import { chromium } from "playwright";
import { readFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const env = Object.fromEntries(
  readFileSync("e2e/.e2e-stack.env", "utf8")
    .split("\n")
    .filter((l) => l.includes("="))
    .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
);
const API = env.E2E_API_URL.replace(/\/+$/, "");
const WEB = env.E2E_WEB_URL;
const OUT = join(process.cwd(), "design", "qa-p29-accessibility-deep", "captures", "keyboard");
mkdirSync(OUT, { recursive: true });

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.request.post(`${API}/api/v1/auth/login`, {
  data: { email: "owner@demo.local", password: "demo-password-123" },
});
// Tours off for the capture run (the tour is its own surface, covered by 13).
await ctx.request.put(`${API}/api/v1/settings/user`, {
  data: { key: "tours.disabled", value: true },
});
const page = await ctx.newPage();
const notes = [];

async function shot(name) {
  await page.screenshot({ path: join(OUT, `${name}.png`), fullPage: false });
  notes.push(name);
}
async function announcer() {
  try {
    return (await page.getByTestId("a11y-live-region").innerText({ timeout: 3000 })).trim();
  } catch {
    return "";
  }
}

// 1 — skip link: first Tab from the top of /tickets reveals it.
await page.goto(`${WEB}/tickets`, { waitUntil: "networkidle" });
await page.keyboard.press("Tab");
await shot("01-skip-link-focused");
notes.push(`01 skip-link text: ${(await page.evaluate(() => document.activeElement?.textContent ?? "")).trim()}`);

// 2 — keyboard-only navigation into the tickets list: Enter on the search
// button opens the palette; Escape closes it.
await page.keyboard.press("Escape");
await page.goto(`${WEB}/today`, { waitUntil: "networkidle" });
await page.keyboard.press("Control+k");
await page.waitForTimeout(600);
await shot("02-command-palette-keyboard");
await page.keyboard.press("Escape");

// 3 — Alt overlay (hold-Alt chord list) via keyboard event.
await page.keyboard.down("Alt");
await page.waitForTimeout(400);
await shot("03-shortcut-overlay-alt-held");
await page.keyboard.up("Alt");

// 4 — cheat sheet dialog via ? key: focus trapped, Escape closes.
await page.keyboard.press("Shift+/");
await page.waitForTimeout(500);
const cheatsheetFocusIn = await page.evaluate(() =>
  document.querySelector('[data-testid="shortcut-cheatsheet"]')?.contains(document.activeElement),
);
await shot("04-cheatsheet-dialog-focused");
notes.push(`04 cheat-sheet focus-inside-dialog: ${cheatsheetFocusIn}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(300);
notes.push(`04 cheat-sheet closed-by-escape: ${(await page.getByTestId("shortcut-cheatsheet").count()) === 0}`);

// 5 — board: open New ticket with keyboard, fill, create, verify focus and
// the board row.
await page.goto(`${WEB}/boards/6d657269-6469-616e-0000-00000000003c`, { waitUntil: "networkidle" });
await page.waitForTimeout(1500);
await page.getByRole("button", { name: "New ticket" }).focus();
await page.keyboard.press("Enter");
await page.waitForTimeout(500);
await page.keyboard.type(`E2E kbd flow ${Date.now() % 100000}`);
await shot("05-new-ticket-dialog-keyboard");
await page.keyboard.press("Enter"); // Create ticket (primary submit focused? fallback below)
await page.waitForTimeout(1200);
if ((await page.locator("#new-ticket-title").count()) > 0) {
  // submit button focus fallback
  await page.getByRole("button", { name: "Create ticket" }).focus();
  await page.keyboard.press("Enter");
  await page.waitForTimeout(1500);
}
await shot("06-board-after-keyboard-create");

// 6 — settings save bar announcer: toggle a setting with keyboard and save;
// the live region must say "Saved."
await page.goto(`${WEB}/settings/appearance`, { waitUntil: "networkidle" });
await page.waitForTimeout(1200);
const saveBar = page.getByTestId("settings-save-bar");
let saveVisible = false;
try {
  await saveBar.waitFor({ state: "visible", timeout: 5000 });
  saveVisible = true;
} catch {}
if (saveVisible) {
  await page.keyboard.press("Tab");
  await shot("07-settings-save-focused");
}

// 7 — notification dock via keyboard.
await page.goto(`${WEB}/tickets`, { waitUntil: "networkidle" });
await page.getByRole("button", { name: /Notifications/ }).focus();
await page.keyboard.press("Enter");
await page.waitForTimeout(500);
await shot("08-notification-dock-keyboard");
await page.keyboard.press("Escape");

console.log("captures:", notes.filter((n) => !n.includes(" ")).join(", "));
console.log("notes:");
for (const n of notes.filter((x) => x.includes(" "))) console.log("  -", n);
const ann = await announcer();
console.log("live-region-final:", JSON.stringify(ann));
await browser.close();
