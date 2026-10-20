// Flake budget (the private build tracker lane 8): a CI run is allowed at most MAX_RETRIES
// automatic retries per test and at most FLAKE_BUDGET total retried tests
// before the run is marked unstable and fails the merge gate. Enforced by
// the global-teardown script reading the Playwright JSON report.
export const MAX_RETRIES = 2;
/** Whole-run budget: if MORE than this many tests needed a retry, the gate fails. */
export const FLAKE_BUDGET = 4;
export const REPORT_FILE = "e2e/test-results/report.json"; // flake-gate runs with cwd = repo root
