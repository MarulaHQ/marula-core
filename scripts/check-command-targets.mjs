#!/usr/bin/env node
/**
 * Command-target check (Wave 47 lane A25, p47-silent-command-noops).
 *
 * Every control a keyboard command clicks is declared on its registry row
 * (`targets: [testid("new-record")]`, see apps/web/lib/commands/targets.ts).
 * This script resolves each declaration against the source tree: a command
 * that clicks `[data-testid="new-record"]` passes only while some page or
 * component actually renders `data-testid="new-record"`.
 *
 * Why it exists: the Wave 41 pack port renamed the clients page's create
 * button. Nothing failed. `new.clients` clicked a selector that matched
 * nothing, `ctx.click` returned a boolean nobody read, and Alt+N stayed in the
 * overlay and the cheat sheet doing nothing for months. A rename now turns
 * this check red in the same commit that made it.
 *
 * Scope: declarations in apps/web/lib/commands; markup in apps/web and
 * packages. Tests, stories and build output are skipped — a testid that only
 * a test file mentions is not a control a user can click.
 *
 * Read-only, no browser, no build: it runs on the light path.
 * Usage: node scripts/check-command-targets.mjs
 *        node scripts/check-command-targets.mjs --self-test
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DECLARATION_DIRS = ["apps/web/lib/commands"];
const SOURCE_ROOTS = ["apps/web", "packages"];
const SKIP_DIRS = new Set([
  "node_modules",
  ".next",
  "dist",
  "coverage",
  ".turbo",
  "tests",
  "__snapshots__",
]);
const SKIP_FILE = /\.test\.[cm]?[jt]sx?$/;

/** Both declaration forms live in `targets.ts` calls: testid("x") / attr("a","b"). */
const TESTID_DECL = /\btestid\(\s*"([^"]+)"/g;
const ATTR_DECL = /\battr\(\s*"([^"]+)"\s*,\s*"([^"]+)"/g;

function walk(dir, out = []) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue;
      walk(join(dir, entry.name), out);
    } else if (/\.(tsx?|jsx?|mjs)$/.test(entry.name) && !SKIP_FILE.test(entry.name)) {
      out.push(join(dir, entry.name));
    }
  }
  return out;
}

/** Every target declaration, in file order: what the registry promises. */
function declarations() {
  const found = [];
  for (const dir of DECLARATION_DIRS) {
    for (const file of walk(resolve(root, dir))) {
      const text = readFileSync(file, "utf8");
      const where = relative(root, file);
      for (const m of text.matchAll(TESTID_DECL)) {
        found.push({ kind: "testid", name: m[1], value: null, where });
      }
      for (const m of text.matchAll(ATTR_DECL)) {
        found.push({ kind: "attr", name: m[1], value: m[2], where });
      }
    }
  }
  return found;
}

const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** The markup forms that render an attribute: JSX string, or a braced literal. */
function markupPattern(attribute, value) {
  const v = escape(value);
  return new RegExp(`${escape(attribute)}=(?:"${v}"|'${v}'|\\{"${v}"\\}|\\{'${v}'\\})`);
}

/** Resolve declarations against a list of `{path, text}` files. */
function audit(decls, files) {
  const results = [];
  for (const decl of decls) {
    const attribute = decl.kind === "testid" ? "data-testid" : decl.name;
    const value = decl.kind === "testid" ? decl.name : decl.value;
    const pattern = markupPattern(attribute, value);
    const hits = files.filter((f) => pattern.test(f.text)).map((f) => relative(root, f.path));
    results.push({
      decl,
      hits,
      selector: `[${attribute}="${value}"]`,
    });
  }
  return results;
}

function loadSources() {
  const files = [];
  for (const scope of SOURCE_ROOTS) {
    for (const file of walk(resolve(root, scope))) {
      files.push({ path: file, text: readFileSync(file, "utf8") });
    }
  }
  return files;
}

function printReport(results, fileCount) {
  const width = Math.max(...results.map((r) => r.decl.name.length), 12);
  for (const { decl, hits, selector } of results) {
    if (hits.length > 0) {
      const places = hits.length === 1 ? hits[0] : `${hits[0]} (+${hits.length - 1} more)`;
      console.log(`  ok   ${decl.kind.padEnd(6)} ${decl.name.padEnd(width)}  ${places}`);
    } else {
      console.log(
        `  FAIL ${decl.kind.padEnd(6)} ${decl.name.padEnd(width)}  declared in ${decl.where}, ` +
          `but no source file renders ${selector}`,
      );
    }
  }
  const misses = results.filter((r) => r.hits.length === 0);
  console.log(
    `\n${results.length - misses.length}/${results.length} command targets resolve in source ` +
      `(${fileCount} files scanned).`,
  );
  return misses.length === 0;
}

/**
 * Checker self-test: for every declaration, rename its control in memory and
 * prove the audit notices. This is the `new-record` → `new-client` rename,
 * rehearsed on every declaration so a broken matcher cannot pass quietly.
 */
function selfTest(results) {
  const files = loadSources();
  let checked = 0;
  const failures = [];
  for (const { decl } of results) {
    const attribute = decl.kind === "testid" ? "data-testid" : decl.name;
    const value = decl.kind === "testid" ? decl.name : decl.value;
    const pattern = markupPattern(attribute, value);
    const renamed = value === "" ? "renamed" : `${value}-renamed`;
    const mutated = files.map((f) =>
      pattern.test(f.text)
        ? { path: f.path, text: f.text.replace(new RegExp(pattern.source, "g"), `${attribute}="${renamed}"`) }
        : f,
    );
    // Only this declaration should go missing once its control is renamed.
    const after = audit([decl], mutated)[0];
    checked += 1;
    if (after.hits.length !== 0) failures.push(decl.name);
  }
  if (failures.length > 0) {
    console.error(`✗ self-test failed — a rename went unnoticed for: ${failures.join(", ")}`);
    return 1;
  }
  console.log(`self-test ok — ${checked} target${checked === 1 ? "" : "s"}, every rename detected.`);
  return 0;
}

const decls = declarations();
if (decls.length === 0) {
  console.error(
    "✗ no command-target declarations found in " +
      `${DECLARATION_DIRS.join(", ")} — the checker cannot see the registry (did the declaration form change?)`,
  );
  process.exit(1);
}
const results = audit(decls, loadSources());
const files = loadSources();

if (process.argv.includes("--self-test")) {
  process.exit(selfTest(results));
}

console.log(`command targets — ${decls.length} declaration(s) from ${DECLARATION_DIRS.join(", ")}`);
const clean = printReport(results, files.length);
if (!clean) {
  console.error(
    "\nA command clicks a control that no page or component renders. Either the control was " +
      "renamed/deleted (restore it or update the command's target) — a dead chord is still " +
      "advertised in the overlay and the cheat sheet.",
  );
  process.exit(1);
}
