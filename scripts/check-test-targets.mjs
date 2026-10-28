#!/usr/bin/env node
// check-test-targets.mjs — one source of truth for the integration targets
// under crates/core-api/tests, and a guard against the failure the
// `test-binaries` pilot found.
//
// Cargo has two target shapes in that directory: `tests/<name>.rs` is one
// target named `<name>`, and `tests/<dir>/main.rs` is one target named `<dir>`.
// CI enumerated them with `ls *.rs`, so a directory target was in neither shard:
// built by nothing, run by nobody, and no red build. The pilot's own commit
// fixed that glob; this file makes the class of bug impossible by owning the
// enumeration that CI consumes.
//
// Usage:
//   node scripts/check-test-targets.mjs                 verify the tree
//   node scripts/check-test-targets.mjs --list          print target names
//   node scripts/check-test-targets.mjs --shard 1 --shards 2
//                                                       print one shard's targets
//   node scripts/check-test-targets.mjs --self-test     prove the checks
//
// Verify fails when:
//   - two targets resolve to the same cargo target name;
//   - a `.rs` file under tests/ is neither a target nor reachable from a
//     `mod`/`#[path]` declaration, so nothing builds it;
//   - a directory holds a `main.rs` but the directory name would collide.

import { existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename as path_basename, dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const LABEL = "check-test-targets";

/** Target names cargo will build from a `tests/` directory, sorted. */
export function discoverTargets(testsDir) {
  const names = [];
  for (const entry of readdirSync(testsDir, { withFileTypes: true })) {
    if (entry.isFile() && entry.name.endsWith(".rs")) {
      names.push(entry.name.slice(0, -".rs".length));
    } else if (entry.isDirectory() && existsSync(join(testsDir, entry.name, "main.rs"))) {
      names.push(entry.name);
    }
  }
  return names.sort();
}

/** True when `stem` is declared somewhere in `dir` by `mod` or `#[path]`. */
function isDeclared(dir, stem, seen = new Set()) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (seen.has(path)) continue;
      seen.add(path);
      if (isDeclared(path, stem, seen)) return true;
      continue;
    }
    if (!entry.name.endsWith(".rs")) continue;
    const body = readFileSync(path, "utf8");
    if (new RegExp(`\\bmod\\s+${stem}\\b`).test(body)) return true;
    if (new RegExp(`#\\[path\\s*=\\s*"[^"]*${stem}\\.rs"\\]`).test(body)) return true;
  }
  return false;
}

/** Everything wrong with the tree, as readable lines. */
export function verifyTree(testsDir) {
  const problems = [];
  const targets = discoverTargets(testsDir);

  const seen = new Map();
  for (const name of targets) {
    if (seen.has(name)) problems.push(`duplicate target name: ${name}`);
    seen.set(name, true);
  }

  for (const entry of readdirSync(testsDir, { withFileTypes: true })) {
    if (entry.isFile() && entry.name.endsWith(".rs")) continue;
    if (!entry.isDirectory()) continue;
    const dir = join(testsDir, entry.name);
    const hasMain = existsSync(join(dir, "main.rs"));
    const walk = (current) => {
      for (const child of readdirSync(current, { withFileTypes: true })) {
        const path = join(current, child.name);
        if (child.isDirectory()) {
          walk(path);
          continue;
        }
        if (!child.name.endsWith(".rs")) continue;
        const stem = child.name.slice(0, -".rs".length);
        if (hasMain && current === dir && stem === "main") continue;
        // A directory module root (`mod.rs`) is declared by its directory name,
        // usually through `#[path = ".../mod.rs"] mod <dir>`; every other file
        // is declared by its own stem.
        const declared = stem === "mod" ? path_basename(current) : stem;
        const reachable = stem === "mod"
          ? isDeclared(testsDir, declared)
          : isDeclared(dir, declared) || (current !== dir && isDeclared(current, declared));
        if (!reachable) {
          problems.push(
            `unreachable: ${path} — no mod declaration builds it, so no shard runs it`,
          );
        }
      }
    };
    walk(dir);
  }
  return { targets, problems };
}

/** Round-robin shard selection, identical to the CI split. */
export function shardOf(targets, shard, shards) {
  return targets.filter((_, index) => index % shards === shard - 1);
}

function selfTest() {
  const root = mkdtempSync(join(tmpdir(), "test-targets-"));
  const tests = join(root, "tests");
  mkdirSync(join(tests, "group"), { recursive: true });
  mkdirSync(join(tests, "support"), { recursive: true });

  writeFileSync(
    join(tests, "solo_flow.rs"),
    '#[path = "support/mod.rs"]\nmod support;\n#[test]\nfn t() {}\n',
  );
  writeFileSync(join(tests, "group", "main.rs"), "mod common;\nmod a_flow;\n");
  writeFileSync(join(tests, "group", "common.rs"), "pub fn harness() {}\n");
  writeFileSync(join(tests, "group", "a_flow.rs"), "#[test]\nfn t() {}\n");
  writeFileSync(join(tests, "support", "helper.rs"), "pub fn h() {}\n");
  writeFileSync(join(tests, "support", "mod.rs"), "mod helper;\n");

  const good = verifyTree(tests);
  if (good.problems.length) throw new Error(`self-test: valid tree reported ${good.problems}`);
  if (good.targets.join(",") !== "group,solo_flow") {
    throw new Error(`self-test: discovery returned ${good.targets.join(",")}`);
  }
  const shards = [shardOf(good.targets, 1, 2), shardOf(good.targets, 2, 2)];
  if (shards.flat().sort().join(",") !== "group,solo_flow") {
    throw new Error(`self-test: sharding lost a target: ${JSON.stringify(shards)}`);
  }

  writeFileSync(join(tests, "orphan_flow.rs.bak"), "");
  writeFileSync(join(tests, "orphan.rs"), "#[test]\nfn t() {}\n");
  // An orphan is only an orphan when it is not a target; a top-level .rs IS a
  // target, so move it into the group where nothing declares it.
  rmSync(join(tests, "orphan.rs"));
  writeFileSync(join(tests, "group", "orphan_flow.rs"), "#[test]\nfn t() {}\n");
  const bad = verifyTree(tests);
  if (!bad.problems.some((p) => p.includes("orphan_flow.rs"))) {
    throw new Error("self-test: unreachable module was not reported");
  }

  rmSync(root, { recursive: true, force: true });
  console.log(`${LABEL}: self-test OK — discovery, sharding and the orphan check`);
}

function main(argv) {
  if (argv.includes("--self-test")) {
    selfTest();
    return 0;
  }
  const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
  const tests = join(repo, "crates", "core-api", "tests");
  const { targets, problems } = verifyTree(tests);

  if (argv.includes("--list")) {
    for (const name of targets) console.log(name);
    return 0;
  }
  const shardAt = argv.indexOf("--shard");
  if (shardAt !== -1) {
    const shard = Number(argv[shardAt + 1]);
    const shardsAt = argv.indexOf("--shards");
    const shards = shardsAt === -1 ? 2 : Number(argv[shardsAt + 1]);
    if (!Number.isInteger(shard) || shard < 1 || shard > shards) {
      console.error(`${LABEL}: --shard must be 1..${shards}`);
      return 2;
    }
    for (const name of shardOf(targets, shard, shards)) console.log(name);
    return 0;
  }

  if (problems.length) {
    for (const problem of problems) console.error(`${LABEL}: ${problem}`);
    console.error(`${LABEL}: ${problems.length} problem(s)`);
    return 1;
  }
  console.log(`${LABEL}: ${targets.length} targets, unique names, every file reachable`);
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = main(process.argv.slice(2));
}
