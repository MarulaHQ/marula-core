// Types for the bundle budget gate (scripts/check-web-bundle-budget.mjs).
//
// The gate is a plain Node script so it runs before any build tooling exists;
// this declaration is what lets `apps/web/tests/perf/bundle-budget.test.ts`
// import it under `tsc --noEmit` without turning the whole module into `any`.
export const DEFAULT_TOLERANCE_BYTES: number;
export const DEFAULT_HEADROOM_PCT: number;
export const DEFAULT_MIN_HEADROOM_BYTES: number;

export interface RouteSize {
  raw: number;
  gzip: number;
}

export interface BudgetRow {
  route: string;
  gzip: number | null;
  baseline: number | null;
  ceiling: number | null;
  verdict: string;
}

export interface BudgetFailure {
  kind: "ceiling" | "regression" | "group-ceiling" | string;
  route: string;
  detail: string;
}

export interface BudgetResult {
  ok: boolean;
  failures: BudgetFailure[];
  warnings: string[];
  rows: BudgetRow[];
}

export function computeRouteSizes(appDir: string): Map<string, RouteSize>;

export function derivedCeiling(baselineBytes: number, config: Record<string, unknown>): number;

export function evaluateBudget(
  sizes: Map<string, RouteSize>,
  config: Record<string, unknown>,
  options?: { toleranceBytes?: number },
): BudgetResult;

export function formatReport(result: Pick<BudgetResult, "rows">): string;
