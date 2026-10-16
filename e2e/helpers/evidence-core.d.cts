// Type declarations for evidence-core.cjs (the pure rule lives in CJS so plain
// node can test it; see evidence.ts). Keeps an editor or a `tsc` run from
// reporting an implicit `any` for the import.

/** Directory prefix every lane evidence dir shares: design/qa-<lane>. */
export const EVIDENCE_PREFIX: string;

/** A branch name, run id or sha made safe to use as a directory name. */
export function sanitizeEvidenceName(value: unknown): string;

export interface EvidenceContext {
  /** Explicit override (absolute, or relative to the repo root). */
  override?: string;
  /** `git rev-parse --abbrev-ref HEAD`; "HEAD" means detached. */
  branch?: string;
  /** Short HEAD sha, for a detached run with no run id. */
  sha?: string;
  /** Run id from e2e/.e2e-stack.env, if a stack is up. */
  runId?: string;
}

/** Predict the evidence directory (absolute); creates nothing. */
export function resolveEvidenceDir(ctx: EvidenceContext, repo: string): string;
