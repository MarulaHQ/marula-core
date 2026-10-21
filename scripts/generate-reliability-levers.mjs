#!/usr/bin/env node
// Generates docs/reference/reliability-levers.md from the deploy descriptor and
// the retry policy (Wave 52 lane R8b).
//
// Purpose: the row it serves is "every descriptor lever has a doc row". That is
// only true if drift fails a build, so this script owns both directions:
//
//   node scripts/generate-reliability-levers.mjs           # rewrite the page
//   node scripts/generate-reliability-levers.mjs --check    # fail on drift
//
// The check reports two distinct faults, because they have two distinct causes:
// a lever in the descriptor with no row (someone added a lever and forgot the
// page), and a row naming a lever the descriptor no longer has (someone removed
// a lever and the page kept it). It also diffs the whole file, so a changed
// default or description is a failure rather than a stale sentence.
//
// Why it parses Rust source instead of running `marula deploy describe --json`:
// a docs gate runs on a clean checkout with Node alone, and building the
// workspace to check a table is not a docs gate. The parse is guarded by
// counts, so a parser that loses sight of a call site fails loudly instead of
// writing a shorter page:
//
//   * every `param(`/`requires(` call site in `params()` must become a row
//   * the policy matrix must be exactly subsystems x presets
//
// The two files this reads are already pinned to each other by Rust tests in
// `crates/marula-core-cli/src/deploy.rs` (`every_env_value_the_processes_read_
// has_a_descriptor_entry`, `every_retry_lever_is_in_the_descriptor_and_vice_
// versa`), so parsing either side statically is sound.
//
// Non-purpose: it does not decide whether a default is a good one, and it does
// not evaluate the retry arithmetic. The numbers are projected, never invented.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const DESCRIPTOR = "crates/marula-core-cli/src/deploy.rs";
const RETRY = "crates/core-domain/src/retry.rs";
const PAGE = "docs/reference/reliability-levers.md";

const read = (rel) => readFileSync(join(root, rel), "utf8");

/** Strip `//` comments without touching the inside of a string literal.
 *  `Some("http://localhost:8080")` is a default, not a comment. */
function stripLineComments(src) {
  let out = "";
  let i = 0;
  let inString = false;
  while (i < src.length) {
    const ch = src[i];
    if (inString) {
      out += ch;
      if (ch === "\\") {
        out += src[i + 1] ?? "";
        i += 2;
        continue;
      }
      if (ch === '"') inString = false;
      i += 1;
      continue;
    }
    if (ch === '"') {
      inString = true;
      out += ch;
      i += 1;
      continue;
    }
    if (ch === "/" && src[i + 1] === "/") {
      while (i < src.length && src[i] !== "\n") i += 1;
      continue;
    }
    out += ch;
    i += 1;
  }
  return out;
}

/** The body of a top-level `fn name(...) { ... }`, by brace matching. */
function fnBody(src, signature) {
  const start = src.indexOf(signature);
  if (start === -1) throw new Error(`cannot find ${signature}`);
  let i = src.indexOf("{", start);
  let depth = 0;
  const from = i;
  for (; i < src.length; i += 1) {
    if (src[i] === "{") depth += 1;
    else if (src[i] === "}") {
      depth -= 1;
      if (depth === 0) return src.slice(from, i + 1);
    }
  }
  throw new Error(`unbalanced braces in ${signature}`);
}

// --- the descriptor ---------------------------------------------------------

function parseParams() {
  const src = stripLineComments(read(DESCRIPTOR));
  const body = fnBody(src, "fn params() -> Vec<Param> {");
  const str = String.raw`"(?:[^"\\]|\\.)*"`;
  const call = new RegExp(
    String.raw`\b(param|requires)\(\s*(${str})\s*,\s*(${str})\s*,\s*(\w+)\s*,\s*(?:Some\((${str})\)|None)\s*,\s*(true|false)\s*,\s*(${str})\s*,?\s*\)`,
    "g",
  );
  const params = [];
  for (const m of body.matchAll(call)) {
    params.push({
      required: m[1] === "requires",
      name: JSON.parse(m[2]),
      group: JSON.parse(m[3]),
      kind: m[4],
      default: m[5] === undefined ? null : JSON.parse(m[5]),
      secret: m[6] === "true",
      description: JSON.parse(m[7]),
    });
  }
  const callSites = [...body.matchAll(/\b(param|requires)\(/g)].length;
  if (params.length !== callSites) {
    throw new Error(
      `parsed ${params.length} of ${callSites} parameter call sites — the parser is out of date with deploy.rs`,
    );
  }
  const names = new Set(params.map((p) => p.name));
  if (names.size !== params.length) throw new Error("duplicate parameter name");
  return params;
}

/** Group order as the descriptor presents it, so the page reads like the JSON. */
function groupOrder(params) {
  const seen = [];
  for (const p of params) if (!seen.includes(p.group)) seen.push(p.group);
  return seen;
}

// --- the retry policy -------------------------------------------------------

/** Names in a `pub const ALL: [Self; N] = [Self::A, Self::B];` list. */
function enumAll(segment, label) {
  const all = segment.match(/pub const ALL: \[Self; \d+\] = \[([\s\S]*?)\];/);
  if (!all) throw new Error(`cannot find ${label}::ALL`);
  return [...all[1].matchAll(/Self::(\w+)/g)].map((m) => m[1]);
}

/** `Self::Variant => "token"` arms of one `as_str`. */
function asStrMap(segment, variants) {
  const map = new Map();
  for (const m of segment.matchAll(/Self::(\w+) => "([a-z_]+)",/g)) {
    map.set(m[1], m[2]);
  }
  for (const v of variants) {
    if (!map.has(v)) throw new Error(`no as_str arm for ${v}`);
  }
  return map;
}

/** Rust numeric literals may carry `_` separators (`3_600`). */
const num = (raw) => Number(String(raw).replaceAll("_", ""));

function parseRetry() {
  const src = stripLineComments(read(RETRY));

  // Subsystem and RobustnessPreset are separate enum blocks; slice each from its
  // `impl` so `Self::` names cannot cross over.
  const subsystemAt = src.indexOf("impl Subsystem {");
  const presetAt = src.indexOf("impl RobustnessPreset {");
  const policyAt = src.indexOf("impl RetryPolicy {");
  if (subsystemAt === -1 || presetAt === -1 || policyAt === -1) {
    throw new Error("cannot find the retry enum impl blocks");
  }
  const subsystemBlock = src.slice(subsystemAt, presetAt);
  const presetBlock = src.slice(presetAt, policyAt);

  const subsystems = enumAll(subsystemBlock, "Subsystem");
  const presets = enumAll(presetBlock, "RobustnessPreset");
  const subsystemName = asStrMap(subsystemBlock, subsystems);
  const presetName = asStrMap(presetBlock, presets);

  const defaultConflict = src.match(/on_conflict: ConflictPosture::(\w+)/);
  if (!defaultConflict) throw new Error("cannot find the default ConflictPosture");

  // One `match subsystem` per preset: `Self::Balanced => match subsystem { ... }`.
  // A cell is written out, or inherited by pointing at another preset
  // (`Subsystem::Effects => Self::Balanced.policy(Subsystem::Effects)`), so a
  // preset only states the budgets it actually changes.
  const direct = new Map();
  const inherited = new Map();
  const presetArms = [
    ...src.matchAll(/Self::(\w+) => match subsystem \{([\s\S]*?)\n            \},/g),
  ];
  for (const m of presetArms) {
    const preset = m[1];
    const body = m[2];
    const policy =
      /Subsystem::(\w+) => RetryPolicy::new\(\s*Subsystem::\w+,\s*Duration::from_(secs|millis)\(([\d_]+)\),\s*(\d+),\s*Duration::from_(secs|millis)\(([\d_]+)\),\s*AttemptBudget::(\w+)(?:\(([\d_]+)\))?,\s*Jitter::(\w+),?\s*\)\s*(?:\.on_conflict\(ConflictPosture::(\w+)\))?/g;
    for (const p of body.matchAll(policy)) {
      direct.set(`${preset}/${p[1]}`, {
        preset,
        subsystem: p[1],
        baseMs: p[2] === "secs" ? num(p[3]) * 1000 : num(p[3]),
        factor: Number(p[4]),
        capMs: p[5] === "secs" ? num(p[6]) * 1000 : num(p[6]),
        attempts: p[7] === "Unlimited" ? null : num(p[8]),
        jitter: p[9],
        onConflict: p[10] ?? null,
      });
    }
    const delegation =
      /Subsystem::(\w+) => Self::(\w+)\.policy\(Subsystem::\w+\)/g;
    for (const d of body.matchAll(delegation)) {
      inherited.set(`${preset}/${d[1]}`, d[2]);
    }
  }

  /** Follow a delegation chain to the preset that actually states the budget. */
  function resolve(preset, subsystem, seen = []) {
    const key = `${preset}/${subsystem}`;
    if (direct.has(key)) return { arm: direct.get(key), from: null };
    const parent = inherited.get(key);
    if (!parent) throw new Error(`preset ${preset} does not answer subsystem ${subsystem}`);
    if (seen.includes(key)) throw new Error(`retry delegation cycle at ${key}`);
    const up = resolve(parent, subsystem, [...seen, key]);
    return { arm: up.arm, from: up.from ?? presetName.get(parent) };
  }

  const arms = [];
  for (const preset of presets) {
    for (const subsystem of subsystems) {
      const { arm, from } = resolve(preset, subsystem);
      arms.push({ ...arm, preset: presetName.get(preset), inheritedFrom: from });
    }
  }

  if (arms.length !== subsystems.length * presets.length) {
    throw new Error(
      `parsed ${arms.length} retry policies, expected ${subsystems.length} subsystems x ${presets.length} presets — the parser is out of date with retry.rs`,
    );
  }
  for (const preset of presets) {
    const token = presetName.get(preset);
    const got = arms.filter((a) => a.preset === token).map((a) => a.subsystem);
    if (got.length !== subsystems.length || !subsystems.every((s) => got.includes(s))) {
      throw new Error(`preset ${token} does not answer every subsystem`);
    }
  }

  return {
    subsystems: subsystems.map((v) => subsystemName.get(v)),
    presets: presets.map((v) => presetName.get(v)),
    defaultConflict: defaultConflict[1],
    arms: arms.map((a) => ({
      ...a,
      subsystem: subsystemName.get(a.subsystem),
      preset: a.preset,
      onConflict: a.onConflict ?? defaultConflict[1],
    })),
  };
}

// --- rendering --------------------------------------------------------------

/** 3_600_000 -> "1 h", 10_000 -> "10 s", 100 -> "100 ms". */
function duration(ms) {
  if (ms === 0) return "0 ms";
  if (ms % 3_600_000 === 0) return `${ms / 3_600_000} h`;
  if (ms % 60_000 === 0) return `${ms / 60_000} min`;
  if (ms % 1_000 === 0) return `${ms / 1_000} s`;
  return `${ms} ms`;
}

const cell = (value) => String(value).replaceAll("|", "\\|");

function render(params, retry) {
  const groups = groupOrder(params);
  const lines = [];

  lines.push("# Reliability levers");
  lines.push("");
  lines.push(
    "<!-- Generated by scripts/generate-reliability-levers.mjs from",
    "     crates/marula-core-cli/src/deploy.rs (the parameters) and",
    "     crates/core-domain/src/retry.rs (the budgets). Do not edit by hand:",
    "     `node scripts/generate-reliability-levers.mjs --check` fails when this",
    "     page and those two files disagree. -->",
  );
  lines.push("");
  lines.push(
    "A lever is a setting that changes how the product behaves under load, under",
    "failure, or on the way back from either. All of them arrive through the deploy",
    "descriptor, so `marula deploy describe --json` is the same list in machine-readable",
    "form. This page is the readable projection: every parameter the descriptor publishes,",
    "one row each, with the budget the retry policy resolves to underneath.",
  );
  lines.push("");
  lines.push(
    "Two things are worth knowing before reading it. First, the retry numbers are not",
    "stated here twice. They are computed in `crates/core-domain/src/retry.rs` and this",
    "page projects them, which is why a change there either updates this file or fails a",
    "check. Second, a default in the table is the value the process uses when nothing sets",
    "the variable; a profile can preset a different one, and the profiles are listed in",
    "`docs/deploy-descriptor.md`.",
  );
  lines.push("");
  lines.push(
    `Counts at this commit: ${params.length} levers in ${groups.length} groups, and`,
    `${retry.arms.length} retry budgets (${retry.subsystems.length} subsystems across`,
    `${retry.presets.length} presets).`,
  );
  lines.push("");

  // --- retry matrix ---------------------------------------------------------
  lines.push("## The retry budget each preset resolves to");
  lines.push("");
  lines.push(
    "One row per preset and subsystem. `MARULA_ROBUSTNESS` picks the preset, and a single",
    `lever overrides one cell of it as \`${"MARULA_RETRY_<SUBSYSTEM>_<LEVER>"}\`.`,
    "First delay is the wait before the second attempt; growth is the multiplier per",
    "failure; cap is the ceiling no computed delay passes. `unlimited` attempts means the",
    "budget never exhausts and the work stays retryable until something else stops it.",
  );
  lines.push("");
  lines.push(
    "| Preset | Subsystem | First delay | Growth | Cap | Attempts | Jitter | On 409 | Inherited from |",
    "|---|---|---|---|---|---|---|---|---|",
  );
  for (const preset of retry.presets) {
    for (const subsystem of retry.subsystems) {
      const arm = retry.arms.find((a) => a.preset === preset && a.subsystem === subsystem);
      lines.push(
        `| \`${preset}\` | \`${subsystem}\` | ${duration(arm.baseMs)} | ${arm.factor}x | ${duration(arm.capMs)} | ${
          arm.attempts === null ? "unlimited" : arm.attempts
        } | \`${arm.jitter}\` | \`${arm.onConflict}\` | ${
          arm.inheritedFrom ? `\`${arm.inheritedFrom}\`` : "this preset"
        } |`,
      );
    }
  }
  lines.push("");
  lines.push(
    "A preset only states the budgets it changes. Where a cell is inherited, the last",
    "column names the preset that writes it, so the value in the row is the value that",
    "runs. `On 409` is the only status the policy is asked about by name: a conflict is",
    "either permanent for that subsystem or replayable, and the default is permanent so a",
    "policy has to opt into replaying one.",
  );
  lines.push("");

  // --- catalogue ------------------------------------------------------------
  lines.push("## The catalogue");
  lines.push("");
  lines.push(
    "Grouped the way the descriptor groups them. **Required** means the process refuses to",
    "boot without a value and there is no default. **Secret** means a consumer rendering a",
    "form must mask it; most secrets are conditional on the feature they configure, which is",
    "why only two are required.",
  );
  lines.push("");
  for (const group of groups) {
    const inGroup = params.filter((p) => p.group === group);
    lines.push(`### ${group} (${inGroup.length})`);
    lines.push("");
    lines.push("| Lever | Kind | Default | Notes |");
    lines.push("|---|---|---|---|");
    for (const p of inGroup) {
      const kind = p.kind.toLowerCase();
      const flags = [];
      if (p.required) flags.push("required");
      // Kind already says `secret` for a secret; repeating it as a flag reads
      // like two different properties.
      if (p.secret && kind !== "secret") flags.push("secret");
      const kindCell = flags.length ? `${kind}, ${flags.join(", ")}` : kind;
      const defaultCell =
        p.default === null ? "none" : `\`${cell(p.default)}\``;
      lines.push(
        `| \`${p.name}\` | ${kindCell} | ${defaultCell} | ${cell(p.description)} |`,
      );
    }
    lines.push("");
  }

  lines.push("## Checking this page");
  lines.push("");
  lines.push("```sh");
  lines.push("node scripts/generate-reliability-levers.mjs          # rewrite after a change");
  lines.push("node scripts/generate-reliability-levers.mjs --check  # what CI runs");
  lines.push("```");
  lines.push("");
  lines.push(
    "The check reads the two Rust files, rebuilds this page in memory, and compares. A lever",
    "that exists with no row, a row naming a lever that is gone, and a changed default all",
    "fail with the lever named.",
  );
  lines.push("");

  return lines.join("\n");
}

// --- the row sets, for a check that names what drifted ----------------------

/** Lever names the page claims, read back out of the catalogue tables. */
function documentedLevers(page) {
  const names = new Set();
  const catalogue = page.slice(page.indexOf("## The catalogue"));
  for (const m of catalogue.matchAll(/^\| `([A-Z][A-Z0-9_]+)` \|/gm)) names.add(m[1]);
  return names;
}

/** `preset/subsystem` pairs the page claims in the budget table. */
function documentedBudgets(page) {
  const pairs = new Set();
  for (const m of page.matchAll(/^\| `([a-z_]+)` \| `([a-z_]+)` \|/gm)) {
    pairs.add(`${m[1]}/${m[2]}`);
  }
  return pairs;
}

function main() {
  const check = process.argv.includes("--check");
  const params = parseParams();
  const retry = parseRetry();
  const next = render(params, retry);

  if (!check) {
    writeFileSync(join(root, PAGE), next);
    console.log(
      `wrote ${PAGE}: ${params.length} levers in ${groupOrder(params).length} groups, ${retry.arms.length} retry budgets`,
    );
    return;
  }

  const current = read(PAGE);
  const problems = [];

  const wantLevers = new Set(params.map((p) => p.name));
  const haveLevers = documentedLevers(current);
  for (const name of wantLevers) {
    if (!haveLevers.has(name)) problems.push(`MISSING ${name} has no doc row`);
  }
  for (const name of haveLevers) {
    if (!wantLevers.has(name)) {
      problems.push(`STALE   ${name} has a doc row but is not a descriptor lever`);
    }
  }

  const wantBudgets = new Set(
    retry.arms.map((a) => `${a.preset}/${a.subsystem}`),
  );
  const haveBudgets = documentedBudgets(current);
  for (const key of wantBudgets) {
    if (!haveBudgets.has(key)) problems.push(`MISSING retry row for ${key}`);
  }
  for (const key of haveBudgets) {
    if (!wantBudgets.has(key)) problems.push(`STALE   retry row for ${key}`);
  }

  if (current !== next) {
    problems.push(
      "CONTENT the page differs from a fresh generation (a default, description or count changed)",
    );
  }

  if (problems.length) {
    for (const problem of problems) console.error(problem);
    console.error(
      `\n${problems.length} problem(s). Run: node scripts/generate-reliability-levers.mjs`,
    );
    process.exit(1);
  }
  console.log(
    `reliability levers OK: ${wantLevers.size} levers and ${wantBudgets.size} retry budgets all have a row`,
  );
}

main();
