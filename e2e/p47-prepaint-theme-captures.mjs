// Wave 47 lane A3 `p47-prepaint-theme-flash` — the paint order, proven at the
// frame level against the REAL app on the live e2e stack (never mocks).
//
// Three surfaces, because the lane's research found the wrong mode lives in
// more than one place:
//
//   /login       the first screen a prospect ever sees (the brief's target)
//   /magic-link  a second public door — the entry screens used to write the
//                app's own `light` literal, which the kit reads as its CREAM
//                block, so every entry screen on a light device painted cream
//                and a stored dark preference was overridden on the door
//   /today       the authed surface — React's hydration recovery (#418)
//                re-creates <html> and takes the script-set attribute with
//                it, the wrong-mode flash this lane kills
//
// Each leg installs a PER-FRAME SAMPLER at document_start, ahead of the
// inline bootstrap script and all React code, and records the `data-theme`
// attribute plus the resolved `--m-canvas` value on every animation frame
// from the very first. A wrong-mode frame is any frame whose attribute is
// missing or differs from the settled value, once the CSS has resolved. The
// sampler is the primary evidence; the screenshots back it for the vision
// review.
//
// Usage (the stack must be up — scripts/e2e-stack.sh writes the env file):
//   set -a; . e2e/.e2e-stack.env; set +a
//   node e2e/p47-prepaint-theme-captures.mjs [--surface login|magic-link|today]
//
// Output: design/qa-p47-prepaint-theme-flash/captures/<name>.png plus the
// mechanical record at captures/_run.json.
import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/+$/, "");
const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const OUT = "design/qa-p47-prepaint-theme-flash/captures";
mkdirSync(OUT, { recursive: true });

const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? dflt : process.argv[i + 1];
};
const ONLY_SURFACE = arg("surface", null);

// The KIT's settled values: the app's light is `neutral` (the kit's default
// block), dark is `dark`, and `light` is the kit's CREAM block — the wrong
// paint the entry screens produced. Every leg names the settled value its
// direction must paint, and the canvas the kit resolves for it.
const NEUTRAL_CANVAS = "#f8f7f4";
const SURFACES = [
  {
    name: "login",
    legs: [
      { dir: "dark-preference-on-light-device", stored: "dark", system: "light", expect: "dark", expectCanvas: undefined },
      { dir: "light-preference-on-dark-device", stored: "light", system: "dark", expect: "neutral", expectCanvas: NEUTRAL_CANVAS },
      { dir: "system-only-dark", stored: null, system: "dark", expect: "dark", expectCanvas: undefined },
      { dir: "system-only-light", stored: null, system: "light", expect: "neutral", expectCanvas: NEUTRAL_CANVAS },
    ],
  },
  {
    name: "magic-link",
    legs: [
      { dir: "dark-preference-on-light-device", stored: "dark", system: "light", expect: "dark", expectCanvas: undefined },
      { dir: "light-preference-on-dark-device", stored: "light", system: "dark", expect: "neutral", expectCanvas: NEUTRAL_CANVAS },
      { dir: "system-only-light", stored: null, system: "light", expect: "neutral", expectCanvas: NEUTRAL_CANVAS },
    ],
  },
  {
    name: "today",
    authed: true,
    legs: [
      { dir: "dark-preference-on-light-device", stored: "dark", system: "light", expect: "dark", expectCanvas: undefined },
      { dir: "light-preference-on-dark-device", stored: "light", system: "dark", expect: "neutral", expectCanvas: NEUTRAL_CANVAS },
    ],
  },
];

const browser = await chromium.launch();
const record = [];
let failures = 0;

for (const surface of SURFACES) {
  if (ONLY_SURFACE && surface.name !== ONLY_SURFACE) continue;
  for (const leg of surface.legs) {
    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 },
      colorScheme: leg.system,
    });
    if (surface.authed) {
      // The cookie session is the credential: signing in through the
      // context's request jar puts the Set-Cookie in the same store the
      // pages read, so /today renders authed rather than bouncing to login.
      for (let attempt = 0; attempt < 20; attempt++) {
        const res = await context.request.post(`${API}/api/v1/auth/login`, {
          data: { email: EMAIL, password: PASSWORD },
        });
        if (res.ok()) break;
        await new Promise((r) => setTimeout(r, 3_000));
      }
    }
    const page = await context.newPage();

    // The sampler starts at document_start — ahead of the bootstrap script —
    // so every frame the browser can paint is recorded, including any frame
    // with no attribute (the wrong-mode flash) or the cream block.
    await page.addInitScript(() => {
      const samples = [];
      const note = () => {
        samples.push({
          frame: samples.length,
          theme: document.documentElement.getAttribute("data-theme"),
          canvas: getComputedStyle(document.documentElement).getPropertyValue("--m-canvas").trim(),
        });
        if (samples.length < 900) requestAnimationFrame(note);
      };
      requestAnimationFrame(note);
      window.__PAINT_SAMPLES__ = samples;
    });
    if (leg.stored !== null) {
      await page.addInitScript((theme) => {
        window.localStorage.setItem("marula-theme", theme);
      }, leg.stored);
    }

    await page.goto(`${WEB}/${surface.name}`, { waitUntil: "commit" });
    await page.waitForTimeout(150);
    const firstPaint = join(OUT, `${surface.name}-${leg.dir}--first-paint.png`);
    await page.screenshot({ path: firstPaint });
    await page.waitForTimeout(3000);
    const settled = join(OUT, `${surface.name}-${leg.dir}--settled.png`);
    await page.screenshot({ path: settled });

    const samples = await page.evaluate(() => window.__PAINT_SAMPLES__);
    const painted = samples.filter((s) => s.canvas !== "");
    const missing = painted.filter((s) => s.theme === null);
    const wrongValue = painted.filter((s) => s.theme !== null && s.theme !== leg.expect);
    const wrong = missing.concat(wrongValue);
    const firstSettledFrame = painted.find((s) => s.theme !== null);
    const canvasAtSettle = painted.length > 0 ? painted[painted.length - 1].canvas : "";
    const wrongCanvas =
      leg.expectCanvas !== undefined && canvasAtSettle.toLowerCase() !== leg.expectCanvas.toLowerCase();
    const wrongFrames = wrong.length + (wrongCanvas ? 1 : 0);
    if (wrongFrames > 0) failures += 1;

    record.push({
      surface: surface.name,
      leg: leg.dir,
      stored: leg.stored,
      system: leg.system,
      expectTheme: leg.expect,
      framesSampled: samples.length,
      framesPainted: painted.length,
      wrongModeFrames: wrong.length,
      wrongModeDetail: wrong.slice(0, 5),
      firstSettledFrame: firstSettledFrame ?? null,
      canvasAtSettle,
      wrongCanvas,
      captures: [firstPaint, settled],
      verdict: wrongFrames === 0 ? "PASS" : "FAIL",
    });
    const r = record[record.length - 1];
    console.log(
      `${r.verdict} ${surface.name}/${leg.dir}: ${painted.length} painted frames, ${wrong.length} wrong-mode frames` +
        (wrongCanvas ? `, canvas ${canvasAtSettle} != ${leg.expectCanvas}` : ""),
    );

    await context.close();
  }
}

await browser.close();
writeFileSync(join(OUT, "_run.json"), `${JSON.stringify(record, null, 2)}\n`);
console.log(failures === 0 ? "ALL LEGS PASS" : `${failures} LEG(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
