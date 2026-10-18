#!/usr/bin/env node
/**
 * check-hook-paths.mjs — a tracked agent config must not name one machine's checkout.
 *
 * Wave 47 lane A18 (`p47-tracker-freshness-guard`), item (C).  `.cursor/hooks.json`
 * was tracked with the root checkout's absolute path baked into every command
 * (`/Users/<name>/Documents/GitHub/.../.cursor/hooks/trail-hooks.cjs`).  Trail's
 * wiring rewrites the file per checkout, so every worktree lane started with a
 * dirty tree holding a path that only exists on this laptop.  Two lanes already
 * had to exclude it by hand: `p43-api-payload-gaps` recorded the exclusion in its
 * landing note, and this lane found it dirty again on a fresh worktree.
 *
 * The file is untracked now (see `.gitignore`), because the thing that writes it
 * writes absolute paths.  This check is the belt: any tracked agent config that
 * reintroduces an absolute path fails before it can be committed.
 *
 *   node scripts/check-hook-paths.mjs              # gate
 *   node scripts/check-hook-paths.mjs --self-test  # the detector's own rules
 *
 * Exit 0 = clean · 1 = a tracked config carries an absolute path or a missing hook.
 */
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const REPO = resolve(import.meta.dirname, "..");

/** Config files an agent host reads. Only tracked ones are scanned. */
const CANDIDATES = [
  ".cursor/hooks.json",
  ".cursor/mcp.json",
  ".claude/settings.json",
  ".claude/settings.local.json",
  ".mcp.json",
  "opencode.json",
  ".vscode/settings.json",
];

const ABSOLUTE = /(^|[\s"'=])(?:[A-Za-z]:[\\/]|\/(?:Users|home|root|opt|private|tmp|var|Volumes)\/)/;

/** A command that points at a script in this repo must resolve relative to it. */
const REPO_SCRIPT = /(?:^|[\s"'])(\.?\/)?((?:\.?cursor|\.?claude|scripts|\.?kilo)\/[\w./-]+\.(?:cjs|mjs|js|sh|py))(?:[\s"']|$)/;

export function findAbsolutePaths(value, path = []) {
  const hits = [];
  if (typeof value === "string") {
    if (ABSOLUTE.test(value)) hits.push({ path: path.join("."), value });
  } else if (Array.isArray(value)) {
    value.forEach((v, i) => hits.push(...findAbsolutePaths(v, [...path, String(i)])));
  } else if (value && typeof value === "object") {
    for (const [k, v] of Object.entries(value)) hits.push(...findAbsolutePaths(v, [...path, k]));
  }
  return hits;
}

export function repoScripts(value) {
  const out = [];
  const visit = (v) => {
    if (typeof v === "string") {
      for (const m of v.matchAll(new RegExp(REPO_SCRIPT.source, "g"))) out.push(m[2]);
    } else if (Array.isArray(v)) v.forEach(visit);
    else if (v && typeof v === "object") Object.values(v).forEach(visit);
  };
  visit(value);
  return out;
}

function tracked(file) {
  try {
    execFileSync("git", ["ls-files", "--error-unmatch", file], { cwd: REPO, stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function selfTest() {
  const failures = [];
  const eq = (name, actual, expected) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      failures.push(`${name}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
    }
  };
  eq(
    "absolute command",
    findAbsolutePaths({ command: 'node "/Users/x/repo/.cursor/hooks/h.cjs" cursor' }).length,
    1,
  );
  eq("relative command", findAbsolutePaths({ command: 'node ".cursor/hooks/h.cjs" cursor' }).length, 0);
  eq("bare binary", findAbsolutePaths({ command: "trail", args: ["mcp"] }).length, 0);
  eq(
    "relative script extraction",
    repoScripts({ command: 'node ".cursor/hooks/trail-hooks.cjs" cursor-post-tool' }),
    [".cursor/hooks/trail-hooks.cjs"],
  );
  eq("windows path", findAbsolutePaths({ cwd: "C:\\\\Users\\\\x" }).length, 1);
  if (failures.length) {
    for (const f of failures) console.error(`SELFTEST FAIL ${f}`);
    process.exit(1);
  }
  console.log("self-test ok — 5 detector rules");
}

function main() {
  if (process.argv.includes("--self-test")) return selfTest();

  const problems = [];
  for (const file of CANDIDATES) {
    if (!tracked(file)) continue;
    let parsed;
    try {
      parsed = JSON.parse(readFileSync(join(REPO, file), "utf8"));
    } catch (error) {
      problems.push(`${file}: cannot parse (${error.message})`);
      continue;
    }
    for (const hit of findAbsolutePaths(parsed)) {
      problems.push(`${file} ${hit.path}: absolute path — ${hit.value.slice(0, 120)}`);
    }
    for (const script of repoScripts(parsed)) {
      if (!existsSync(join(REPO, script))) {
        problems.push(`${file}: points at ${script}, which is not in this checkout`);
      }
    }
  }

  if (problems.length) {
    console.error(`Agent config hygiene — ${problems.length} problem${problems.length === 1 ? "" : "s"}:`);
    for (const p of problems) console.error(`  ${p}`);
    console.error(
      "\nUse a path relative to the checkout root, or leave the generated file untracked\n" +
        "(trail re-writes it per worktree). See .gitignore.",
    );
    process.exit(1);
  }
  console.log("Agent config hygiene — no absolute paths in tracked hook configs.");
}

main();
