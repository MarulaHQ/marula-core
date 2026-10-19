#!/usr/bin/env node
/**
 * p47-rendered-states-audit — the lane's one entry point.
 *
 * Wires the two halves of the rendered-state audit together:
 *
 *   1. `scripts/p47-row-effect-check.mjs` — the mechanical detector for the
 *      "row treatment authored as a cell treatment" class. Static, no stack.
 *   2. `scripts/p47-rendered-states-harness.mjs` — the browser instrument that
 *      drives the state modifiers and records computed values. Needs the stack.
 *
 * Usage:
 *   node scripts/p47-rendered-states-audit.mjs            # static only
 *   node scripts/p47-rendered-states-audit.mjs --browser  # static + browser
 *   node scripts/p47-rendered-states-audit.mjs --browser --shots
 */

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const OUT = resolve(ROOT, "design/qa-p47-rendered-states");
mkdirSync(OUT, { recursive: true });

function run(script, args = []) {
  const started = Date.now();
  const r = spawnSync("node", [resolve(ROOT, "scripts", script), ...args], {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  return { code: r.status, out: r.stdout ?? "", err: r.stderr ?? "", ms: Date.now() - started };
}

const lines = [];
const startedAt = new Date().toISOString();

// 1. static detector — must produce the proving case.
const det = run("p47-row-effect-check.mjs", [
  "--json",
  resolve(OUT, "ROWS.json"),
]);
if (det.code !== 0) {
  console.error(`row-effect-check exited ${det.code}\n${det.err}`);
}
lines.push(det.out.trim());
const provingOk = det.out.includes("proving case apps/web/components/money.css:228-238 present: YES");

// 2. browser instrument — only when asked, only when the stack env exists.
let harness = null;
if (process.argv.includes("--browser")) {
  const args = [];
  if (process.argv.includes("--shots")) args.push("--shots");
  const h = run("p47-rendered-states-harness.mjs", args);
  if (h.code !== 0) console.error(`harness exited ${h.code}\n${h.err}`);
  harness = { exit: h.code, stdout: h.out, stderr: h.err, ms: h.ms };
  lines.push(h.out.trim());
}

const summary = {
  startedAt,
  static: { exit: det.code, provingCasePresent: provingOk },
  browser: harness ? { exit: harness.exit, ms: harness.ms } : "not run (pass --browser with the stack up)",
  console: lines,
};
writeFileSync(resolve(OUT, "AUDIT-RUN.json"), JSON.stringify(summary, null, 2) + "\n");

console.log(lines.join("\n"));
console.log("");
if (!provingOk) {
  console.error("FAIL: the detector did not reproduce apps/web/components/money.css:228-238.");
  process.exit(2);
}
console.log("audit runner complete.");
