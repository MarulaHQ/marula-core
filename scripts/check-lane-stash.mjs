#!/usr/bin/env node
/**
 * check-lane-stash.mjs — a lane may not park work in an anonymous stash.
 *
 * Wave 47 lane A24 (`p47-shared-stash-collision`). A git stash lives in one ref
 * in the common `.git`, so every worktree of this repository shares ONE stack:
 * on 2026-10-17 the stack carried 13 entries from other lanes, and when a
 * lane's path-limited `git stash push` created nothing, the `git stash pop`
 * that followed restored a foreign entry and conflicted on
 * `docs/site/reference/api.md`. Nothing was lost that time, by luck.
 *
 * Two things make that possible, and this check closes the second:
 *   1. the stack is shared by every worktree (nothing in git can change that);
 *   2. entries are anonymous, so `pop` means "top of the stack", not "mine".
 *
 * An entry is named when it was pushed with `-m "<lane> <purpose>"` — git then
 * stores `On <branch>: <message>` as its subject. Everything git generates on
 * its own starts `WIP on`, `index on` or `untracked files on`, and is a
 * failure: no lane can prove it owns it, and none can pop it safely.
 *
 *   node scripts/check-lane-stash.mjs              # gate: this checkout's stack
 *   node scripts/check-lane-stash.mjs --self-test  # the classifier, no stack needed
 *
 * The rule of record is in AGENTS.md ("Lanes never park work in an anonymous
 * stash"). Exit 0 = every entry is named (or the stack is empty) · 1 = at least
 * one anonymous entry.
 */
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const REPO = resolve(import.meta.dirname, "..");

/** Subjects git writes when the caller passed no message. */
const AUTO_GENERATED = /^(WIP on|index on|untracked files on) /;
/** `On <branch>: <message>` — the only shape a named entry has. */
const NAMED = /^On .+?: \S/;

export function classify(subject) {
  const text = String(subject ?? "").trim();
  if (text === "") return "anonymous"; // `-m ""` is still an unnamed entry
  if (AUTO_GENERATED.test(text)) return "anonymous";
  if (NAMED.test(text)) return "named";
  return "anonymous"; // an unknown shape cannot be shown to belong to a lane
}

function entries(cwd) {
  let raw = "";
  try {
    raw = execFileSync("git", ["stash", "list", "--format=%gd%x09%gs"], {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
  } catch {
    return [];
  }
  return raw
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => {
      const [ref = "", subject = ""] = line.split("\t");
      return { ref, subject, kind: classify(subject) };
    });
}

/** The gate itself: returns the anonymous entries in `cwd`'s stash stack. */
export function anonymousEntries(cwd) {
  return entries(cwd).filter((entry) => entry.kind === "anonymous");
}

function gate() {
  const all = entries(REPO);
  const anonymous = all.filter((entry) => entry.kind === "anonymous");
  if (all.length === 0) {
    console.log("lane-stash: the shared stack is empty — nothing can be popped by mistake.");
    return 0;
  }
  if (anonymous.length === 0) {
    console.log(`lane-stash: ${all.length} named entr${all.length === 1 ? "y" : "ies"}, all addressable.`);
    for (const entry of all) console.log(`  ok    ${entry.ref}  ${entry.subject}`);
    return 0;
  }
  console.error(
    `lane-stash: ${anonymous.length} of ${all.length} stash entr${
      all.length === 1 ? "y is" : "ies are"
    } anonymous — any lane's \`pop\` can take one of these.`,
  );
  for (const entry of anonymous) console.error(`  FAIL  ${entry.ref}  ${entry.subject}`);
  console.error(
    "\nA named stash is addressable: `git stash push -m \"<lane> <purpose>\"`, then " +
      "`git stash list` to find it and `git stash pop stash@{N}` to take THAT one. " +
      "For anything longer than a probe, park on the lane's own branch instead — a " +
      "branch cannot be grabbed by a peer.",
  );
  console.error(
    "\nTo retire one entry, read it first: `git stash show -p <ref>`; drop it only " +
      "after ruling that it is dead (`git stash drop <ref>`).",
  );
  return 1;
}

function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), "lane-stash-selftest-"));
  const git = (...args) => execFileSync("git", args, { cwd: dir, stdio: "ignore" });
  const checks = [];
  try {
    git("init", "-q");
    git("config", "user.email", "selftest@example.invalid");
    git("config", "user.name", "lane-stash self-test");
    writeFileSync(join(dir, "probe.txt"), "one\n");
    git("add", "probe.txt");
    git("commit", "-q", "-m", "base");

    checks.push(["empty stack passes", anonymousEntries(dir).length === 0, "expected no entries"]);

    writeFileSync(join(dir, "probe.txt"), "two\n");
    git("stash", "push", "-q"); // no -m: git generates the anonymous subject
    const bare = anonymousEntries(dir);
    checks.push([
      "an unnamed stash is anonymous",
      bare.length === 1 && classify(bare[0].subject) === "anonymous",
      `classified ${bare.length} entr${bare.length === 1 ? "y" : "ies"} as anonymous`,
    ]);

    git("stash", "push", "-q", "-m", "p47-hygiene scratch: probe");
    const mixed = anonymousEntries(dir);
    checks.push([
      "a named stash is not anonymous",
      mixed.length === 1,
      `expected the single unnamed entry to remain, saw ${mixed.length}`,
    ]);

    git("stash", "clear");
    git("stash", "push", "-q", "-m", "p47-hygiene scratch: probe");
    checks.push([
      "a stack of only named entries passes",
      anonymousEntries(dir).length === 0,
      "a named entry was classified as anonymous",
    ]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }

  const failed = checks.filter(([, ok]) => !ok);
  for (const [label, ok, detail] of checks) {
    if (!ok) console.error(`  FAIL  ${label}: ${detail}`);
  }
  if (failed.length > 0) {
    console.error(`lane-stash self-test: ${failed.length} of ${checks.length} failed`);
    return 1;
  }
  console.log(`lane-stash self-test: ${checks.length} checks pass (temporary repo, no stack in this checkout)`);
  return 0;
}

if (process.argv.includes("--self-test")) {
  process.exit(selfTest());
}

process.exit(gate());
