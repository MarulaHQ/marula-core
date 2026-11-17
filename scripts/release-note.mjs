#!/usr/bin/env node
// Release notes: one fragment per pull request, and the version they add up to.
//
// Every pull request that changes something a user can see writes one file in
// `release-notes/unreleased/`. The file declares the bump that change needs —
// `minor` for a capability, `patch` for a fix — and carries the changelog
// bullet in the user's words. `plan` adds the fragments up; `apply` writes the
// release; `check` is the pull-request gate that makes sure nobody forgets.
//
// Why fragments instead of a changelog edit in every PR: the changelog is
// ordered by release, and concurrent PRs would fight over one file. A fragment
// is a file nobody else touches, and the order is decided once, at release.
//
// Usage
//   node scripts/release-note.mjs add --bump patch|minor|none --slug <slug> \
//        --section Added|Changed|Fixed|Security|Internal --note "one line" [--note "..."]
//   node scripts/release-note.mjs list
//   node scripts/release-note.mjs plan
//   node scripts/release-note.mjs apply --date YYYY-MM-DD
//   node scripts/release-note.mjs check --base origin/main
//   node scripts/release-note.mjs advise --base origin/main
//   node scripts/release-note.mjs --self-test
//
// `none` is the honest answer for product code with no user-facing change: the
// fragment is a record that the author thought about the release, and it is
// dropped at `apply` time.

import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
  existsSync,
} from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const NOTES_DIR = join(ROOT, "release-notes/unreleased");
const CHANGELOG = join(ROOT, "CHANGELOG.md");
const CARGO = join(ROOT, "Cargo.toml");

/** Every package that carries the product version. Kept in one list so a bump
 * cannot move seven of eight. */
const VERSION_FILES = [
  "Cargo.toml",
  "apps/web/package.json",
  "apps/installer/package.json",
  "packages/ui/package.json",
  "packages/ui-native/package.json",
  "packages/layout/package.json",
  "packages/sdk/package.json",
  "packages/plugin-kit/package.json",
  "native/package.json",
  // The shells read their own version from these, and a release that moved
  // nine of eleven files ships a desktop "About" that disagrees with the API.
  "native/app.json",
  "desktop/tauri.conf.json",
];

const SECTIONS = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"];
const BUMPS = ["patch", "minor", "none"];

/** Product paths: a change here can reach a user. Tests, docs and tooling
 * cannot, and asking for a fragment on those is how a gate gets ignored. */
const PRODUCT_ROOTS = /^(crates|apps|packages|plugins|native)\//;
const NON_PRODUCT = [
  /(^|\/)tests?\//,
  /\.md$/,
  /^(e2e|docs|scripts|\.github|design)\//,
  /(^|\/)tests?\.(rs|ts|tsx)$/,
  /\.(test|spec)\.(ts|tsx|js|jsx)$/,
  // A version or dependency manifest is not behaviour. A release moves the
  // number in nine files and asks its own author for no fragment.
  /(^|\/)package\.json$/,
  /^Cargo\.(toml|lock)$/,
  // Generated from the handlers: the fragment belongs to the code that changed
  // the contract, not to the snapshot it produced.
  /^packages\/sdk\/openapi\.json$/,
  /^packages\/sdk\/src\/generated\//,
];

const USAGE = `Usage
  node scripts/release-note.mjs add --bump patch|minor|none --slug <slug> \\
       --section Added|Changed|Fixed|Security|Internal --note "one line" [--note "..."]
  node scripts/release-note.mjs list
  node scripts/release-note.mjs plan
  node scripts/release-note.mjs apply --date YYYY-MM-DD
  node scripts/release-note.mjs check --base origin/main
  node scripts/release-note.mjs advise --base origin/main
  node scripts/release-note.mjs --self-test`;

function usage(code = 0) {
  (code === 0 ? console.log : console.error)(USAGE);
  process.exit(code);
}

function fail(message) {
  console.error(`release-note: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const flags = { notes: [] };
  for (let i = 0; i < rest.length; i += 1) {
    const arg = rest[i];
    if (!arg.startsWith("--")) fail(`unexpected argument ${arg}`);
    const key = arg.slice(2);
    if (key === "note") {
      const value = rest[++i];
      if (value === undefined || value.startsWith("--")) {
        fail("--note needs a value (an empty flag would be dropped silently)");
      }
      flags.notes.push(value);
      continue;
    }
    if (key === "self-test") {
      flags.selfTest = true;
      continue;
    }
    const value = rest[i + 1];
    if (value === undefined || value.startsWith("--")) fail(`--${key} needs a value`);
    flags[key] = value;
    i += 1;
  }
  return { command, flags };
}

// --- versions ---------------------------------------------------------------

export function parseVersion(raw) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(raw.trim());
  if (match === null) throw new Error(`not a version: ${raw}`);
  return match.slice(1).map(Number);
}

/** A fragment's bump applied to the current version.
 *
 * 0.x is the only series this product has shipped, so "minor" is the release
 * that says "here is a capability" and it moves the middle number: 0.3.7 ->
 * 0.4.0. Patch moves the last. This is the whole rule, and it is here so two
 * agents cannot disagree about it in prose. */
export function nextVersion(current, bump) {
  const [major, minor, patch] = parseVersion(current);
  if (bump === "none") return `${major}.${minor}.${patch}`;
  if (bump === "patch") return `${major}.${minor}.${patch + 1}`;
  if (bump === "minor") return `${major}.${minor + 1}.0`;
  throw new Error(`unknown bump ${bump}`);
}

export function highestBump(bumps) {
  if (bumps.includes("minor")) return "minor";
  if (bumps.includes("patch")) return "patch";
  return "none";
}

/** A real calendar date, not just the shape: `2026-02-31` matches YYYY-MM-DD
 * and does not exist, and the value is published in a changelog heading the
 * website builds its feed from. */
export function isCalendarDate(value) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const parsed = new Date(`${value}T00:00:00Z`);
  return !Number.isNaN(parsed.getTime()) && parsed.toISOString().slice(0, 10) === value;
}

function currentVersion() {
  const cargo = readFileSync(CARGO, "utf8");
  const match = /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m.exec(cargo);
  if (match === null) fail("Cargo.toml has no [workspace.package] version");
  return match[1];
}

// --- fragments --------------------------------------------------------------

export function parseFragment(raw, file = "<fragment>") {
  const match = /^---\n([\s\S]*?)\n---\n([\s\S]*)$/.exec(raw.replace(/\r\n/g, "\n"));
  if (match === null) throw new Error(`${file}: needs a --- frontmatter block`);
  const [, front, body] = match;
  const bumpMatch = /^bump:\s*(\S+)\s*$/m.exec(front);
  if (bumpMatch === null) throw new Error(`${file}: frontmatter needs "bump:"`);
  const bump = bumpMatch[1];
  if (!BUMPS.includes(bump)) throw new Error(`${file}: bump must be one of ${BUMPS.join(", ")}`);
  const trimmed = body.trim();
  if (trimmed === "") throw new Error(`${file}: the body is empty`);
  // Anchored at the start, no `m` flag: `assembleChangelog` reads everything
  // after the FIRST heading as that section's bullets, so a heading found in
  // the middle would silently publish the text above it under the wrong one.
  const heading = /^### (Added|Changed|Deprecated|Removed|Fixed|Security|Internal)$/.exec(
    trimmed.split("\n", 1)[0],
  );
  if (heading === null) {
    throw new Error(`${file}: the body must START with a ### heading`);
  }
  const headings = trimmed.match(/^### /gm) ?? [];
  if (headings.length > 1) {
    throw new Error(
      `${file}: one fragment carries one ### heading, found ${headings.length}`,
    );
  }
  if (!/^- \S/m.test(trimmed)) throw new Error(`${file}: body needs at least one "- " bullet`);
  if (bump === "none" && heading[1] !== "Internal") {
    throw new Error(`${file}: bump: none must use ### Internal (nothing reaches the changelog)`);
  }
  if (bump !== "none" && heading[1] === "Internal") {
    throw new Error(`${file}: ### Internal cannot ship a ${bump} bump`);
  }
  return { bump, body: trimmed };
}

function fragments() {
  if (!existsSync(NOTES_DIR)) return [];
  return readdirSync(NOTES_DIR)
    .filter((name) => name.endsWith(".md") && name !== "README.md")
    .sort()
    .map((name) => {
      const path = join(NOTES_DIR, name);
      const parsed = parseFragment(readFileSync(path, "utf8"), name);
      return { name, path, ...parsed };
    });
}

export function assembleChangelog(version, date, list) {
  const bySection = new Map();
  for (const fragment of list) {
    if (fragment.bump === "none") continue;
    const match = /^### (Added|Changed|Deprecated|Removed|Fixed|Security)[^\n]*\n([\s\S]*)$/.exec(
      fragment.body,
    );
    if (match === null) continue;
    const [, section, rest] = match;
    const bullets = rest.trim();
    bySection.set(section, [...(bySection.get(section) ?? []), bullets]);
  }
  const parts = [`## [${version}] - ${date}`];
  for (const section of SECTIONS) {
    if (!bySection.has(section)) continue;
    parts.push("", `### ${section}`, "", bySection.get(section).join("\n"));
  }
  parts.push("");
  // Two newlines, not one: the section is inserted directly before the next
  // release heading, and Markdown needs a blank line on both sides of it.
  return `${parts.join("\n")}\n`;
}

/** The changelog text a release would produce. Pure: no write, so `apply` can
 * prepare every file and only then touch the tree. */
function prepareChangelog(version, date, release, list) {
  const section = assembleChangelog(version, date, list);
  let text = readFileSync(CHANGELOG, "utf8");
  const unreleased = /^## \[Unreleased\][\s\S]*?(?=^## \[)/m.exec(text);
  if (unreleased === null) fail("CHANGELOG.md has no [Unreleased] section to sit under");
  const at = unreleased.index + unreleased[0].length;
  text = `${text.slice(0, at)}${section}${text.slice(at)}`;
  // The compare links at the bottom are the navigation; keep them current.
  text = text.replace(
    /^\[Unreleased\]: .*$/m,
    `[Unreleased]: https://github.com/MarulaHQ/marula-core/compare/v${version}...HEAD`,
  );
  if (!text.includes(`[${version}]: `)) {
    const line = `[${version}]: https://github.com/MarulaHQ/marula-core/compare/v${release}...v${version}\n`;
    text = text.replace("[Unreleased]: ", `${line}[Unreleased]: `);
  }
  return { path: CHANGELOG, content: text };
}

/** Every version file at `version`, validated before any of them is written. */
function prepareVersionFiles(version) {
  return VERSION_FILES.map((relative) => {
    const path = join(ROOT, relative);
    let before;
    try {
      before = readFileSync(path, "utf8");
    } catch (error) {
      fail(
        `${relative} is listed as a version file but cannot be read ` +
          `(${error.code ?? error.message}); update VERSION_FILES with it`,
      );
    }
    const after =
      relative === "Cargo.toml"
        ? before.replace(
            /(\[workspace\.package\][\s\S]*?^version\s*=\s*")[^"]+(")/m,
            `$1${version}$2`,
          )
        : before.replace(/("version"\s*:\s*")[^"]+(")/, `$1${version}$2`);
    if (after === before) fail(`${relative} did not carry the version`);
    return { path, content: after };
  });
}

/** Write through a sibling temp file, so a reader never sees half a file. */
function writeAtomically(path, content) {
  const staging = `${path}.release-note.tmp`;
  writeFileSync(staging, content);
  renameSync(staging, path);
}

// --- git --------------------------------------------------------------------

function git(args) {
  return execFileSync("git", args, { cwd: ROOT, encoding: "utf8" }).trim();
}

/** Whether `base` can be handed to git as a revision argument rather than an
 * option. Only the two things that make a string an option matter: it must not
 * start with a dash, and it must not contain whitespace or a control character.
 * Everything else is git's rule to apply, and a ref may contain `+`, which a
 * character whitelist would have rejected. */
export function isRevisionArgument(base) {
  return base !== "" && !base.startsWith("-") && !/[\s\x00-\x1f]/.test(base);
}

/** A base that resolves to a commit. The shape check runs first so a dash-led
 * value never reaches git at all; `--end-of-options` covers the resolution step
 * on the same reasoning. */
function safeBase(base) {
  if (!isRevisionArgument(base)) fail(`--base ${base} is not a git revision`);
  try {
    execFileSync(
      "git",
      ["rev-parse", "--verify", "--quiet", "--end-of-options", `${base}^{commit}`],
      { cwd: ROOT, stdio: "pipe" },
    );
  } catch {
    fail(`--base ${base} does not resolve to a commit; fetch it first`);
  }
  return base;
}

export function isProductPath(path) {
  // A version manifest is not behaviour, whichever directory it sits in, and
  // this is the same list the pump writes — so adding a manifest to the release
  // cannot make every future bump ask its author for a fragment. Answered
  // first, on its own line, so the exemption does not depend on reading a
  // compound condition correctly.
  if (VERSION_FILES.includes(path)) return false;
  if (!PRODUCT_ROOTS.test(path)) return false;
  return !NON_PRODUCT.some((pattern) => pattern.test(path));
}

function changedPaths(base) {
  return git(["diff", "--name-only", `${base}...HEAD`])
    .split("\n")
    .filter(Boolean);
}

function addedFragments(base) {
  return git(["diff", "--name-status", `${base}...HEAD`, "--", "release-notes/unreleased"])
    .split("\n")
    .filter((line) => line.startsWith("A\t"))
    .map((line) => line.slice(2))
    // The directory holds a README and nothing else that is not a fragment;
    // parsing it as one would fail a pull request that only edited the docs.
    .filter((path) => path.endsWith(".md") && !path.endsWith("/README.md"));
}

// --- commands ---------------------------------------------------------------

function cmdAdd(flags) {
  const bump = flags.bump;
  if (!BUMPS.includes(bump)) fail(`--bump must be one of ${BUMPS.join(", ")}`);
  const section = flags.section;
  const allowed = bump === "none" ? ["Internal"] : SECTIONS;
  if (!allowed.includes(section)) {
    fail(`--section must be one of ${allowed.join(", ")} for a ${bump} bump`);
  }
  // The slug becomes a filename, so it is the one input that must not be able
  // to leave the directory: `--slug ../../.github/workflows/x` would otherwise
  // write wherever the caller pointed it.
  if (!/^[a-z0-9][a-z0-9-]*$/.test(flags.slug ?? "")) {
    fail("--slug must be lowercase letters, digits and dashes (it becomes a filename)");
  }
  const notes = flags.notes.filter((note) => typeof note === "string" && note.trim() !== "");
  if (notes.length === 0) fail("at least one --note is required; it becomes the bullet");
  const date = new Date().toISOString().slice(0, 10);
  const file = join(NOTES_DIR, `${date}-${flags.slug}.md`);
  if (existsSync(file)) fail(`${file} already exists; pick another slug`);
  mkdirSync(NOTES_DIR, { recursive: true });
  const body = [`---`, `bump: ${bump}`, `---`, "", `### ${section}`, ""]
    .concat(notes.map((note) => `- ${note.trim()}`))
    .join("\n");
  writeFileSync(file, `${body}\n`);
  console.log(`wrote ${file.slice(ROOT.length + 1)}`);
  const suggestion = plan();
  console.log(`next release as it stands: ${suggestion.version} (${suggestion.bump})`);
}

function cmdList() {
  const list = fragments();
  if (list.length === 0) {
    console.log("no fragments: the next release carries no user-facing change yet");
    return;
  }
  for (const fragment of list) {
    console.log(`${fragment.bump.padEnd(5)}  ${fragment.name}`);
  }
}

export function plan(list = fragments(), current = currentVersion()) {
  const bump = highestBump(list.map((fragment) => fragment.bump));
  const version = nextVersion(current, bump);
  const minorCount = list.filter((f) => f.bump === "minor").length;
  const patchCount = list.filter((f) => f.bump === "patch").length;
  const reasons =
    bump === "minor"
      ? `${minorCount} fragment(s) declare minor`
      : bump === "patch"
        ? `${patchCount} fragment(s) declare patch and none declares minor`
        : "no fragment declares patch or minor";
  return { version, bump, current, reasons, count: list.length };
}

function cmdPlan() {
  const result = plan();
  console.log(`current  ${result.current}`);
  console.log(`next     ${result.version}  (${result.bump})`);
  console.log(`why      ${result.reasons}`);
  if (result.bump === "none") {
    console.log("nothing to release: add a fragment when a user-visible change lands");
  }
}

function cmdApply(flags) {
  const list = fragments();
  const result = plan(list);
  if (result.bump === "none") fail("no patch or minor fragment: nothing to release");
  // A release says what shipped on the day it shipped; a caller that wants a
  // backdated entry (a split of work that already merged) passes --date.
  const date = flags.date ?? new Date().toISOString().slice(0, 10);
  if (!isCalendarDate(date)) fail(`--date ${date} is not a calendar date (YYYY-MM-DD)`);
  // Prepare every write first. A validation failure above leaves the tree
  // untouched; without this, a bad manifest would exit after the changelog was
  // already rewritten and before any fragment was consumed, and the next run
  // would read the bumped version and plan a release that never happened.
  const writes = [
    prepareChangelog(result.version, date, result.current, list),
    ...prepareVersionFiles(result.version),
  ];
  for (const write of writes) writeAtomically(write.path, write.content);
  for (const fragment of list) rmSync(fragment.path);
  console.log(`released ${result.version} (${result.bump}) with ${list.length} fragment(s)`);
  console.log("still to do, in order:");
  console.log("  1. scripts/regen-openapi-pins.sh   (the API version is the crate version)");
  console.log("  2. node docs/site/scripts/generate-api-reference.mjs");
  console.log("  3. git tag -s v" + result.version + " -m v" + result.version);
}

function cmdCheck(flags) {
  const base = safeBase(flags.base ?? "origin/main");
  let paths;
  try {
    paths = changedPaths(base);
  } catch {
    fail(`cannot diff against ${base}; fetch it first (git fetch origin ${base.split("/").pop()})`);
  }
  const product = paths.filter(isProductPath);
  const added = addedFragments(base);
  for (const relative of added) {
    parseFragment(readFileSync(join(ROOT, relative), "utf8"), relative);
  }
  if (product.length > 0 && added.length === 0) {
    console.error("release-note: this pull request changes product code but carries no release note.");
    console.error("");
    console.error(`  ${product.length} product file(s) changed, for example: ${product[0]}`);
    console.error("");
    console.error("  Add one with:");
    console.error('    node scripts/release-note.mjs add --bump patch --slug <short-name> \\');
    console.error('         --section Fixed --note "what changed for the person using it"');
    console.error("");
    console.error("  bump: minor  a capability someone can use (0.4.0: the middle number moves)");
    console.error("  bump: patch  a fix to something that already works");
    console.error("  bump: none   product code with no user-visible change (### Internal)");
    console.error("  Only tests, docs, e2e, scripts and .github are exempt.");
    process.exit(1);
  }
  console.log(`release-note: ${added.length} fragment(s) on ${product.length} changed product file(s)`);
}

export function advisePaths(paths) {
  const product = paths.filter(isProductPath);
  if (product.length === 0) return { bump: "none", reasons: ["no product path changed"] };
  const reasons = [];
  const capability = [
    [/^crates\/.*\/migrations\//, "a migration: new stored state"],
    [/^crates\/core-api\/src\/routes\/[^/]+\.rs$/, "a route module: new or changed surface"],
    [/^apps\/web\/app\/.*page\.tsx$/, "a page"],
    [/^packages\/ui\/src\//, "a shared component"],
    [/^plugins\/connectors\/[^/]+\/src\/(?!tests)/, "a connector capability"],
  ];
  for (const [pattern, reason] of capability) {
    const hit = product.find((path) => pattern.test(path));
    if (hit !== undefined) reasons.push(`${hit} — ${reason}`);
  }
  if (reasons.length > 0) return { bump: "minor", reasons };
  return {
    bump: "patch",
    reasons: [`${product.length} product file(s) changed outside a new-surface path`],
  };
}

function cmdAdvise(flags) {
  const base = safeBase(flags.base ?? "origin/main");
  const result = advisePaths(changedPaths(base));
  console.log(`suggested bump: ${result.bump}`);
  for (const reason of result.reasons) console.log(`  ${reason}`);
  console.log("");
  console.log(
    result.bump === "none"
      ? "no fragment needed (tests, docs, tooling)"
      : `confirm with: node scripts/release-note.mjs add --bump ${result.bump} …`,
  );
  console.log("the suggestion is a heuristic; the fragment is the decision");
}

// --- self-test --------------------------------------------------------------

function selfTest() {
  const assert = (condition, message) => {
    if (!condition) {
      console.error(`self-test failed: ${message}`);
      process.exit(1);
    }
  };

  assert(JSON.stringify(parseVersion("0.3.7")) === "[0,3,7]", "parseVersion");
  assert(nextVersion("0.3.7", "patch") === "0.3.8", "patch moves the last number");
  assert(nextVersion("0.3.7", "minor") === "0.4.0", "minor moves the middle number in 0.x");
  assert(nextVersion("0.4.0", "minor") === "0.5.0", "minor again");
  assert(nextVersion("0.4.0", "none") === "0.4.0", "none holds");
  assert(highestBump(["patch", "none", "patch"]) === "patch", "patch wins over none");
  assert(highestBump(["patch", "minor"]) === "minor", "minor wins over patch");
  assert(highestBump(["none"]) === "none", "nothing declared");
  const assembled = assembleChangelog("0.5.0", "2026-11-18", [
    { bump: "minor", body: "### Added\n\n- A capability.\n- Another one.\n" },
    { bump: "none", body: "### Internal\n\n- A refactor nobody sees.\n" },
  ]);
  assert(assembled.startsWith("## [0.5.0] - 2026-11-18"), "the heading is written");
  assert(assembled.includes("### Added\n\n- A capability.\n- Another one."), "bullets land under their section");
  assert(!assembled.includes("A refactor nobody sees"), "a none fragment never reaches the changelog");
  assert(assembled.endsWith("\n\n"), "a blank line separates it from the next release");

  assert(isRevisionArgument("origin/main"), "a normal base is accepted");
  assert(
    isRevisionArgument("origin/release/0.4+maintenance"),
    "git allows + in a ref, and so does this",
  );
  assert(!isRevisionArgument("--upload-pack=touch"), "a leading dash is never a revision");
  assert(!isRevisionArgument(""), "an empty base is not a revision");

  assert(isCalendarDate("2026-11-17"), "a real date is accepted");
  assert(!isCalendarDate("2026-02-31"), "a date that does not exist is refused");
  assert(!isCalendarDate("2026-13-01"), "a month past December is refused");
  assert(!isCalendarDate("2026-1-7"), "the shape is strict");
  for (const relative of VERSION_FILES) {
    assert(existsSync(join(ROOT, relative)), `${relative} carries a version`);
  }

  const good = parseFragment(
    "---\nbump: patch\n---\n\n### Fixed\n\n- The thing stops doing the wrong thing.\n",
  );
  assert(good.bump === "patch", "fragment bump");
  assert(good.body.includes("The thing"), "fragment body");

  const rejects = [
    ["---\nbump: major\n---\n\n### Fixed\n\n- x\n", "unknown bump"],
    ["---\nbump: patch\n---\n\nno heading\n- x\n", "missing heading"],
    ["---\nbump: patch\n---\n\n### Fixed\n\nno bullet\n", "missing bullet"],
    ["---\nbump: none\n---\n\n### Fixed\n\n- x\n", "none cannot ship a Fixed section"],
    ["---\nbump: patch\n---\n\n### Internal\n\n- x\n", "Internal cannot ship a bump"],
    ["---\nbump: patch\n---\n\nIntro first.\n\n### Fixed\n\n- x\n", "heading must be first"],
    ["---\nbump: patch\n---\n\n### Fixed\n\n- x\n\n### Added\n\n- y\n", "one heading only"],
    ["\n### Fixed\n\n- x\n", "missing frontmatter"],
  ];
  for (const [raw, why] of rejects) {
    let threw = false;
    try {
      parseFragment(raw);
    } catch {
      threw = true;
    }
    assert(threw, `should reject: ${why}`);
  }

  assert(isProductPath("crates/core-api/src/routes/rag.rs"), "route is product");
  assert(isProductPath("apps/web/app/billing/page.tsx"), "page is product");
  assert(!isProductPath("crates/core-api/tests/flow.rs"), "test crate is not product");
  assert(isProductPath("apps/web/components/auth/entry.css"), "css under apps is product");
  assert(!isProductPath("scripts/release-note.mjs"), "tooling is not product");
  assert(!isProductPath("packages/ui/package.json"), "a manifest is not behaviour");
  assert(!isProductPath("Cargo.toml"), "the version manifest is not behaviour");
  assert(!isProductPath("native/app.json"), "a shell manifest is not behaviour");
  assert(
    !isProductPath("desktop/tauri.conf.json"),
    "the desktop manifest is not behaviour",
  );
  assert(isProductPath("native/App.tsx"), "a native source file is still product");
  assert(!isProductPath("docs/site/reference/api.md"), "docs are not product");
  assert(!isProductPath("e2e/flows/01-auth.spec.ts"), "e2e is not product");

  const minor = advisePaths(["crates/core-api/migrations/0187_x.sql"]);
  assert(minor.bump === "minor", "a migration suggests minor");
  const patch = advisePaths(["crates/core-api/src/auth/cookies.rs"]);
  assert(patch.bump === "patch", "a fix module suggests patch");
  const none = advisePaths(["docs/adr/0034.md", "scripts/x.mjs"]);
  assert(none.bump === "none", "docs-only suggests none");

  console.log("release-note: self-test passed (versions, fragments, paths, advice)");
}

// --- entry ------------------------------------------------------------------

const invokedDirectly =
  process.argv[1] !== undefined &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  const args = process.argv.slice(2);
  if (args.includes("--self-test")) {
    selfTest();
  } else {
    const { command, flags } = parseArgs(args);
    switch (command) {
      case "add":
        cmdAdd(flags);
        break;
      case "list":
        cmdList();
        break;
      case "plan":
        cmdPlan();
        break;
      case "apply":
        cmdApply(flags);
        break;
      case "check":
        cmdCheck(flags);
        break;
      case "advise":
        cmdAdvise(flags);
        break;
      default:
        usage(command === undefined ? 1 : 2);
    }
  }
}
