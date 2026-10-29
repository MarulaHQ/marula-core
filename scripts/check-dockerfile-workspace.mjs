#!/usr/bin/env node
// check-dockerfile-workspace.mjs — every Cargo workspace member must reach
// deploy/Dockerfile.rust's build stage.
//
// Why: cargo resolves the whole workspace graph before it compiles anything, so
// a member whose Cargo.toml was never COPY'd into the build context stops the
// image build at "failed to load manifest for workspace member". `installer`
// joined the workspace and the Dockerfile was never told; no job built the
// images, so nothing went red until someone built from a clean context. This
// check owns that class of drift and runs in milliseconds, with no Docker.
//
// Usage:
//   node scripts/check-dockerfile-workspace.mjs              verify the tree
//   node scripts/check-dockerfile-workspace.mjs --self-test  prove the checks
//
// Fails when:
//   - a workspace member's Cargo.toml is not covered by any COPY source;
//   - a COPY source does not exist in the repo (a rename the Dockerfile missed);
//   - .dockerignore excludes a member's directory (the COPY would then fail).
//
// Limits (deliberate): wildcards in COPY sources and .dockerignore patterns are
// not expanded; a source containing * or ? is skipped for the existence check.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

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

/** Sources of every non-`--from` COPY instruction (flags and destination dropped). */
export function parseCopySources(dockerfile) {
  const out = [];
  const logical = dockerfile.replace(/\\\n/g, " ").split("\n");
  for (const line of logical) {
    const t = line.trim();
    if (!/^COPY\s/i.test(t)) continue;
    const args = t.split(/\s+/).slice(1);
    const flags = args.filter((a) => a.startsWith("--"));
    if (flags.some((f) => f.startsWith("--from="))) continue;
    const paths = args.filter((a) => !a.startsWith("--"));
    for (const src of paths.slice(0, -1)) out.push(norm(src));
  }
  return out;
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

/** Pure check: returns a list of error strings. */
export function check({ members, copySources, ignore, exists }) {
  const errors = [];
  for (const member of members) {
    const manifest = `${member}/Cargo.toml`;
    const covered = copySources.some(
      (src) => src === "." || src === manifest || manifest.startsWith(`${src}/`),
    );
    if (!covered) {
      errors.push(`workspace member "${member}": ${manifest} is not covered by any COPY in Dockerfile.rust`);
    }
    const ignored = ignore.find((pat) => pat === member || manifest.startsWith(`${pat}/`) || pat === manifest);
    if (ignored) {
      errors.push(`workspace member "${member}" is excluded by .dockerignore entry "${ignored}"`);
    }
  }
  for (const src of copySources) {
    if (src === "." || /[*?]/.test(src)) continue;
    if (!exists(src)) errors.push(`COPY source "${src}" does not exist in the repository`);
  }
  return errors;
}

function selfTest() {
  const base = {
    members: ["desktop", "installer", "crates/core-api", "plugins/connectors/unifi"],
    copySources: ["Cargo.toml", "crates", "plugins", "desktop/Cargo.toml", "installer/Cargo.toml"],
    ignore: [],
    exists: () => true,
  };
  const cases = [
    ["clean tree passes", base, 0],
    ["missing installer COPY is caught", { ...base, copySources: base.copySources.filter((s) => s !== "installer/Cargo.toml") }, 1],
    ["parent-dir COPY covers nested members", { ...base, copySources: ["crates", "plugins", "desktop", "installer"] }, 0],
    ["ignored member dir is caught", { ...base, ignore: ["installer"] }, 1],
    ["missing COPY source is caught", { ...base, exists: (p) => p !== "plugins" }, 1],
  ];
  let failed = 0;
  for (const [name, input, want] of cases) {
    const got = check(input).length;
    if (got !== want) {
      failed++;
      console.error(`SELF-TEST FAIL: ${name} (wanted ${want} errors, got ${got})`);
    }
  }
  const parsed = parseMembers('[workspace]\nmembers = [\n  "a", # note\n  # "gone",\n  "b/c",\n]\n');
  if (parsed.join() !== "a,b/c") {
    failed++;
    console.error(`SELF-TEST FAIL: member parsing gave ${parsed.join()}`);
  }
  const copies = parseCopySources("COPY --from=build /out/x /y\nCOPY a b /dst/\nCOPY --chown=1:1 c d\n");
  if (copies.join() !== "a,b,c") {
    failed++;
    console.error(`SELF-TEST FAIL: COPY parsing gave ${copies.join()}`);
  }
  if (failed) process.exit(1);
  console.log("self-test ok");
}

if (process.argv.includes("--self-test")) {
  selfTest();
} else {
  const read = (p) => readFileSync(join(root, p), "utf8");
  const members = parseMembers(read("Cargo.toml"));
  const copySources = parseCopySources(read("deploy/Dockerfile.rust"));
  const ignore = existsSync(join(root, ".dockerignore")) ? parseIgnore(read(".dockerignore")) : [];
  const errors = check({ members, copySources, ignore, exists: (p) => existsSync(join(root, p)) });
  if (errors.length) {
    for (const e of errors) console.error(`FAIL ${e}`);
    process.exit(1);
  }
  console.log(`ok: ${members.length} workspace members reach the Rust image build`);
}
