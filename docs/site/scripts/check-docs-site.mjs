#!/usr/bin/env node
// Verifies the docs site: every relative markdown link resolves to a file
// that exists (anchors are ignored), generated pages match regeneration
// (pin & fixture sync), and the PUBLISHING path (ADR 0009) works — the
// publisher renders every page into a byte-deterministic static HTML
// bundle with no dead in-bundle hrefs. Exit 1 on any failure.
import { existsSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs";
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
execFileSync("node", [join(site, "scripts/generate-api-reference.mjs")], {
  cwd: root,
  stdio: "pipe",
});
if (readFileSync(join(site, "reference/api.md"), "utf8") !== before) {
  console.error(
    "STALE reference/api.md — does not match packages/sdk/openapi.json; regenerate.",
  );
  failed++;
}
const before2 = readFileSync(join(site, "reference/upgrade-matrix.md"), "utf8");
execFileSync("node", [join(site, "scripts/generate-upgrade-matrix.mjs")], {
  cwd: root,
  stdio: "pipe",
});
if (readFileSync(join(site, "reference/upgrade-matrix.md"), "utf8") !== before2) {
  console.error(
    "STALE reference/upgrade-matrix.md — does not match fixtures/upgrades/; regenerate.",
  );
  failed++;
}

// Publishing path (ADR 0009): publish twice into disposable dirs and
// assert the bundle is complete (every md page mirrored to html),
// byte-deterministic, and free of dead in-bundle hrefs.
const runs = ["a", "b"].map((tag) => {
  const out = join(site, `dist-check-${tag}`);
  execFileSync("node", [join(site, "scripts/publish.mjs"), "--out", out], {
    cwd: root,
    stdio: "pipe",
  });
  return out;
});
try {
  function walkAny(dir) {
    return readdirSync(dir).flatMap((n) => {
      const p = join(dir, n);
      return statSync(p).isDirectory() ? walkAny(p) : [p];
    });
  }
  const relAll = (dir) =>
    walkAny(dir).map((f) => relative(dir, f)).sort();

  const setA = relAll(runs[0]);
  const setB = relAll(runs[1]);
  for (const f of setA)
    if (!setB.includes(f)) {
      console.error(`PUBLISH asymmetry: ${f} present in run a only`);
      failed++;
    }
  for (const f of setB)
    if (!setA.includes(f)) {
      console.error(`PUBLISH asymmetry: ${f} present in run b only`);
      failed++;
    }
  for (const f of setA) {
    const a = readFileSync(join(runs[0], f));
    const b = readFileSync(join(runs[1], f));
    if (Buffer.compare(a, b) !== 0) {
      console.error(`PUBLISH non-deterministic output for ${f}`);
      failed++;
    }
  }

  // Completeness: every site markdown page published its html twin.
  const finals = new Set(setA);
  const htmlOf = (relMd) =>
    relMd.endsWith(".md") ? `${relMd.slice(0, -3)}.html` : relMd;
  for (const file of files) {
    const relMd = relative(site, file);
    if (relMd.startsWith("scripts")) continue;
    if (!finals.has(htmlOf(relMd))) {
      console.error(`PUBLISH missing page: ${relMd} not published`);
      failed++;
    }
  }

  // Dead-href sweep across the published bundle.
  const hrefRe = /<a href="([^"#]+)(#[^" ]*)?"/g;
  for (const rel of finals) {
    if (!rel.endsWith(".html")) continue;
    const text = readFileSync(join(runs[0], rel), "utf8");
    let m;
    while ((m = hrefRe.exec(text))) {
      if (/^(https?:|mailto:)/.test(m[1])) continue;
      const resolved = join(
        dirname(join(runs[0], rel)),
        decodeURIComponent(m[1]),
      );
      if (!existsSync(resolved)) {
        console.error(`PUBLISH dead href: ${rel} -> ${m[1]}`);
        failed++;
      }
    }
  }
} finally {
  for (const dir of runs) rmSync(dir, { recursive: true, force: true });
}

if (failed) {
  console.error(`\n${failed} problem(s)`);
  process.exit(1);
}
console.log(
  `docs-site check OK: ${files.length} pages, links resolve, generated pages in sync, publishing path verified`,
);
