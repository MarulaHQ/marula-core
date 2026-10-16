// p47a15-chord-probe.mjs — the macOS keyboard-chord measurement (Wave 47 lane A15).
//
// Why this exists: `apps/web/lib/commands/chords.ts` read `event.key`, and on
// macOS a held Option changes what `key` says — Option+2 arrives as "™",
// Option+N as a dead key. This probe measures what Chromium on THIS machine
// actually delivers for each chord the registry authors, then evaluates both
// the old and the new parser against those delivered events, and finally
// presses the real chord against the running app and records where the router
// ended up.
//
// Two input paths on purpose:
//   - page.keyboard.press(...)  — what the Playwright flow suite sends.
//   - raw CDP Input.dispatchKeyEvent with the composed `key` — what a real
//     physical Option+2 produces; used to show the OS-accurate event.
//
// Run: node e2e/p47a15-chord-probe.mjs    (stack up: scripts/e2e-stack.sh up)
// Writes design/qa-<branch>/chord-measurement.json — no hard-coded lane path.

import { chromium } from "playwright";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
// The canonical evidence-dir rule (same one e2e/helpers/evidence.ts uses), so
// this probe lands in the running lane's own directory and nowhere else.
const require = createRequire(import.meta.url);
const { resolveEvidenceDir } = require("./helpers/evidence-core.cjs");

const env = Object.fromEntries(
  readFileSync("e2e/.e2e-stack.env", "utf8")
    .split("\n")
    .filter((l) => l.includes("="))
    .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
);
const API = env.E2E_API_URL.replace(/\/+$/, "");
const WEB = env.E2E_WEB_URL;
const REPO = process.cwd();
const git = (args) => execFileSync("git", ["-C", REPO, ...args], { encoding: "utf8" }).trim();
const BRANCH = git(["rev-parse", "--abbrev-ref", "HEAD"]);
const OUT = resolveEvidenceDir(
  {
    override: process.env.E2E_EVIDENCE_DIR,
    branch: BRANCH,
    sha: git(["rev-parse", "--short", "HEAD"]),
    runId: env.MARULA_RUN_ID ?? "",
  },
  resolve(REPO),
);
mkdirSync(OUT, { recursive: true });

// The two parser bodies, side by side, so the probe shows the difference on
// the SAME delivered event rather than asserting it in prose.
const PARSERS = `
window.__p47a15 = {
  // OLD: read event.key (the composed character under Option).
  old(e) { return e.key.length !== 1 ? null : e.key.toLowerCase(); },
  // NEW: the physical key for letters/digits, event.key for punctuation.
  new(e) {
    const code = e.code || "";
    if (code.length === 4 && code.startsWith("Key")) return code.slice(3).toLowerCase();
    if (code.length === 6 && code.startsWith("Digit")) return code.slice(5);
    return e.key.length !== 1 ? null : e.key.toLowerCase();
  },
  chord(e, keyFn) {
    const parts = [];
    if (e.altKey) parts.push("alt");
    if (e.metaKey) parts.push("mod");
    if (e.shiftKey) parts.push("shift");
    const k = keyFn(e);
    if (k === null) return null;
    parts.push(k);
    return parts.join("+");
  },
};
window.__p47a15Events = [];
window.addEventListener(
  "keydown",
  (e) => {
    if (e.key === "Alt") return; // the overlay trigger, not a chord
    window.__p47a15Events.push({
      key: e.key,
      code: e.code,
      altKey: e.altKey,
      metaKey: e.metaKey,
      ctrlKey: e.ctrlKey,
      shiftKey: e.shiftKey,
    });
  },
  true,
);
`;

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.request.post(`${API}/api/v1/auth/login`, {
  data: { email: "owner@demo.local", password: "demo-password-123" },
});
await ctx.request.put(`${API}/api/v1/settings/user`, {
  data: { key: "tours.disabled", value: true },
});
const page = await ctx.newPage();

const results = [];

async function installHook() {
  await page.evaluate(PARSERS);
}

/** Press via Playwright's own keyboard, then read what the page saw. */
async function pressPlaywright(label, keys, chord) {
  // Re-install per press: every goto() replaces the document, and with it the
  // hook and the captured events.
  await installHook();
  await page.evaluate(() => {
    window.__p47a15Events = [];
  });
  const before = new URL(page.url()).pathname;
  await page.keyboard.press(keys);
  await page.waitForTimeout(600);
  const after = new URL(page.url()).pathname;
  const seen = await page.evaluate(() => window.__p47a15Events);
  const evaluated = await page.evaluate(
    (events) =>
      events.map((e) => ({
        key: e.key,
        code: e.code,
        altKey: e.altKey,
        metaKey: e.metaKey,
        ctrlKey: e.ctrlKey,
        shiftKey: e.shiftKey,
        oldParser: window.__p47a15.chord(e, window.__p47a15.old),
        newParser: window.__p47a15.chord(e, window.__p47a15.new),
      })),
    seen,
  );
  const row = { label, chord, sent: keys, playwright: true, before, after, navigated: before !== after, seen: evaluated };
  results.push(row);
  console.log(
    `[${label}] sent=${keys}\n` +
      evaluated
        .map(
          (e) =>
            `    key=${JSON.stringify(e.key)} code=${e.code} alt=${e.altKey} meta=${e.metaKey} ctrl=${e.ctrlKey} shift=${e.shiftKey}` +
            ` -> old="${e.oldParser}" new="${e.newParser}"`,
        )
        .join("\n") +
      `\n    ${before} -> ${after}${before === after ? "  (NOT NAVIGATED)" : ""}`,
  );
}

/** Dispatch the OS-accurate event (composed character) through raw CDP. */
async function pressRawCdp(label, chord, { key, code, modifiers }) {
  const client = await page.context().newCDPSession(page);
  await installHook();
  await page.evaluate(() => {
    window.__p47a15Events = [];
  });
  const before = new URL(page.url()).pathname;
  await client.send("Input.dispatchKeyEvent", {
    type: "keyDown",
    modifiers,
    key,
    code,
    windowsVirtualKeyCode: codeToVk(code),
    nativeVirtualKeyCode: codeToVk(code),
    text: "",
  });
  await client.send("Input.dispatchKeyEvent", {
    type: "keyUp",
    modifiers,
    key,
    code,
    windowsVirtualKeyCode: codeToVk(code),
    nativeVirtualKeyCode: codeToVk(code),
  });
  await client.send("Input.dispatchKeyEvent", {
    type: "keyUp",
    modifiers: 0,
    key: "Alt",
    code: "AltLeft",
    windowsVirtualKeyCode: 18,
    nativeVirtualKeyCode: 18,
  });
  await page.waitForTimeout(600);
  const after = new URL(page.url()).pathname;
  const seen = await page.evaluate(
    (events) => events.map((e) => ({ ...e, oldParser: window.__p47a15.chord(e, window.__p47a15.old), newParser: window.__p47a15.chord(e, window.__p47a15.new) })),
    await page.evaluate(() => window.__p47a15Events),
  );
  results.push({ label, chord, sent: { key, code, modifiers }, playwright: false, before, after, navigated: before !== after, seen });
  console.log(
    `[${label}] rawCDP sent key=${JSON.stringify(key)} code=${code} mods=${modifiers}\n` +
      seen
        .map((e) => `    key=${JSON.stringify(e.key)} code=${e.code} alt=${e.altKey} -> old="${e.oldParser}" new="${e.newParser}"`)
        .join("\n") +
      `\n    ${before} -> ${after}${before === after ? "  (NOT NAVIGATED)" : ""}`,
  );
  await client.detach();
}

function codeToVk(code) {
  if (code.startsWith("Key")) return code.charCodeAt(3);
  if (code.startsWith("Digit")) return code.charCodeAt(5);
  if (code === "AltLeft") return 18;
  return 0;
}

// --- the measurement ------------------------------------------------------
await page.goto(`${WEB}/tickets`, { waitUntil: "networkidle" });
await page.getByTestId("shortcut-profile-ready").waitFor({ timeout: 30_000 });
await installHook();

const profile = await page.getByTestId("shortcut-profile-ready").getAttribute("data-profile");
console.log(`shortcut profile: ${profile}`);

await pressPlaywright("nav-jump-alt2 (playwright)", "Alt+2", "alt+2");
await page.goto(`${WEB}/tickets`, { waitUntil: "networkidle" });
await pressPlaywright("nav-jump-alt1 (playwright)", "Alt+1", "alt+1");
await page.goto(`${WEB}/tickets`, { waitUntil: "networkidle" });
await pressPlaywright("palette-mod-k (playwright)", "ControlOrMeta+k", "mod+k");

// The OS-accurate Option+2: macOS composes "™" from the Option layer while
// the physical key is Digit2. This is the event a real keyboard sends.
await page.goto(`${WEB}/tickets`, { waitUntil: "networkidle" });
await pressRawCdp("nav-jump-alt2 (OS-accurate composed key)", "alt+2", {
  key: "™",
  code: "Digit2",
  modifiers: 1, // Alt
});

writeFileSync(join(OUT, "chord-measurement.json"), JSON.stringify({ branch: BRANCH, profile, results }, null, 2));
console.log(`\nwrote ${join(OUT, "chord-measurement.json")}`);
await browser.close();
