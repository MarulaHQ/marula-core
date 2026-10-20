// evidence.ts — where a Playwright run is allowed to write evidence.
//
// Wave 47 lane A17 (`p47-lane-env-hygiene`).  Why this exists: specs used to
// hard-code another lane's committed evidence directory, so any lane running
// `pnpm e2e` rewrote the owner's record — a sweep on 2026-10-16 modified seven
// tracked files belonging to three other lanes (`design/qa-p29-accessibility-deep/
// axe-results.json`, four captures under `design/qa-p32-today-day-pack/`, two
// under `design/qa-p34-plugin-surface-closeout/`).  A lane that gated with
// `git add -A` would have committed that churn over three lanes' evidence.
//
// The rule is one line: the running lane writes into ITS OWN evidence dir.
//
//   1. E2E_EVIDENCE_DIR      — explicit override (absolute or repo-relative)
//   2. design/qa-<branch>    — the lane's branch, which IS the repo's existing
//                              evidence-dir convention (`design/qa-<lane>/`), so
//                              the lane that owns a directory still lands in it
//   3. design/qa-<run id>    — detached HEAD with a stack from e2e/.e2e-stack.env
//   4. design/qa-<short sha> — detached HEAD, nothing else
//
// The root `design/` is local and git-ignored: `.gitignore` ignores `/design/`
// because that path held the private design corpus before it moved out. Capture
// output still lands there, because it is scratch a run rewrites and never a
// file this repository ships.
//
// The pure rule lives in evidence-core.cjs (one implementation, tested by
// `node scripts/check-evidence-writes.mjs --self-test` without a browser or a
// stack).  This module adds the git/run-id lookup and creates the directory.
//
// `scripts/check-evidence-writes.mjs` fails the gate if a spec or helper in the
// committed suite carries a hard-coded design/qa-* path again.

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import * as path from "node:path";
// eslint-disable-next-line @typescript-eslint/no-var-requires
import { resolveEvidenceDir } from "./evidence-core.cjs";

const REPO = path.resolve(__dirname, "..", "..");

function git(args: string[]): string {
  try {
    return execFileSync("git", ["-C", REPO, ...args], { encoding: "utf8" }).trim();
  } catch {
    return "";
  }
}

function runIdFromStackEnv(): string {
  try {
    const text = readFileSync(path.join(REPO, "e2e", ".e2e-stack.env"), "utf8");
    const match = text.match(/^MARULA_RUN_ID=(.*)$/m);
    return match ? match[1].trim() : "";
  } catch {
    return "";
  }
}

/**
 * The lane's evidence directory, created on demand.  `sub` is a path below it
 * (e.g. "captures").  Call this instead of writing a literal design/qa-* path.
 */
export function evidenceDir(sub?: string): string {
  const root = resolveEvidenceDir(
    {
      override: process.env.E2E_EVIDENCE_DIR,
      branch: git(["rev-parse", "--abbrev-ref", "HEAD"]),
      sha: git(["rev-parse", "--short", "HEAD"]),
      runId: runIdFromStackEnv(),
    },
    REPO,
  );
  const full = sub ? path.join(root, sub) : root;
  mkdirSync(full, { recursive: true });
  return full;
}

/** Convenience: a path inside the lane's evidence dir. */
export function evidencePath(...segments: string[]): string {
  return path.join(evidenceDir(), ...segments);
}

/** The pure part, re-exported so specs can assert the rule without disk writes. */
export { resolveEvidenceDir };
