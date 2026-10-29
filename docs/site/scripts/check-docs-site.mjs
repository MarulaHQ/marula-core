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

// Audience-tag check (Wave 27 lane 2): every how-to and tutorial page
// (except index pages) states its audience up front.
for (const file of files) {
  const rel = relative(site, file).replaceAll("\\", "/");
  const m = rel.match(/^(how-to|tutorials)\/([^/]+)\.md$/);
  if (!m || m[2] === "index") continue;
  const src = readFileSync(file, "utf8");
  if (!src.includes("**Audience:**")) {
    console.error(`AUDIENCE ${rel} — missing the **Audience:** tag line`);
    failed++;
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

// In-app help index manifest (Wave 26 lane 2, ADR 0009 extension):
// every manifest doc path must exist, its title must match the page's
// first heading, its audience must match the page's **Audience:** tag,
// route keys must be unique, and the web app's imported copy must be
// byte-identical — so a moved page or a drifted key FAILS CI instead of
// going stale in-product.
{
  const manifestPath = join(site, "help-manifest.json");
  const webCopy = join(root, "apps/web/lib/help/help-manifest.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  if (readFileSync(manifestPath, "utf8") !== readFileSync(webCopy, "utf8")) {
    console.error(
      "HELP-MANIFEST apps/web/lib/help/help-manifest.json is not byte-identical to docs/site/help-manifest.json — copy it again.",
    );
    failed++;
  }
  const seenKeys = new Set();
  for (const [routeKey, entry] of Object.entries(manifest.routes)) {
    if (seenKeys.has(routeKey)) {
      console.error(`HELP-MANIFEST duplicate route key ${routeKey}`);
      failed++;
    }
    seenKeys.add(routeKey);
    for (const doc of entry.docs ?? []) {
      const mdPath = join(site, `${doc.path}.md`);
      if (!existsSync(mdPath)) {
        console.error(
          `HELP-MANIFEST route ${routeKey} -> ${doc.path}.md does not exist (moved page? update the manifest)`,
        );
        failed++;
        continue;
      }
      const src = readFileSync(mdPath, "utf8");
      const heading = src.match(/^#\s+(.+)$/m);
      if (!heading || heading[1].trim() !== doc.title) {
        console.error(
          `HELP-MANIFEST route ${routeKey} -> ${doc.path}: title ${JSON.stringify(doc.title)} does not match the page heading${heading ? ` ${JSON.stringify(heading[1].trim())}` : " (none)"}`,
        );
        failed++;
      }
      // Explanation pages carry no audience tag (only how-to/tutorial
      // pages must); a manifest entry for an untagged page must use "all".
      const aud = src.match(/>\s*\*\*Audience:\*\*\s*([a-z]+)/);
      if (!aud && doc.audience !== "all") {
        console.error(
          `HELP-MANIFEST route ${routeKey} -> ${doc.path}: audience ${JSON.stringify(doc.audience)} set for an untagged page — use "all" or fix the page`,
        );
        failed++;
      } else if (aud && !aud[1].includes(doc.audience)) {
        console.error(
          `HELP-MANIFEST route ${routeKey} -> ${doc.path}: audience ${JSON.stringify(doc.audience)} not found in the page's **Audience:** tag`,
        );
        failed++;
      }
      if (!/^\d{4}-\d{2}-\d{2}$/.test(doc.updated ?? "")) {
        console.error(
          `HELP-MANIFEST route ${routeKey} -> ${doc.path}: 'updated' must be YYYY-MM-DD`,
        );
        failed++;
      }
    }
  }
}

// Licence boundary (ADR 0025): the legal text, the notice and the README
// section must all exist; a missing licence file cannot pass silently.
{
  const licencePath = join(site, "LICENSE");
  const noticePath = join(site, "NOTICE");
  const readme = readFileSync(join(site, "README.md"), "utf8");
  if (!existsSync(licencePath)) {
    console.error("LICENSE docs/site/LICENSE is missing");
    failed++;
  } else if (!readFileSync(licencePath, "utf8").includes("Creative Commons Attribution 4.0 International")) {
    console.error("LICENSE docs/site/LICENSE does not carry the CC BY 4.0 legal text");
    failed++;
  }
  if (!existsSync(noticePath)) {
    console.error("LICENSE docs/site/NOTICE is missing");
    failed++;
  }
  if (!readme.includes("CC BY 4.0")) {
    console.error("LICENSE docs/site/README.md does not name CC BY 4.0");
    failed++;
  }
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

// Information architecture (Wave 54 lane R8a): every published page is
// reachable from the site index and indexed by its quadrant, the audience
// vocabulary holds, and the task entry points, glossary and troubleshooting
// pages are wired in. Run last, so a link failure above is reported first.
try {
  execFileSync(process.execPath, [join(site, "scripts/check-docs-ia.mjs")], {
    cwd: root,
    stdio: "inherit",
  });
} catch {
  failed++;
}

if (failed) process.exit(1);
console.log(
  `docs-site check OK: ${files.length} pages, links resolve, generated pages in sync, publishing path verified, IA indexed`,
);
