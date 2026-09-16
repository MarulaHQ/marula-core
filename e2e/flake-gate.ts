// Flake-budget enforcement (TASKLIST lane 8): reads the JSON report produced
// by the run and fails the job when more tests than the budget needed a
// retry. A run may absorb a handful of flakes; beyond that the gate goes red
// so flakiness gets fixed instead of silently retried forever.
//
// Run after the Playwright suite (`pnpm e2e` chains it; CI runs it as its
// own step) — NOT as globalTeardown, which executes before the reporter
// flushes the report.
import fs from "node:fs";
import { FLAKE_BUDGET, MAX_RETRIES, REPORT_FILE } from "./helpers/budget.ts";

interface ReportCase {
  title: string;
  results?: { status: string }[];
  expectedStatus?: string;
  tests?: { results?: { status: string }[] }[];
}

interface ReportSpec {
  ok?: boolean;
  stats: { expected?: number; unexpected?: number; flaky?: number };
  suites: ReportSuite[];
}

interface ReportSuite {
  title?: string;
  specs?: ReportCase[];
  suites?: ReportSuite[];
}

/** Walk the nested suite tree and yield every spec with its project title. */
function* walkSpecs(suites: ReportSuite[], proj = ""): Generator<ReportCase & { project: string }> {
  for (const s of suites) {
    const p = s.title || proj;
    for (const spec of s.specs ?? []) yield { ...spec, project: p };
    yield* walkSpecs(s.suites ?? [], p);
  }
}

function flakeGate() {
  if (!fs.existsSync(REPORT_FILE)) {
    console.error(`[flake-gate] no report at ${REPORT_FILE} — run did not produce a report`);
    process.exit(1);
  }
  const report = JSON.parse(fs.readFileSync(REPORT_FILE, "utf8")) as ReportSpec;
  const unexpected = report.stats.unexpected ?? 0;
  if (unexpected > 0 || report.ok === false) {
    console.error(
      `[flake-gate] run had ${unexpected} failures — gate already red via exit code`,
    );
    process.exit(1);
  }
  const all = [...walkSpecs(report.suites ?? [])];
  const flaky = all
    .map((spec) => {
      // In the JSON report a spec carries `tests[]` (one entry per project);
      // each entry's `results[]` are the attempts (retries appended).
      const runs = (spec.tests ?? [{ results: spec.results }]).map((t) => ({
        status: t.results?.at(-1)?.status ?? "unknown",
        attempts: t.results?.length ?? 0,
      }));
      return { title: spec.title, project: spec.project, runs };
    })
    .filter((t) => t.runs.some((r) => r.status === "passed" && r.attempts > 1));
  const worstRetry = Math.max(0, ...flaky.flatMap((t) => t.runs.map((r) => r.attempts - 1)));
  console.log(
    `[flake-gate] ${all.length} tests, ${report.stats.flaky ?? flaky.length} flaky (budget ${FLAKE_BUDGET}), max retries used ${worstRetry}/${MAX_RETRIES}, ${unexpected} failures`,
  );
  for (const t of flaky) console.log(`  flaky: ${t.project} — ${t.title}`);
  if (all.length === 0) {
    console.error("[flake-gate] FAIL — report parsed but contains no tests");
    process.exit(1);
  }
  if ((report.stats.flaky ?? flaky.length) > FLAKE_BUDGET) {
    console.error(
      `[flake-gate] FAIL — ${flaky.length} retried tests exceed the flake budget of ${FLAKE_BUDGET}`,
    );
    process.exit(1);
  }
  console.log("[flake-gate] PASS");
}

flakeGate();
