#!/usr/bin/env node
// Information-architecture check for the documentation tree (Wave 54 lane
// R8a). The site checker (check-docs-site.mjs) proves links resolve and the
// publish path works; this proves the tree can be navigated at all:
//
//   1. LINK    every relative markdown link in docs/ resolves to a real file
//   2. ANCHOR  a `#anchor` link resolves to a heading in its target page
//   3. ORPHAN  every site page is reachable from docs/site/index.md, or is
//              declared site-internal in ALLOWED_UNLINKED with a reason
//   4. INDEX   every page under tutorials/ how-to/ reference/ explanation/
//              is linked directly from that quadrant's index.md
//   5. AUDIENCE every **Audience:** tag starts with a known audience word
//   6. WAYFIND the site index links the task entry points, the glossary and
//              troubleshooting, and every start page reaches troubleshooting
//
// Exit 1 on any failure. Node only: no install, no network.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const docs = join(dirname(fileURLToPath(import.meta.url)), "../..");
const site = join(docs, "site");
const repo = join(docs, "..");

// Pages that exist for repository readers and are deliberately not linked
// from the site. Each entry needs a reason; an entry without one fails.
const ALLOWED_UNLINKED = new Map([
  [
    "README.md",
    "site-internal contributor page (layout, generated pages, publish commands)",
  ],
]);

const AUDIENCES = ["both", "admin", "staff", "developer", "owner", "site"];

const QUADRANTS = {
  tutorials: "tutorials/index.md",
  "how-to": "how-to/index.md",
  reference: "reference/index.md",
  explanation: "explanation/index.md",
};

const START_PAGES = [
  "start/index.md",
  "start/owners.md",
  "start/admins.md",
  "start/technicians.md",
  "start/operators.md",
  "start/agents.md",
];

/** Pages every site index must reach, by the names readers use. */
const WAYFINDING = {
  "index.md": [
    "start/index.md",
    "troubleshooting.md",
    "reference/glossary.md",
  ],
};

const problems = [];
const fail = (rule, message) => problems.push(`${rule} ${message}`);

// --- file walk ---------------------------------------------------------------

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    if (name === "node_modules" || name === "dist") return [];
    if (name.startsWith("dist-check-")) return [];
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : path.endsWith(".md") ? [path] : [];
  });
}

const siteFiles = walk(site);
const docsFiles = walk(docs).filter((f) => !siteFiles.includes(f));
const relSite = (f) => relative(site, f).split("\\").join("/");
const relDocs = (f) => relative(docs, f).split("\\").join("/");

// --- 1/2. links and anchors --------------------------------------------------

/** Drop fenced blocks and inline code so `[label](url)` inside prose about
 *  markdown is not read as a link. */
function prose(src) {
  return src
    .replace(/^```[\s\S]*?^```/gm, "")
    .replace(/`[^`\n]*`/g, "");
}

const slugify = (text) =>
  text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64) || "section";

function anchorsOf(file) {
  const set = new Set();
  const src = prose(readFileSync(file, "utf8"));
  for (const line of src.split("\n")) {
    const m = line.match(/^#{1,6}\s+(.+)$/);
    if (m) set.add(slugify(m[1].replace(/[*`]/g, "")));
  }
  // Generated pages declare their own ids with inline HTML
  // (`<a id="schema-x"></a>`), which the renderer keeps as targets.
  for (const m of src.matchAll(/<a id="([^"]+)"><\/a>/g)) set.add(m[1]);
  return set;
}

function linksIn(file) {
  const out = [];
  const re = /\[[^\]]*\]\(([^)\s]+)\)/g;
  const src = prose(readFileSync(file, "utf8"));
  let m;
  while ((m = re.exec(src))) out.push(m[1]);
  return out;
}

function checkLinks(files, label) {
  for (const file of files) {
    for (const target of linksIn(file)) {
      if (/^(https?:|mailto:|tel:)/.test(target)) continue;
      const hashAt = target.indexOf("#");
      const path = hashAt === -1 ? target : target.slice(0, hashAt);
      const anchor = hashAt === -1 ? "" : decodeURIComponent(target.slice(hashAt + 1));
      const where = `${label}:${relative(repo, file).split("\\").join("/")}`;
      if (!path) {
        if (anchor && !anchorsOf(file).has(anchor)) {
          fail("ANCHOR", `${where} -> ${target} (no such heading in this page)`);
        }
        continue;
      }
      const resolved = resolve(dirname(file), decodeURIComponent(path));
      if (!existsSync(resolved)) {
        fail("LINK", `${where} -> ${target}`);
        continue;
      }
      if (anchor && resolved.endsWith(".md") && !anchorsOf(resolved).has(anchor)) {
        fail(
          "ANCHOR",
          `${where} -> ${target} (no such heading in ${relative(repo, resolved)})`,
        );
      }
    }
  }
}
checkLinks(siteFiles, "site");
checkLinks(docsFiles, "docs");

// --- 3. orphans --------------------------------------------------------------

const siteLinks = new Map(siteFiles.map((f) => [relSite(f), linksIn(f)]));

/** Every site page reachable by following .md links from the site index. */
const seen = new Set(["index.md"]);
const queue = ["index.md"];
while (queue.length) {
  const current = queue.pop();
  for (const target of siteLinks.get(current) ?? []) {
    if (/^(https?:|mailto:|tel:|#)/.test(target)) continue;
    const path = target.split("#")[0];
    if (!path) continue;
    const rel = relative(site, resolve(site, dirname(current), decodeURIComponent(path)))
      .split("\\")
      .join("/");
    if (!rel.endsWith(".md") || seen.has(rel) || !siteLinks.has(rel)) continue;
    seen.add(rel);
    queue.push(rel);
  }
}
for (const rel of siteLinks.keys()) {
  if (seen.has(rel)) continue;
  const excuse = ALLOWED_UNLINKED.get(rel);
  if (excuse) continue;
  fail("ORPHAN", `site/${rel} — no path from site/index.md; link it or declare it site-internal`);
}
for (const [rel, excuse] of ALLOWED_UNLINKED) {
  if (!siteLinks.has(rel)) fail("ORPHAN", `allowlist names site/${rel}, which no longer exists`);
  if (!excuse || excuse.trim().length < 20) {
    fail("ORPHAN", `allowlist entry for site/${rel} carries no usable reason`);
  }
}

// --- 4. index coverage -------------------------------------------------------

for (const [quadrant, indexRel] of Object.entries(QUADRANTS)) {
  const linked = new Set(
    linksIn(join(site, indexRel))
      .map((t) => relative(site, resolve(site, quadrant, t.split("#")[0])).split("\\").join("/"))
  );
  for (const file of siteFiles) {
    const rel = relSite(file);
    if (!rel.startsWith(`${quadrant}/`) || rel === indexRel) continue;
    if (!linked.has(rel)) {
      fail("INDEX", `site/${rel} is not linked from site/${indexRel}`);
    }
  }
}

// --- 5. audience vocabulary --------------------------------------------------

for (const file of siteFiles) {
  const src = readFileSync(file, "utf8");
  const tag = src.match(/^>\s*\*\*Audience:\*\*\s*(.+)$/m);
  if (!tag) continue;
  const word = tag[1].trim().split(/[\s,.;:/—–-]/)[0].toLowerCase();
  if (!AUDIENCES.includes(word)) {
    fail(
      "AUDIENCE",
      `site/${relSite(file)} — tag starts with "${word}", expected one of ${AUDIENCES.join(", ")}`,
    );
  }
}

// --- 6. wayfinding -----------------------------------------------------------

for (const [indexRel, required] of Object.entries(WAYFINDING)) {
  const linked = new Set(
    linksIn(join(site, indexRel)).map((t) =>
      relative(site, resolve(site, dirname(indexRel), t.split("#")[0]))
        .split("\\")
        .join("/"),
    ),
  );
  for (const target of required) {
    if (!linked.has(target)) {
      fail("WAYFIND", `site/${indexRel} does not link site/${target}`);
    }
  }
}

for (const startRel of START_PAGES) {
  if (!existsSync(join(site, startRel))) {
    fail("WAYFIND", `site/${startRel} is missing from the entry-point set`);
    continue;
  }
  if (startRel === "start/index.md") continue;
  const targets = linksIn(join(site, startRel)).map((t) =>
    relative(site, resolve(site, dirname(startRel), t.split("#")[0])).split("\\").join("/"),
  );
  if (!targets.includes("troubleshooting.md")) {
    fail("WAYFIND", `site/${startRel} does not link site/troubleshooting.md`);
  }
}

// --- 7. engineering docs reachable from docs/README.md -----------------------

if (!existsSync(join(docs, "README.md"))) {
  fail("INDEX", "docs/README.md is missing — the engineering docs have no entry point");
} else {
  const linked = new Set(
    linksIn(join(docs, "README.md")).map((t) =>
      relative(docs, resolve(docs, t.split("#")[0])).split("\\").join("/"),
    ),
  );
  for (const file of docsFiles) {
    const rel = relDocs(file);
    if (rel.includes("/")) continue; // subdirectories have their own indexes
    if (rel === "README.md") continue;
    if (!linked.has(rel)) {
      fail("INDEX", `docs/${rel} is not linked from docs/README.md`);
    }
  }
}

// --- report ------------------------------------------------------------------

if (problems.length) {
  for (const problem of problems) console.error(problem);
  console.error(`\n${problems.length} problem(s)`);
  process.exit(1);
}
console.log(
  `docs IA check OK: ${siteFiles.length} site pages reachable and indexed, ` +
    `${docsFiles.length + siteFiles.length} pages of links resolve, ` +
    `${START_PAGES.length} entry points in place`,
);
