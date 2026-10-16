#!/usr/bin/env node
/**
 * check-evidence-writes.mjs — one run may only write its own lane's evidence.
 *
 * Wave 47 lane A17 (`p47-lane-env-hygiene`).  The incident: a full Playwright
 * sweep on a lane that owned none of them rewrote seven TRACKED files belonging
 * to three other lanes — `design/qa-p29-accessibility-deep/axe-results.json`
 * (the axe spec overwrites it), four captures under
 * `design/qa-p32-today-day-pack/captures/` and two under
 * `design/qa-p34-plugin-surface-closeout/captures/`.  They were restored by
 * hand; a lane gating with `git add -A` would have committed the churn over
 * three lanes' records instead.
 *
 * The committed suite (specs + helpers, the part `pnpm e2e` runs unattended)
 * must resolve its evidence directory through `e2e/helpers/evidence.ts`, so the
 * running lane always writes its own `design/qa-<lane>/`.  This check fails on
 * any hard-coded design/qa-* path that comes back in that scope, in the same
 * spirit as check-shared-chrome.mjs and check-theme-packs.mjs.
 *
 * Standalone capture scripts (`e2e/p*-captures.mjs`) are invoked deliberately by
 * the lane that owns them, so a frozen path there is a WARNING, not a failure —
 * they are listed so their owners can adopt the same helper.
 *
 *   node scripts/check-evidence-writes.mjs              # gate
 *   node scripts/check-evidence-writes.mjs --self-test  # the resolver's rules
 *
 * Exit 0 = clean · 1 = a spec/helper writes into a frozen evidence path.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { join, relative, resolve } from "node:path";

const REPO = resolve(import.meta.dirname, "..");
const require = createRequire(import.meta.url);

/** The suite that runs unattended: a hard-coded evidence path here is a bug. */
const SUITE_DIRS = ["e2e/flows", "e2e/helpers"];
/** Deliberate, lane-owned capture scripts: reported, not failed. */
const SCRIPT_DIR = "e2e";
const ALLOW = [
  // "e2e/flows/xx.spec.ts — reason it must name a foreign evidence dir"
];

/** Any of these in executable code means the path is frozen to one lane. */
const FROZEN = [/design\/qa-/, /["'`]qa-p\d/, /["'`]qa-e2e-/, /design["'`],\s*["'`]qa-/];

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...walk(full));
    else if (/\.(ts|mjs|cjs)$/.test(entry)) out.push(full);
  }
  return out;
}

/** Drop comments so a documented path (like this file's own header) is not a hit. */
function stripComments(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, (block) => block.replace(/[^\n]/g, " "))
    .replace(/^[ \t]*\/\/.*$/gm, "");
}

function scan(files) {
  const hits = [];
  for (const file of files) {
    const rel = relative(REPO, file);
    if (ALLOW.some((a) => a.startsWith(`${rel} `))) continue;
    const lines = stripComments(readFileSync(file, "utf8")).split("\n");
    lines.forEach((line, index) => {
      if (FROZEN.some((pattern) => pattern.test(line))) {
        hits.push({ file: rel, line: index + 1, text: line.trim().slice(0, 120) });
      }
    });
  }
  return hits;
}

function selfTest() {
  const { resolveEvidenceDir, sanitizeEvidenceName } = require("../e2e/helpers/evidence-core.cjs");
  const repo = "/repo";
  const cases = [
    [
      "explicit override wins (absolute)",
      resolveEvidenceDir({ override: "/tmp/ev", branch: "p32-x" }, repo),
      "/tmp/ev",
    ],
    [
      "explicit override, repo-relative",
      resolveEvidenceDir({ override: "design/qa-custom", branch: "p32-x" }, repo),
      "/repo/design/qa-custom",
    ],
    [
      "branch names the lane's own evidence dir",
      resolveEvidenceDir({ branch: "p32-today-day-pack" }, repo),
      "/repo/design/qa-p32-today-day-pack",
    ],
    [
      "detached HEAD falls back to the run id",
      resolveEvidenceDir({ branch: "HEAD", runId: "e2e-1792", sha: "abc1234" }, repo),
      "/repo/design/qa-e2e-1792",
    ],
    [
      "detached HEAD with no stack falls back to the short sha",
      resolveEvidenceDir({ branch: "HEAD", sha: "abc1234" }, repo),
      "/repo/design/qa-abc1234",
    ],
    [
      "slashes and spaces in a branch name cannot escape the prefix",
      resolveEvidenceDir({ branch: "feat/two words" }, repo),
      "/repo/design/qa-feat-two-words",
    ],
    ["empty name sanitizes to a usable dir", sanitizeEvidenceName("///"), "unattached"],
  ];
  let failed = 0;
  for (const [label, got, want] of cases) {
    if (got !== want) {
      console.error(`  FAIL  ${label}: got ${got}, want ${want}`);
      failed += 1;
    }
  }
  if (failed > 0) {
    console.error(`evidence-dir self-test: ${failed} of ${cases.length} failed`);
    return 1;
  }
  console.log(`evidence-dir self-test: ${cases.length} checks pass (no browser, no stack)`);
  return 0;
}

if (process.argv.includes("--self-test")) {
  process.exit(selfTest());
}

const suiteFiles = SUITE_DIRS.flatMap((dir) => walk(resolve(REPO, dir)));
const scriptFiles = readdirSync(resolve(REPO, SCRIPT_DIR))
  .filter((f) => f.endsWith(".mjs"))
  .map((f) => resolve(REPO, SCRIPT_DIR, f));

const failures = scan(suiteFiles);
const warnings = scan(scriptFiles);

console.log(
  `evidence-writes: ${suiteFiles.length} suite files, ${scriptFiles.length} capture scripts, ` +
    `${failures.length} frozen path(s) in the suite, ${warnings.length} in lane-owned scripts.`,
);

if (warnings.length > 0) {
  const verbose = process.argv.includes("--all");
  const shown = verbose ? warnings : warnings.slice(0, 5);
  console.log(
    "\nLane-owned capture scripts still naming a frozen path (deliberate lane tools — adopt " +
      "evidenceDir when you next touch them; `--all` lists every one):",
  );
  for (const w of shown) console.log(`  warn  ${w.file}:${w.line}  ${w.text}`);
  if (!verbose && warnings.length > shown.length) {
    console.log(`  … and ${warnings.length - shown.length} more`);
  }
}

if (failures.length > 0) {
  console.error(`\n${failures.length} frozen evidence path(s) in the committed suite:`);
  for (const f of failures) console.error(`  ${f.file}:${f.line}  ${f.text}`);
  console.error(
    "\nResolve the directory through `evidenceDir()` / `evidencePath()` from " +
      "e2e/helpers/evidence.ts — it writes design/qa-<this lane>/, and the lane that " +
      "owns a directory still lands in it because the branch name is the directory name.",
  );
  process.exit(1);
}
console.log("Clean — every suite write resolves the running lane's own evidence dir.");
