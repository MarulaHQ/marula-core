#!/usr/bin/env node
// Checks docs/agents/manifest.json against the API it documents (Wave 52 lane
// R8b).
//
// Purpose: the manifest is data an agent reads to learn the surface, so it must
// not describe a surface that has moved. Two failures matter and this reports
// both by name:
//
//   1. GONE      the manifest documents a namespace the spec no longer exposes
//   2. UNDOCUMENTED  the spec exposes a namespace the manifest does not describe
//
// The namespace set is derived the same way the adapter derives it, because a
// check that invents its own rule would pass while the catalog disagreed with
// both of them: one tool per GET with an operationId, excluded prefixes removed,
// then the first path segment after the version prefix. The exclusion list is
// read out of `crates/mcp-server/src/lib.rs` rather than copied here, so the
// adapter and this check cannot drift apart.
//
// It also fails when the manifest points at a file that is gone, which is how a
// renamed route module or a deleted extension-point page gets caught.
//
// Usage:
//   node scripts/check-agent-manifest.mjs
//   node scripts/check-agent-manifest.mjs --json
//
// Non-purpose: it does not check that a grant exists for a namespace, and it
// does not call the API. Grants are runtime data, and the catalog's own
// `filter_granted` is the authority on them.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST = "docs/agents/manifest.json";
const ADAPTER = "crates/mcp-server/src/lib.rs";

const read = (rel) => readFileSync(join(root, rel), "utf8");
const problems = [];
const fail = (rule, message) => problems.push(`${rule} ${message}`);

const manifest = JSON.parse(read(MANIFEST));

// --- the exclusion list, read from the adapter ------------------------------

function excludedPrefixes() {
  const src = read(ADAPTER);
  const block = src.match(
    /const EXCLUDED_PATH_PREFIXES: &\[&str\] = &\[([\s\S]*?)\];/,
  );
  if (!block) {
    throw new Error(`cannot find EXCLUDED_PATH_PREFIXES in ${ADAPTER}`);
  }
  const list = [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  if (list.length === 0) {
    throw new Error("EXCLUDED_PATH_PREFIXES parsed as empty — refusing to check");
  }
  return list;
}

// --- the surface the spec describes -----------------------------------------

/** The namespace a path derives, mirroring `capability_of`. */
function capabilityOf(path) {
  if (!path.startsWith("/api/")) return null;
  const rest = path.slice("/api/".length);
  const unversioned = rest.startsWith("v1/") ? rest.slice(3) : rest;
  const segment = unversioned.split("/")[0];
  return segment === "" ? null : segment;
}

function derivedNamespaces(spec, excluded) {
  const isExcluded = (path) =>
    excluded.some((prefix) => path.startsWith(prefix)) || path.includes("/attachments");

  const namespaces = new Map();
  let tools = 0;
  for (const [path, item] of Object.entries(spec.paths ?? {})) {
    const get = item.get;
    if (!get || typeof get.operationId !== "string") continue;
    if (isExcluded(path)) continue;
    const namespace = capabilityOf(path);
    if (!namespace) continue;
    namespaces.set(namespace, (namespaces.get(namespace) ?? 0) + 1);
    tools += 1;
  }
  return { namespaces, tools };
}

// --- 1/2. namespaces, both directions ---------------------------------------

const excluded = excludedPrefixes();
const spec = JSON.parse(read(manifest.tool_derivation.spec));
const { namespaces: live, tools } = derivedNamespaces(spec, excluded);

const documented = manifest.namespaces ?? [];
const documentedSet = new Set(documented);

if (documentedSet.size !== documented.length) {
  fail("DUPLICATE", "the namespace list repeats an entry");
}
const sorted = [...documented].sort();
if (sorted.join(",") !== documented.join(",")) {
  fail("ORDER", "the namespace list is not sorted, which makes the diff unreadable");
}

for (const namespace of documented) {
  if (!live.has(namespace)) {
    fail("GONE", `${namespace} is documented but /api/ no longer exposes it`);
  }
}
for (const namespace of live.keys()) {
  if (!documentedSet.has(namespace)) {
    fail("UNDOCUMENTED", `${namespace} is exposed by the spec and missing from the manifest`);
  }
}

// --- 3. sensitive namespaces ------------------------------------------------

for (const [namespace, why] of Object.entries(manifest.sensitive ?? {})) {
  if (!documentedSet.has(namespace)) {
    fail("SENSITIVE", `${namespace} is flagged sensitive and is not in the namespace list`);
  }
  if (!live.has(namespace)) {
    fail("SENSITIVE", `${namespace} is flagged sensitive and the spec no longer exposes it`);
  }
  if (typeof why !== "string" || why.trim().length < 20) {
    fail("SENSITIVE", `${namespace} carries no usable reason`);
  }
}

// --- 4. every pointer resolves ----------------------------------------------

const pointer = (rule, label, value) => {
  if (typeof value !== "string" || value.length === 0) {
    fail(rule, `${label} is missing`);
    return;
  }
  const path = value.split(":")[0];
  if (!path.endsWith(".json") && !path.endsWith(".md") && !path.endsWith(".rs")) {
    fail(rule, `${label} does not name a file: ${value}`);
    return;
  }
  if (!existsSync(join(root, path))) {
    fail(rule, `${label} points at ${path}, which does not exist`);
  }
};

for (const [id, path] of Object.entries(manifest.docs ?? {})) pointer("DOC", `docs.${id}`, path);
for (const surface of manifest.surfaces ?? []) pointer("SURFACE", `surfaces.${surface.id}`, surface.source);
for (const rule of manifest.prohibitions ?? []) pointer("PROHIBITION", rule.id, rule.enforced_by);
pointer("DERIVATION", "tool_derivation.spec", manifest.tool_derivation.spec);
pointer("DERIVATION", "tool_derivation.adapter", manifest.tool_derivation.adapter);
pointer("DERIVATION", "tool_derivation.excluded_prefixes_source", manifest.tool_derivation.excluded_prefixes_source);

// --- 5. the prohibitions say something --------------------------------------

for (const rule of manifest.prohibitions ?? []) {
  if (!rule.id || !rule.statement) fail("PROHIBITION", "an entry is missing an id or a statement");
  if (typeof rule.statement === "string" && rule.statement.trim().length < 30) {
    fail("PROHIBITION", `${rule.id} states too little to act on`);
  }
}

// --- report -----------------------------------------------------------------

if (process.argv.includes("--json")) {
  console.log(
    JSON.stringify(
      {
        ok: problems.length === 0,
        namespaces: documented.length,
        tools,
        problems,
      },
      null,
      2,
    ),
  );
} else if (problems.length) {
  for (const problem of problems) console.error(problem);
  console.error(`\n${problems.length} problem(s) in ${MANIFEST}`);
} else {
  console.log(
    `agent manifest OK: ${documented.length} namespaces documented and all present, ${tools} tools derived, every pointer resolves`,
  );
}

process.exit(problems.length === 0 ? 0 : 1);
