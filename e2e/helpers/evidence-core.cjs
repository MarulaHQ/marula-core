// evidence-core.cjs — the pure rule for "which evidence dir may this run write?"
//
// Wave 47 lane A17.  Plain CommonJS on purpose: it is imported by
// e2e/helpers/evidence.ts (through Playwright's transpiler) AND by
// scripts/check-evidence-writes.mjs --self-test (plain node), so the rule has
// exactly one implementation and a test that does not need a browser or a
// stack.  No filesystem access here — see evidence.ts for the side effects.
//
// Keep this rule in step with the comment at the top of evidence.ts.

/** Directory prefix every lane evidence dir shares: design/qa-<lane>. */
// The root is `design/`, a local and git-ignored scratch tree (`.gitignore`
// ignores `/design/`). The private design corpus used to live at that path and
// now lives in the private sibling repository, but capture output still lands
// here: it is scratch that a run rewrites, not a record this repository ships.
const EVIDENCE_PREFIX = "qa-";

/** A branch name / run id / sha is usable as a directory name only if safe. */
function sanitizeEvidenceName(value) {
  const cleaned = String(value ?? "")
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/^[.-]+|[.-]+$/g, "");
  return cleaned.length > 0 ? cleaned : "unattached";
}

/**
 * Predict the evidence directory (absolute), creating nothing.
 *
 *   1. override            E2E_EVIDENCE_DIR, absolute or repo-relative
 *   2. branch              design/qa-<branch>  — the repo's own convention, so
 *                          the lane that owns an evidence dir keeps landing in it
 *   3. runId               design/qa-<run id>  — detached HEAD with a live stack
 *   4. sha                 design/qa-<short sha> — detached HEAD, nothing else
 *
 * A detached HEAD never falls back to a name that another lane owns: the whole
 * point is that no two runs share a directory by accident.
 */
function resolveEvidenceDir(ctx, repo) {
  const context = ctx || {};
  const root = repo;
  if (context.override && String(context.override).length > 0) {
    const value = String(context.override);
    return value.startsWith("/") ? value : joinPath(root, value);
  }
  const branch = String(context.branch ?? "").trim();
  if (branch.length > 0 && branch !== "HEAD") {
    return joinPath(root, "design", EVIDENCE_PREFIX + sanitizeEvidenceName(branch));
  }
  const runId = String(context.runId ?? "").trim();
  if (runId.length > 0) {
    return joinPath(root, "design", EVIDENCE_PREFIX + sanitizeEvidenceName(runId));
  }
  return joinPath(root, "design", EVIDENCE_PREFIX + sanitizeEvidenceName(context.sha ?? "unknown-sha"));
}

/** path.join without importing node:path (keeps this file dependency-free). */
function joinPath(...parts) {
  return parts
    .filter((p) => String(p).length > 0)
    .map((p, i) => (i === 0 ? String(p).replace(/\/+$/, "") : String(p).replace(/^\/+|\/+$/g, "")))
    .join("/");
}

module.exports = { EVIDENCE_PREFIX, resolveEvidenceDir, sanitizeEvidenceName };
