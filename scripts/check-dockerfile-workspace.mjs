#!/usr/bin/env node
// check-dockerfile-workspace.mjs — the build contexts the image Dockerfiles
// COPY must still contain everything their builds read.
//
// Why: both images build from a hand-listed subset of the repository, and a
// miss only shows when someone builds from a clean context.
//   * Rust: cargo resolves the whole workspace graph before compiling, so a
//     member whose Cargo.toml was never COPY'd stops the build at "failed to
//     load manifest for workspace member". `installer` joined the workspace
//     and the Dockerfile was never told.
//   * Web: deploy/Dockerfile.web COPYs apps/web, three packages and
//     LEGAL/fonts instead of `COPY . .`. An import that reaches outside that
//     set (app/layout.tsx pulls its fonts from LEGAL/fonts) fails the build,
//     and a new workspace dependency needs its manifest before `pnpm install`.
// No job builds these images, so nothing else would notice. This runs in
// milliseconds with no Docker.
//
// Usage:
//   node scripts/check-dockerfile-workspace.mjs              verify the tree
//   node scripts/check-dockerfile-workspace.mjs --self-test  prove the checks
//   npm run docker:check                                     both of the above
//
// Rust (deploy/Dockerfile.rust) fails when:
//   - a workspace member's Cargo.toml is not covered by any COPY source;
//   - a COPY source does not exist in the repo (a rename the Dockerfile missed);
//   - .dockerignore excludes a member's directory.
// Web (deploy/Dockerfile.web) fails when:
//   - a workspace package apps/web depends on (transitively, via `workspace:`)
//     has no package.json COPY'd before the `pnpm install` step, or its
//     source is never COPY'd;
//   - a non-test source file in apps/web or those packages references a repo
//     path outside its own package that no COPY covers;
//   - a COPY source does not exist.
//
// Limits (deliberate): wildcards in COPY sources and .dockerignore patterns are
// not expanded. The web path scan reads string literals and CSS url()s that
// contain `../` and resolve to an existing repo path; paths assembled at
// runtime are invisible to it. Test, spec and e2e files are not scanned.

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// ---------------------------------------------------------------- parsing

/** Workspace member paths from the root Cargo.toml (`members = [ ... ]`). */
export function parseMembers(cargoToml) {
  const m = cargoToml.match(/members\s*=\s*\[([\s\S]*?)\]/);
  if (!m) throw new Error("no `members = [...]` array in Cargo.toml");
  const body = m[1]
    .split("\n")
    .map((l) => l.replace(/#.*$/, ""))
    .join("\n");
  return [...body.matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}

/** Ordered COPY (non-`--from`) and RUN instructions of a Dockerfile. */
export function parseInstructions(dockerfile) {
  const out = [];
  for (const line of dockerfile.replace(/\\\n/g, " ").split("\n")) {
    const t = line.trim();
    if (/^RUN\s/i.test(t)) {
      out.push({ op: "RUN", text: t.slice(4) });
    } else if (/^COPY\s/i.test(t)) {
      const args = t.split(/\s+/).slice(1);
      if (args.some((a) => a.startsWith("--from="))) continue;
      const paths = args.filter((a) => !a.startsWith("--"));
      out.push({ op: "COPY", sources: paths.slice(0, -1).map(norm) });
    }
  }
  return out;
}

/** Sources of every non-`--from` COPY instruction. */
export function parseCopySources(dockerfile) {
  return parseInstructions(dockerfile).flatMap((i) => (i.op === "COPY" ? i.sources : []));
}

export function parseIgnore(text) {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#") && !l.startsWith("!"))
    .map(norm);
}

function norm(p) {
  return p.replace(/^\.\//, "").replace(/\/+$/, "");
}

const covers = (src, rel) => src === "." || src === rel || rel.startsWith(`${src}/`);

// ------------------------------------------------------------------ rust

/** Pure check: returns a list of error strings. */
export function checkRust({ members, copySources, ignore, exists }) {
  const errors = [];
  for (const member of members) {
    const manifest = `${member}/Cargo.toml`;
    if (!copySources.some((src) => covers(src, manifest))) {
      errors.push(`rust: workspace member "${member}": ${manifest} is not covered by any COPY in Dockerfile.rust`);
    }
    const ignored = ignore.find((pat) => pat === member || manifest.startsWith(`${pat}/`) || pat === manifest);
    if (ignored) {
      errors.push(`rust: workspace member "${member}" is excluded by .dockerignore entry "${ignored}"`);
    }
  }
  for (const src of copySources) {
    if (src === "." || /[*?]/.test(src)) continue;
    if (!exists(src)) errors.push(`rust: COPY source "${src}" does not exist in the repository`);
  }
  return errors;
}

// ------------------------------------------------------------------- web

const ROOT_MANIFESTS = ["package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml"];

/**
 * Pure check. `dirs` is apps/web plus its transitive workspace deps;
 * `escapes` are repo-relative paths the scanned sources reference outside
 * their own package.
 */
export function checkWeb({ dirs, instructions, escapes, exists }) {
  const errors = [];
  const installAt = instructions.findIndex((i) => i.op === "RUN" && /pnpm/.test(i.text) && /\binstall\b/.test(i.text));
  if (installAt < 0) errors.push("web: no `pnpm ... install` RUN found in Dockerfile.web");
  const beforeInstall = installAt < 0 ? [] : instructions.slice(0, installAt).flatMap((i) => (i.op === "COPY" ? i.sources : []));
  const all = instructions.flatMap((i) => (i.op === "COPY" ? i.sources : []));

  for (const f of ROOT_MANIFESTS) {
    if (!beforeInstall.some((s) => covers(s, f))) errors.push(`web: ${f} is not COPY'd before pnpm install`);
  }
  for (const dir of dirs) {
    const manifest = `${dir}/package.json`;
    if (!beforeInstall.some((s) => covers(s, manifest))) {
      errors.push(`web: ${manifest} is not COPY'd before pnpm install, so the workspace link cannot resolve`);
    }
    if (!all.some((s) => covers(s, dir))) {
      errors.push(`web: ${dir} source is never COPY'd into the build`);
    }
  }
  for (const rel of escapes) {
    if (!all.some((s) => covers(s, rel))) {
      errors.push(`web: the build reads "${rel}", which no COPY in Dockerfile.web covers`);
    }
  }
  for (const src of all) {
    if (src === "." || /[*?]/.test(src)) continue;
    if (!exists(src)) errors.push(`web: COPY source "${src}" does not exist in the repository`);
  }
  return errors;
}

const SKIP_DIRS = new Set(["node_modules", ".next", ".turbo", "coverage", "tests", "test", "__tests__", "e2e"]);
const SCAN_EXT = /\.(?:[cm]?[jt]sx?|css)$/;
const TEST_FILE = /\.(?:test|spec)\./;
const REL_SPEC = /["'`(]((?:\.\.\/)+[^"'`)\s]*)/g;

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) yield* walk(p);
    else if (SCAN_EXT.test(name) && !TEST_FILE.test(name)) yield p;
  }
}

function existsLoose(rel) {
  if (existsSync(join(root, rel))) return true;
  return ["", ".ts", ".tsx", ".js", ".mjs", ".cjs", ".json", "/index.ts", "/index.tsx", "/index.js"].some(
    (ext) => existsSync(join(root, rel + ext)),
  );
}

/** Repo-relative paths a package's non-test sources reach outside the package. */
function scanEscapes(dirs) {
  const found = new Set();
  for (const dir of dirs) {
    const pkgRoot = join(root, dir);
    for (const file of walk(pkgRoot)) {
      const text = readFileSync(file, "utf8");
      for (const m of text.matchAll(REL_SPEC)) {
        const target = resolve(dirname(file), m[1]);
        if (target === pkgRoot || target.startsWith(pkgRoot + sep)) continue;
        const rel = relative(root, target).split(sep).join("/");
        // An empty relative path means the reference resolved to the repository
        // root itself — `outputFileTracingRoot: path.join(import.meta.dirname,
        // "../..")` in apps/web/next.config.mjs is one. The root is not a file a
        // COPY has to cover; reporting it as an uncovered read produced the
        // useless message `the build reads ""`.
        if (rel === "") continue;
        if (rel.startsWith("..") || rel.split("/").includes("node_modules")) continue;
        if (existsLoose(rel)) found.add(rel);
      }
    }
  }
  return [...found].sort();
}

/** apps/web plus every package it reaches through `workspace:` dependencies. */
function workspaceClosure(start) {
  const byName = new Map();
  for (const parent of ["apps", "packages"]) {
    if (!existsSync(join(root, parent))) continue;
    for (const d of readdirSync(join(root, parent))) {
      const pj = join(root, parent, d, "package.json");
      if (existsSync(pj)) byName.set(JSON.parse(readFileSync(pj, "utf8")).name, `${parent}/${d}`);
    }
  }
  const seen = new Set();
  const queue = [start];
  while (queue.length) {
    const dir = queue.shift();
    if (seen.has(dir)) continue;
    seen.add(dir);
    const pj = JSON.parse(readFileSync(join(root, dir, "package.json"), "utf8"));
    for (const field of ["dependencies", "devDependencies", "optionalDependencies"]) {
      for (const [name, range] of Object.entries(pj[field] ?? {})) {
        if (String(range).startsWith("workspace:") && byName.has(name)) queue.push(byName.get(name));
      }
    }
  }
  return [...seen].sort();
}

// ------------------------------------------------------------- self-test

function selfTest() {
  let failed = 0;
  const expect = (name, got, want) => {
    if (got !== want) {
      failed++;
      console.error(`SELF-TEST FAIL: ${name} (wanted ${want}, got ${got})`);
    }
  };

  const rust = {
    members: ["desktop", "installer", "crates/core-api", "plugins/connectors/unifi"],
    copySources: ["Cargo.toml", "crates", "plugins", "desktop/Cargo.toml", "installer/Cargo.toml"],
    ignore: [],
    exists: () => true,
  };
  expect("rust: clean tree passes", checkRust(rust).length, 0);
  expect("rust: missing installer COPY is caught", checkRust({ ...rust, copySources: rust.copySources.filter((s) => s !== "installer/Cargo.toml") }).length, 1);
  expect("rust: parent-dir COPY covers nested members", checkRust({ ...rust, copySources: ["crates", "plugins", "desktop", "installer"] }).length, 0);
  expect("rust: ignored member dir is caught", checkRust({ ...rust, ignore: ["installer"] }).length, 1);
  expect("rust: missing COPY source is caught", checkRust({ ...rust, exists: (p) => p !== "plugins" }).length, 1);

  const ins = parseInstructions(
    [
      "COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./",
      "COPY apps/web/package.json apps/web/",
      "COPY packages/ui/package.json packages/ui/",
      "RUN --mount=type=cache,target=/s pnpm --filter x... install --frozen-lockfile",
      "COPY apps/web apps/web",
      "COPY packages/ui packages/ui",
      "COPY LEGAL/fonts LEGAL/fonts",
    ].join("\n"),
  );
  const web = { dirs: ["apps/web", "packages/ui"], instructions: ins, escapes: ["LEGAL/fonts/a.woff2"], exists: () => true };
  expect("web: clean tree passes", checkWeb(web).length, 0);
  expect("web: new workspace dep without manifest is caught", checkWeb({ ...web, dirs: [...web.dirs, "packages/new"] }).length, 2);
  expect("web: uncovered escape path is caught", checkWeb({ ...web, escapes: ["LEGAL/fonts/a.woff2", "scripts/x.mjs"] }).length, 1);
  expect("web: manifest COPY after install is caught", checkWeb({ ...web, instructions: [...ins.slice(3), ...ins.slice(0, 3)] }).length > 0, true);
  expect("web: missing install RUN is caught", checkWeb({ ...web, instructions: ins.filter((i) => i.op !== "RUN") }).length > 0, true);

  expect("parse: members", parseMembers('[workspace]\nmembers = [\n  "a", # note\n  # "gone",\n  "b/c",\n]\n').join(), "a,b/c");
  expect("parse: COPY sources", parseCopySources("COPY --from=build /out/x /y\nCOPY a b /dst/\nCOPY --chown=1:1 c d\n").join(), "a,b,c");

  if (failed) process.exit(1);
  console.log("self-test ok");
}

// ------------------------------------------------------------------ main

if (process.argv.includes("--self-test")) {
  selfTest();
} else {
  const read = (p) => readFileSync(join(root, p), "utf8");
  const exists = (p) => existsSync(join(root, p));
  const ignore = exists(".dockerignore") ? parseIgnore(read(".dockerignore")) : [];

  const members = parseMembers(read("Cargo.toml"));
  const errors = checkRust({ members, copySources: parseCopySources(read("deploy/Dockerfile.rust")), ignore, exists });

  const dirs = workspaceClosure("apps/web");
  const escapes = scanEscapes(dirs);
  errors.push(...checkWeb({ dirs, instructions: parseInstructions(read("deploy/Dockerfile.web")), escapes, exists }));

  if (errors.length) {
    for (const e of errors) console.error(`FAIL ${e}`);
    process.exit(1);
  }
  console.log(`ok: ${members.length} workspace members reach the Rust image; web closure ${dirs.join(", ")} (+${escapes.length} outside paths) covered`);
}
