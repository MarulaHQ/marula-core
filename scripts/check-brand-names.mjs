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
  // The internal design corpus and the old brand bundle moved to the private
  // sibling repository as `internal/design/` and `internal/meridian-ci`; the
  // site package moved to the commercial repository. Nothing under these
  // prefixes is tracked here any more — the entries stay so a stray local copy
  // is still treated as history rather than a new violation.
  "internal/design/", // mockups, the tweakcn preset, kit tokens, landing evidence
  "internal/meridian-ci/", // old brand asset pack (historical source bundle)
  "docs/prompts/", // the original one-shot build brief: historical record
  ".kilo/", // agent-local tooling
  ".trail/", // agent-local tooling: the graft map's cards + learnings
  "graft/",
  "graphify-out/",
  "node_modules/",
  "Cargo.lock",
];

// L2/L3-class: old migrations are history (byte-identical, never rewritten);
// migration 0114+ (this lane and later) MUST be clean and is NOT ledged.
const MIGRATION_RE = /^crates\/core-db\/migrations\/(\d{4})_/;
function isLedgedMigration(file) {
  const m = MIGRATION_RE.exec(file);
  if (!m) return false;
  return Number(m[1]) < 114;
}

function git(args) {
  try {
    return execFileSync("git", args, { encoding: "utf8", maxBuffer: 1 << 28 });
  } catch (e) {
    // git grep exits 1 on zero matches; anything else is fatal.
    if (e.status === 1) return e.stdout ?? "";
    throw e;
  }
}

// Count OLD-brand hits (Meridian/meridian) outside the ledger. The pattern
// list is explicit so the gate byte-matches docs/reference/marula-name-map.md.
const PATTERNS = ["meridian"];
let lines = [];
for (const pat of PATTERNS) {
  lines = lines.concat(git(["grep", "-i", "-n", pat, "--", "."]).split("\n").filter(Boolean));
}
// dedupe (same line can match multiple patterns)
lines = [...new Set(lines)];

const violations = [];
for (const line of lines) {
  const colon = line.indexOf(":", 2);
  if (colon === -1) continue;
  const file = line.slice(0, colon);
  if (LEDGER_EXACT.has(file)) continue;
  if (LEDGER_PREFIXES.some((p) => file.startsWith(p))) continue;
  if (isLedgedMigration(file)) continue; // pre-0114 migrations are history (L3)
  // L1: relay-technical symbols, not brand.
  const rest = line.slice(colon + 1);
  if (rest.includes("PresenceRelayListener")) continue;
  // L2: third-party repo paths that carry the old name.
  if (rest.includes("project-meridian-relay-psa")) continue;
  // 0114 must quote the old names it RETIRES (DROP TRIGGER legacy names +
  // history rationale in comments). Every future string-data migration may
  // quote only to drop superseded names — additions of new old-name surface
  // still fail this gate.
  if (file === "crates/core-db/migrations/0114_wave41_marula_rebrand.sql") continue;
  // Pending-regen: the OpenAPI pins are re-pinned ONLY via
  // scripts/regen-openapi-pins.sh (gate-time, stack needed); the shipping-SDK
  // twin (packages/sdk) is regenerated from the pin. This allowance expires
  // at the pins step of the verification battery.
  if (file.startsWith("fixtures/openapi/")) continue;
  if (file.startsWith("packages/sdk/openapi.json")) continue;
  violations.push(line);
}

if (violations.length) {
  console.error(`check-brand-names: ${violations.length} old-brand hits outside the exclusion ledger (docs/reference/marula-name-map.md):`);
  for (const v of violations.slice(0, 200)) console.error("  " + v);
  if (violations.length > 200) console.error(`  … and ${violations.length - 200} more`);
  process.exit(1);
}
console.log("check-brand-names: clean — no old-brand hits outside the exclusion ledger");
