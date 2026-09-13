#!/usr/bin/env node
// Verifies the docs site: every relative markdown link resolves to a file
// that exists (anchors are ignored), and generated pages match regeneration
// (pin & fixture sync). Exit 1 on any failure.
import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const root = join(site, "../..");

function walk(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p) : p.endsWith(".md") ? [p] : [];
  });
}
const files = walk(site);
let failed = 0;

for (const file of files) {
  const src = readFileSync(file, "utf8");
  const re = /\[[^\]]*\]\(([^)\s]+)\)/g;
  let m;
  while ((m = re.exec(src))) {
    const target = m[1];
    if (/^(https?:|mailto:|#)/.test(target)) continue;
    const [path] = target.split("#");
    if (!path) continue;
    const resolved = join(dirname(file), decodeURIComponent(path));
    if (!existsSync(resolved)) {
      console.error(`BROKEN ${relative(root, file)} -> ${target}`);
      failed++;
    }
  }
}

// Generated pages must match regeneration from the current pin/fixtures.
const before = readFileSync(join(site, "reference/api.md"), "utf8");
execFileSync("node", [join(site, "scripts/generate-api-reference.mjs")], { cwd: root, stdio: "pipe" });
if (readFileSync(join(site, "reference/api.md"), "utf8") !== before) {
  console.error("STALE reference/api.md — does not match packages/sdk/openapi.json; regenerate.");
  failed++;
}
const before2 = readFileSync(join(site, "reference/upgrade-matrix.md"), "utf8");
execFileSync("node", [join(site, "scripts/generate-upgrade-matrix.mjs")], { cwd: root, stdio: "pipe" });
if (readFileSync(join(site, "reference/upgrade-matrix.md"), "utf8") !== before2) {
  console.error("STALE reference/upgrade-matrix.md — does not match fixtures/upgrades/; regenerate.");
  failed++;
}

if (failed) {
  console.error(`\n${failed} problem(s)`);
  process.exit(1);
}
console.log(`docs-site check OK: ${files.length} pages, links resolve, generated pages in sync`);
