// Lighthouse sweep for the p34 verdict walk: seven key routes, mobile +
// desktop presets, JSON evidence per run. Exit code 1 on any failure.
import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const BASE = process.env.SITE_URL ?? "http://localhost:3100";
const OUT = resolve("design/qa-p34-spec-verdict-sweep/lighthouse");
const NPX = "/Users/iordn/.nvm/versions/node/v24.11.1/bin/npx";
const ROUTES = [
  ["index", "/"],
  ["ticketing", "/ticketing"],
  ["compare", "/compare"],
  ["billing", "/billing"],
  ["client-portal", "/client-portal"],
  ["integrations", "/integrations"],
  ["developers", "/developers"],
];
mkdirSync(OUT, { recursive: true });

let failed = 0;
for (const [name, path] of ROUTES) {
  for (const mode of ["mobile", "desktop"]) {
    const args = [
      "--yes",
      "lighthouse",
      `${BASE}${path}`,
      "--output",
      "json",
      "--output-path",
      `${OUT}/${name}-${mode}.json`,
      "--quiet",
      "--chrome-flags=--headless --no-sandbox --disable-dev-shm-usage",
    ];
    if (mode === "desktop") args.push("--preset=desktop");
    const r = spawnSync(NPX, args, { stdio: "ignore" });
    if (r.status !== 0) {
      failed++;
      console.log(`${name}-${mode} FAILED (exit ${r.status})`);
      continue;
    }
    const report = JSON.parse(
      (await import("node:fs")).readFileSync(`${OUT}/${name}-${mode}.json`, "utf8"),
    );
    const c = report.categories;
    console.log(
      `${name}-${mode}: perf ${Math.round(c.performance.score * 100)}, a11y ${Math.round(c.accessibility.score * 100)}, bp ${Math.round(c["best-practices"].score * 100)}, seo ${Math.round(c.seo.score * 100)}`,
    );
  }
}
process.exit(failed ? 1 : 0);
