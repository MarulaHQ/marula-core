// p47-pack-states.mjs — Wave 47 lane A13, `p47-vision-captures`.
//
// Enumerate what the pack actually declares, straight from the mockups.
//
// Every mockup carries a `ST` array of `[id, title, apply]` and populates a
// `<select id="jump">` from it (`clients.html:274,387,389`), plus its theme
// switch. So the pack's real coverage is not "nine surfaces" - it is every
// surface x every declared state x every flag x both themes, and the mockups
// declare it themselves. This reads that declaration instead of guessing.
//
// Output: design/qa-p47-vision-captures/PACK-STATES.json, plus a coverage
// matrix printed as markdown for the ledger.
//
// Usage: node scripts/p47-pack-states.mjs
import { chromium } from "playwright";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const LANE = "design/qa-p47-vision-captures";
mkdirSync(LANE, { recursive: true });
const PACK = process.env.MARULA_PACK_ROOT ?? "/Users/iordn/Documents/GitHub/Marula/brand/packs/core-surface-pack";

const MOCKUPS = [
  "tickets-list", "clients", "approvals", "today", "dispatch",
  "settings", "quoting", "knowledge-vault", "workflow-builder",
  "portal", "auth", "mobile-core", "mobile-scan",
];

const browser = await chromium.launch();
const out = { generatedAt: new Date().toISOString(), pack: PACK, surfaces: [] };

for (const name of MOCKUPS) {
  const file = resolve(PACK, "source", `${name}.html`);
  if (!existsSync(file)) {
    out.surfaces.push({ surface: name, error: "missing" });
    continue;
  }
  const page = await browser.newPage({ viewport: { width: 1600, height: 994 } });
  await page.goto(`file://${file}`, { waitUntil: "load", timeout: 60_000 });
  await page.waitForTimeout(1200);

  const read = await page.evaluate(() => {
    const opts = [...document.querySelectorAll("#jump option")].map((o) => ({
      id: o.value,
      title: o.textContent.replace(/^\s*\d+\s*·\s*/, "").trim(),
    }));
    // The flag/toggle axes: any other select, plus the theme buttons.
    const selects = [...document.querySelectorAll("select")].map((s) => ({
      id: s.id || s.getAttribute("aria-label") || "(unnamed)",
      labels: s.id === "jump" ? [] : [...s.options].map((o) => o.textContent.trim()),
    }));
    const themeNames = [...document.querySelectorAll("button, [role=radio]")]
      .map((b) => b.textContent.trim())
      .filter((t) => /^(neutral|dark|light|sand)$/i.test(t));
    // Can the mockup be driven? This is what makes a state-comparison possible.
    const canDrive = typeof window.ST !== "undefined" || typeof window.setState === "function";
    const sourceCount = typeof window.ST !== "undefined" ? window.ST.length : null;
    return { opts, selects, themeNames: [...new Set(themeNames)], canDrive, sourceCount };
  });
  await page.close();

  out.surfaces.push({
    surface: name,
    states: read.opts,
    stateCount: read.opts.length,
    flags: read.selects.filter((s) => s.labels.length > 0),
    themes: read.themeNames,
    drivable: read.canDrive,
    declaredStateCount: read.sourceCount,
  });
  console.log(
    `${name.padEnd(18)} states=${String(read.opts.length).padStart(2)} themes=${read.themeNames.join("/") || "-"} flags=${read.selects.filter((s) => s.labels.length > 0).length} drivable=${read.canDrive}`,
  );
}

await browser.close();
writeFileSync(resolve(LANE, "PACK-STATES.json"), JSON.stringify(out, null, 2));

const totalStates = out.surfaces.reduce((n, s) => n + (s.stateCount ?? 0), 0);
const web = out.surfaces.filter((s) => (s.stateCount ?? 0) > 0);
console.log(`\n[states] ${totalStates} declared states across ${web.length} mockups`);
const lines = ["# The pack's declared coverage", "", "| Surface | States | Themes | Flags |", "|---|---|---|---|"];
for (const s of out.surfaces) {
  if (s.error) continue;
  lines.push(`| \`${s.surface}\` | ${s.stateCount} | ${(s.themes ?? []).join(", ") || "-"} | ${(s.flags ?? []).map((f) => f.labels.length).reduce((a, b) => a + b, 0) || 0} |`);
}
writeFileSync(resolve(LANE, "PACK-COVERAGE.md"), lines.join("\n") + "\n");
console.log(`[states] -> ${LANE}/PACK-STATES.json and PACK-COVERAGE.md`);
