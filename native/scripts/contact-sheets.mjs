/**
 * Contact-sheet the pack captures for the vision review.
 *
 * Sheets hold at most 8 screens (the provider rejects bigger image batches),
 * laid out as capture-next-to-pack-state so a reviewer can diff a pair by
 * eye. Run after capture-pack.mjs.
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const outDir = path.join(root, "design/qa-p41-ui-mobile/captures");
const sheetsDir = path.join(root, "design/qa-p41-ui-mobile/sheets");

/** The pack's own PNG for each state, when it has one. */
const PACK = "/Users/iordn/Documents/GitHub/meridian-open-psa-core/brand/marula-mockups/marula-surface-pack/mobile";
const PACK_STATES = {
  "board-list": "01-board-list",
  "ticket-detail": "02-ticket-detail",
  "note-composer": "03-note-composer",
  "time-day": "04-time-day",
  "stop-and-log": "05-stop-and-log",
  more: "06-more",
  notifications: "07-notifications",
  "deep-linked-open": "08-deep-linked-open",
  locked: "09-locked",
  login: "10-login",
  mfa: "11-mfa",
  "scan-camera": "scan-01-camera",
  "scan-found": "scan-02-found",
  "scan-ambiguous": "scan-03-ambiguous",
  "scan-unknown": "scan-04-unknown",
  "camera-off": "scan-05-camera-off",
  "asset-record": "scan-06-asset-record",
  move: "scan-07-move",
  "stock-item": "15-stock-item",
  "stock-location": "14-stock-location",
  "stock-serial": "13-stock-serial",
  "tool-checkout": "scan-08-tool-checkout",
  "tool-overdue": "scan-09-tool-overdue",
};

/** Scan states live in states/, core states in core/states/. */
function packPathFor(theme, file) {
  const folder = file.startsWith("scan-") ? "states" : "core/states";
  return `${PACK}/${theme}/${folder}/${file}.png`;
}

const GROUPS = [
  ["01-chrome", ["board-list", "ticket-detail", "note-composer", "time-day"]],
  ["02-sheets", ["stop-and-log", "more", "notifications", "deep-linked-open"]],
  ["03-auth", ["locked", "login", "mfa", "scan-camera"]],
  ["04-scan", ["scan-found", "scan-unknown", "camera-off", "stock-item"]],
  ["05-fieldops", ["stock-location", "stock-serial", "tool-checkout", "tool-overdue", "move", "asset-record"]],
];

const THEMES = ["light", "dark"];

async function exists(file) {
  try {
    await readFile(file);
    return true;
  } catch {
    return false;
  }
}

async function main() {
  await mkdir(sheetsDir, { recursive: true });
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1680, height: 1200 }, deviceScaleFactor: 1 });
  const written = [];

  for (const theme of THEMES) {
    for (const [name, states] of GROUPS) {
      const pairs = [];
      for (const state of states) {
        const mine = path.join(outDir, `${theme}-${state}.png`);
        if (!(await exists(mine))) continue;
        const pack = PACK_STATES[state];
        const packFile = pack === undefined ? null : packPathFor(theme, pack);
        // Inline BOTH sides: a file:// image never loads inside setContent.
        let packData = null;
        if (packFile !== null && (await exists(packFile))) {
          const bytes = await readFile(packFile);
          packData = `data:image/png;base64,${bytes.toString("base64")}`;
        }
        const data = await readFile(mine);
        pairs.push({
          state,
          mine: `data:image/png;base64,${data.toString("base64")}`,
          pack: packData,
        });
      }
      if (pairs.length === 0) continue;

      // chunk to 8 per sheet
      for (let i = 0; i < pairs.length; i += 8) {
        const chunk = pairs.slice(i, i + 8);
        const html = `<!doctype html><meta charset="utf-8">
<style>
  body { margin:0; font: 13px/1.4 -apple-system, "Segoe UI", sans-serif; background:#F4F1EA; color:#2B2A33; }
  h1 { font-size: 18px; margin: 18px 18px 4px; }
  .row { display:flex; gap:18px; padding: 12px 18px 24px; flex-wrap: wrap; }
  figure { margin:0; }
  figcaption { font-weight: 600; padding-bottom: 6px; }
  img { height: 720px; display:block; border:1px solid #D8D2C4; border-radius: 10px; background:#fff; }
  .pair { display:flex; gap:10px; }
  .tag { font-size: 11px; color:#6B6558; padding-top:4px; }
  .none { height:720px; width:340px; display:flex; align-items:center; justify-content:center;
          border:1px dashed #C9C2B2; border-radius:10px; color:#6B6558; }
</style>
<h1>${theme} · ${name} · capture vs pack</h1>
<div class="row">
${chunk
  .map(
    (entry) => `<figure>
  <figcaption>${entry.state}</figcaption>
  <div class="pair">
    <div><img src="${entry.mine}"><div class="tag">build</div></div>
    ${
      entry.pack !== null
        ? `<div><img src="${entry.pack}"><div class="tag">pack</div></div>`
        : `<div class="none">no pack state</div>`
    }
  </div>
</figure>`,
  )
  .join("\n")}
</div>`;
        await page.setContent(html, { waitUntil: "load" });
        const suffix = chunk.length < pairs.length ? `-${i / 8 + 2}` : "";
        const file = path.join(sheetsDir, `${theme}-${name}${suffix}.png`);
        await page.screenshot({ path: file, fullPage: true });
        written.push(path.basename(file));
        console.log(`sheet ${path.basename(file)}`);
      }
    }
  }

  await browser.close();
  await writeFile(path.join(sheetsDir, "INDEX.json"), JSON.stringify(written, null, 2));
  console.log(`\n${written.length} sheets in ${sheetsDir}`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
