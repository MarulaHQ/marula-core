#!/usr/bin/env node
// Reliability sweep (Wave 52 lane R1).
//
// Purpose: fail the build when a retry path stops being governed. The point of
// the lane was that one `RetryPolicy` owns every budget, so the checks here are
// about *ownership*, not about specific numbers:
//
//   1. No retry constant outside `crates/core-domain/src/retry.rs`. A second
//      copy of "5·2^n capped at an hour" is the defect this lane removed.
//   2. No hand-rolled doubling in a retry path. `(backoff * 2).min(5_000)` was
//      the shape in ten files; it belongs in the policy now.
//   3. Every subsystem in the policy matrix has at least one call site, so the
//      matrix cannot describe a subsystem nothing uses.
//   4. A retry-path doc comment does not restate a budget in seconds. Numbers
//      live in one file; a docstring that repeats one is a second source of
//      truth waiting to disagree with the code. Naming the policy is fine.
//   5. Every env name in the retry table is a descriptor parameter, so a lever
//      cannot exist without being discoverable in `marula deploy describe`.
//
// Non-purpose: it does not run cargo, boot the stack, or read a database. The
// jitter/cap/classification behaviour is pinned by `cargo test -p core-domain`
// (`retry::tests`), and the descriptor completeness gate by
// `cargo test -p marula-core-cli` (`deploy::tests`).
//
// Usage:
//   node scripts/check-reliability.mjs           # human summary, exit 1 on fail
//   node scripts/check-reliability.mjs --json    # machine-readable verdict
//   node scripts/check-reliability.mjs --root DIR

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";

const POLICY_FILE = "crates/core-domain/src/retry.rs";
const POLICY_FILE_REL = POLICY_FILE.split("/").join(sep);
const DESCRIPTOR_FILE = "crates/marula-core-cli/src/deploy.rs";

/** Files that own a retry loop or settle a retry decision. */
const RETRY_PATHS = [
  "crates/worker/src",
  "crates/core-db/src/queue.rs",
  "crates/core-db/src/emails.rs",
  "crates/core-mail/src/outbound.rs",
  "crates/core-domain/src/webhooks.rs",
  "crates/core-events/src/lib.rs",
  "crates/core-api/src/realtime.rs",
  "crates/core-api/src/routes/internal.rs",
  "crates/core-decision/src/system_one.rs",
];

/** Tokens that mean someone re-invented a budget outside the policy. */
const FORBIDDEN_TOKENS = [
  "backoff_ms",
  "MAX_BACKOFF_MS",
  "BASE_BACKOFF_MS",
  "RESUME_MAX_ATTEMPTS",
];

/**
 * A doc comment may name the policy; it may not carry the seconds. The units
 * are required so "capped at one batch" is not mistaken for a time budget.
 */
const DOC_BUDGET =
  /\bcapped at (?:a|an|one|two|three|\d[\d_,]*)\s*(?:s\b|sec|second|min|minute|h\b|hour)|\b(?:to an hour|one hour|1 hour|3_?600\s*(?:s|sec|seconds))\b/i;
const DOC_POLICY = /RetryPolicy|retry::policy|policy's|policy’s/;

/** Prose lines are documentation, not code: they may name what was removed. */
const isComment = (line) => /^\s*\/\//.test(line);

function walk(path, out = []) {
  let st;
  try {
    st = statSync(path);
  } catch {
    return out;
  }
  if (st.isDirectory()) {
    for (const entry of readdirSync(path)) walk(join(path, entry), out);
  } else if (path.endsWith(".rs")) {
    out.push(path);
  }
  return out;
}

function sourceWithoutTests(source) {
  const cut = source.indexOf("#[cfg(test)]");
  return cut === -1 ? source : source.slice(0, cut);
}

/**
 * Definitions of a backoff function or constant outside the policy file.
 * The policy file is allowed to define them; a delegation is allowed to exist
 * elsewhere as long as its body reads the policy.
 */
function retryConstants(root, files) {
  const violations = [];
  for (const file of files) {
    if (file.endsWith(POLICY_FILE_REL)) continue;
    const source = sourceWithoutTests(readFileSync(file, "utf8"));
    const lines = source.split("\n");
    for (let i = 0; i < lines.length; i += 1) {
      const line = lines[i];
      const defines =
        /^\s*(?:pub\s+)?(?:const|static)\s+(?:[A-Z0-9_]+)\s*:\s*(?:u64|i64|u32|i32|Duration)/.test(
          line,
        ) && /backoff|retry|attempt/i.test(line);
      if (!defines) continue;
      // A delegation is fine: the next 12 lines must mention the policy.
      const body = lines.slice(i, i + 12).join("\n");
      if (!/retry::|RetryPolicy/.test(body)) {
        violations.push({
          check: "unowned-constant",
          file: relative(root, file),
          line: i + 1,
          detail: line.trim(),
        });
      }
    }
  }
  return violations;
}

/** The hand-rolled doubling/delay shape this lane replaced. */
function handRolledBackoff(root, files) {
  const violations = [];
  const pattern = /\(\s*\w*backoff\w*\s*\*\s*2\s*\)|min\(\s*(?:5_000|10_000|MAX_BACKOFF_MS)\s*\)/;
  for (const file of files) {
    if (file.endsWith(POLICY_FILE_REL)) continue;
    const lines = sourceWithoutTests(readFileSync(file, "utf8")).split("\n");
    lines.forEach((line, index) => {
      if (!isComment(line) && pattern.test(line)) {
        violations.push({
          check: "hand-rolled-backoff",
          file: relative(root, file),
          line: index + 1,
          detail: line.trim(),
        });
      }
    });
  }
  return violations;
}

/** Tokens that should not survive the move to the policy. */
function forbiddenTokens(root, files) {
  const violations = [];
  for (const file of files) {
    if (file.endsWith(POLICY_FILE_REL)) continue;
    const lines = sourceWithoutTests(readFileSync(file, "utf8")).split("\n");
    lines.forEach((line, index) => {
      if (isComment(line)) return;
      for (const token of FORBIDDEN_TOKENS) {
        if (line.includes(token)) {
          violations.push({
            check: "forbidden-token",
            file: relative(root, file),
            line: index + 1,
            detail: `${token}: ${line.trim()}`,
          });
        }
      }
    });
  }
  return violations;
}

/** Doc comments in retry paths must not restate a budget in seconds. */
function docstringBudgets(root, files) {
  const violations = [];
  for (const file of files) {
    const lines = sourceWithoutTests(readFileSync(file, "utf8")).split("\n");
    lines.forEach((line, index) => {
      const trimmed = line.trim();
      const isDoc = trimmed.startsWith("///") || trimmed.startsWith("//!");
      if (!isDoc) return;
      if (DOC_BUDGET.test(trimmed) && !DOC_POLICY.test(trimmed)) {
        violations.push({
          check: "docstring-budget",
          file: relative(root, file),
          line: index + 1,
          detail: trimmed,
        });
      }
    });
  }
  return violations;
}

/** Every `Subsystem` variant must be referenced by at least one call site. */
function subsystemCallSites(root, files, policySource) {
  const enumBody = policySource.slice(
    policySource.indexOf("pub enum Subsystem"),
    policySource.indexOf("impl Subsystem"),
  );
  const variants = [...enumBody.matchAll(/^\s{4}([A-Z][A-Za-z]*),\s*$/gm)].map((m) => m[1]);
  const violations = [];
  for (const variant of variants) {
    const needle = `Subsystem::${variant}`;
    const used = files
      .filter((file) => !file.endsWith(POLICY_FILE_REL))
      .some((file) => sourceWithoutTests(readFileSync(file, "utf8")).includes(needle));
    if (!used) {
      violations.push({
        check: "orphan-subsystem",
        file: POLICY_FILE,
        line: 0,
        detail: `${needle} has no call site in the tree`,
      });
    }
  }
  return { violations, variants };
}

/** Every retry env name must appear in the deploy descriptor. */
function descriptorParity(root, policySource) {
  const start = policySource.indexOf("RETRY_ENV_NAMES");
  // Only the table itself: the type annotation above it carries a `];`, and
  // the test modules below name keys that do not exist.
  const body = policySource.indexOf("= [", start);
  const table = policySource.slice(body, policySource.indexOf("];", body));
  const names = [...table.matchAll(/"(MARULA_RETRY_[A-Z0-9_]+)"/g)].map((m) => m[1]);
  const robustness = /ROBUSTNESS_ENV: &str = "([A-Z0-9_]+)"/.exec(policySource);
  if (robustness) names.push(robustness[1]);
  const descriptor = readFileSync(join(root, DESCRIPTOR_FILE), "utf8");
  const violations = [];
  for (const name of names) {
    if (!descriptor.includes(`"${name}"`)) {
      violations.push({
        check: "undiscoverable-lever",
        file: DESCRIPTOR_FILE,
        line: 0,
        detail: `${name} is read by the retry resolver but absent from the descriptor`,
      });
    }
  }
  return { violations, names };
}

function main(argv) {
  const json = argv.includes("--json");
  const rootIndex = argv.indexOf("--root");
  const root = rootIndex === -1 ? process.cwd() : argv[rootIndex + 1];

  let policySource;
  try {
    policySource = readFileSync(join(root, POLICY_FILE), "utf8");
  } catch {
    const message = `cannot read ${POLICY_FILE} under ${root} — run from the repo root`;
    if (json) console.log(JSON.stringify({ ok: false, error: message }, null, 2));
    else console.error(`✗ ${message}`);
    process.exit(1);
  }

  const files = RETRY_PATHS.flatMap((path) => walk(join(root, path)));

  const constantViolations = retryConstants(root, files);
  const handRolled = handRolledBackoff(root, files);
  const tokens = forbiddenTokens(root, files);
  const docstrings = docstringBudgets(root, files);
  const subsystem = subsystemCallSites(root, files, policySource);
  const descriptor = descriptorParity(root, policySource);

  const checks = [
    {
      name: "no retry constant outside the policy",
      violations: constantViolations,
    },
    { name: "no hand-rolled doubling in a retry path", violations: handRolled },
    { name: "no retired retry tokens", violations: tokens },
    { name: "no budget restated in a doc comment", violations: docstrings },
    { name: "every subsystem has a call site", violations: subsystem.violations },
    { name: "every retry lever is in the descriptor", violations: descriptor.violations },
  ];

  const failed = checks.filter((c) => c.violations.length > 0);

  if (json) {
    console.log(
      JSON.stringify(
        {
          ok: failed.length === 0,
          retry_paths: files.length,
          subsystems: subsystem.variants,
          levers: descriptor.names.length,
          checks,
        },
        null,
        2,
      ),
    );
  } else {
    console.log(
      `reliability: ${files.length} retry-path files, ${subsystem.variants.length} subsystems, ${descriptor.names.length} levers`,
    );
    for (const check of checks) {
      if (check.violations.length === 0) {
        console.log(`  ✓ ${check.name}`);
        continue;
      }
      console.log(`  ✗ ${check.name} — ${check.violations.length}`);
      for (const violation of check.violations.slice(0, 12)) {
        const where = violation.line > 0 ? `${violation.file}:${violation.line}` : violation.file;
        console.log(`      ${where}  ${violation.detail}`);
      }
      if (check.violations.length > 12) {
        console.log(`      … ${check.violations.length - 12} more`);
      }
    }
    console.log(failed.length === 0 ? "\nRELIABILITY OK" : "\nRELIABILITY FAILED");
  }

  process.exit(failed.length === 0 ? 0 : 1);
}

main(process.argv.slice(2));
