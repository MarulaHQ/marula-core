#!/usr/bin/env node
/**
 * check-brand-names.mjs — Wave 41 lane 1 brand-NAMES gate.
 *
 * Enforces docs/reference/marula-name-map.md: after the mechanical pass, the
 * only `Marula`/`marula` hits in tracked files are the exclusion ledger
 * (L1–L5 there) and the checker's ownself. New hits outside the ledger FAIL.
 *
 * Exit 0 = clean · exit 1 = violations (they print file:line).
 */
import { execFileSync } from "node:child_process";

// Paths where old-name hits are ALWAYS ledger (prefix / exact match).
const LEDGER_EXACT = new Set([
  "TASKLIST_COMPLETED.md", // L3 historical record
  "AGENTS.md", // L4 lane-history sections
  "TASKLIST.md", // lane row quotes the old scope verbatim
  "docs/reference/wave41-marula-plan.md", // L5 quotes the old state
  "docs/reference/marula-name-map.md", // the map quotes old names
  "scripts/check-brand-names.mjs", // ownself
  "CHANGELOG.md", // historical release record (L3-class)
  "ACKNOWLEDGMENTS.md", // historical record
  "docs/BUILD-PROMPT-VERDICTS.md", // verdict excerpts quote shipped strings
  "docs/STAGE-REPORT.md", // historical stage record
  ".cargo/config.toml", // machine-local dev layout path (repo dir name; repo move is lane 17)
  ".cursor/hooks.json", // machine-local hook paths, same repo-dir reason
]);

const LEDGER_PREFIXES = [
  "design/qa-", // L4 landing records / reviews / captures
  "design/REPORT-", // historical design reports
  "design/mockups/", // marula-* mockup sources, historical
  "marula-ci/", // old brand asset pack (historical source bundle)
  "website/", // FROZEN per D10 — moves to Marula-HQ; mechanical only
  ".kilo/", // agent-local tooling
  "graft/",
  "graphify-out/",
  "node_modules/",
];

function git(args) {
  try {
    return execFileSync("git", args, { encoding: "utf8", maxBuffer: 1 << 28 });
  } catch (e) {
    // git grep exits 1 on zero matches
    return e.stdout ?? "";
  }
}

const lines = git(["grep", "-i", "-n", "marula", "--", "."]).split("\n").filter(Boolean);

const violations = [];
for (const line of lines) {
  const colon = line.indexOf(":", 2);
  if (colon === -1) continue;
  const file = line.slice(0, colon);
  if (LEDGER_EXACT.has(file)) continue;
  if (LEDGER_PREFIXES.some((p) => file.startsWith(p))) continue;
  // L1: relay-technical symbols, not brand.
  const rest = line.slice(colon + 1);
  if (rest.includes("PresenceRelayListener")) continue;
  violations.push(line);
}

if (violations.length) {
  console.error(`check-brand-names: ${violations.length} old-brand hits outside the exclusion ledger (docs/reference/marula-name-map.md):`);
  for (const v of violations.slice(0, 200)) console.error("  " + v);
  if (violations.length > 200) console.error(`  … and ${violations.length - 200} more`);
  process.exit(1);
}
console.log("check-brand-names: clean — no old-brand hits outside the exclusion ledger");
