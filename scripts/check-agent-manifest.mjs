#!/usr/bin/env node
/**
 * check-agent-manifest.mjs — checks `docs/agents/manifest.json` against the API
 * it documents (Wave 52 lane A1, updated 2026-11-05 after the MCP catalog moved
 * to a derived artifact).
 *
 * Purpose: the manifest is data an agent reads to learn the surface, so it must
 * not drift from the tree. Three checks, strongest first:
 *
 *   1. GONE / UNDOCUMENTED — a documented namespace is absent from the spec, or
 *      an agent-relevant namespace in the spec is undocumented.
 *   2. ARTIFACT — the MCP read surface (`packages/sdk/mcp-catalog.json`) is
 *      derived from the same spec and every family it publishes is documented.
 *      Its tools are counted here, not re-derived from the spec.
 *   3. SENSITIVE — every namespace the manifest calls sensitive exists and
 *      carries a usable reason.
 *
 * The namespace exclusion list lives in the manifest's `tool_derivation`, not in
 * a Rust constant. It used to be parsed out of `mcp-server/src/lib.rs`; the MCP
 * catalog moved to `packages/sdk/mcp-catalog.json`, generated from the policy in
 * `packages/sdk/mcp-catalog.config.json`, and the list moved with the surface.
 *
 * It also fails when a pointer names a file that is gone, which is how a
 * manifest rots silently after a move.
 *
 *   node scripts/check-agent-manifest.mjs
 *   node scripts/check-agent-manifest.mjs --json
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST = "docs/agents/manifest.json";

const read = (rel) => readFileSync(join(root, rel), "utf8");
const problems = [];
const fail = (rule, message) => problems.push(`${rule} ${message}`);

const manifest = JSON.parse(read(MANIFEST));
const derivation = manifest.tool_derivation ?? {};
const SPEC = derivation.spec;
const ADAPTER = derivation.adapter;
const ARTIFACT = derivation.artifact;
const POLICY = derivation.policy;

// --- the agent-relevant namespace policy, read from the manifest ------------

function namespaceExclusions() {
  const list = derivation.namespace_exclusions;
  if (!Array.isArray(list) || list.length === 0) {
    throw new Error(
      "tool_derivation.namespace_exclusions is missing or empty — refusing to check",
    );
  }
  for (const prefix of list) {
    if (typeof prefix !== "string" || !prefix.startsWith("/api/")) {
      throw new Error(`namespace exclusion is not an /api/ prefix: ${prefix}`);
    }
  }
  return list;
}

/** The namespace a path derives, mirroring the catalog's `familyOf`. */
function capabilityOf(path) {
  if (!path.startsWith("/api/")) return null;
  const rest = path.slice("/api/".length);
  const unversioned = rest.startsWith("v1/") ? rest.slice(3) : rest;
  const segment = unversioned.split("/")[0];
  return segment === "" ? null : segment;
}

function specNamespaces(spec, excluded) {
  const isExcluded = (path) =>
    excluded.some((prefix) => {
      const unversioned = prefix.replace("/api/v1", "/api");
      return path.startsWith(prefix) || path.startsWith(unversioned);
    });

  const namespaces = new Set();
  let operations = 0;
  for (const [path, item] of Object.entries(spec.paths ?? {})) {
    const get = item.get;
    if (!get || typeof get.operationId !== "string") continue;
    if (isExcluded(path)) continue;
    const namespace = capabilityOf(path);
    if (!namespace) continue;
    namespaces.add(namespace);
    operations += 1;
  }
  return { namespaces, operations };
}

// --- 1. the documented surface, both directions ------------------------------

const excluded = namespaceExclusions();
const spec = JSON.parse(read(SPEC));
const { namespaces: live, operations } = specNamespaces(spec, excluded);

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
    fail("GONE", `${namespace} is documented but the spec no longer exposes it`);
  }
}
for (const namespace of live) {
  if (!documentedSet.has(namespace)) {
    fail("UNDOCUMENTED", `${namespace} is exposed by the spec and missing from the manifest`);
  }
}

// --- 2. the MCP artifact is a documented subset ------------------------------

let tools = 0;
let artifactFamilies = null;
if (!ARTIFACT) {
  fail("DERIVATION", "tool_derivation.artifact is missing");
} else if (!existsSync(join(root, ARTIFACT))) {
  fail("DERIVATION", `tool_derivation.artifact points at ${ARTIFACT}, which does not exist`);
} else {
  const artifact = JSON.parse(read(ARTIFACT));
  const families = artifact.families ?? [];
  artifactFamilies = new Set(families.map((family) => family.namespace));
  for (const family of families) {
    const namespace = family.namespace;
    if (typeof namespace !== "string" || namespace.length === 0) {
      fail("ARTIFACT", "a family has no namespace");
      continue;
    }
    if (!documentedSet.has(namespace)) {
      fail("ARTIFACT", `${namespace} is in the MCP catalog but not documented in the manifest`);
    }
    if (typeof family.capability !== "string" || !family.capability.endsWith(":read")) {
      fail("ARTIFACT", `${namespace} publishes no :read capability`);
    }
    for (const tool of family.tools ?? []) {
      tools += 1;
      if (typeof tool.name !== "string" || tool.name.length === 0) {
        fail("ARTIFACT", `${namespace} has a tool with no name`);
      }
      if (typeof tool.path !== "string" || !tool.path.startsWith("/api/")) {
        fail("ARTIFACT", `${namespace}/${tool.name} has no /api/ path`);
      }
      if (typeof tool.capability !== "string" || tool.capability.length === 0) {
        fail("ARTIFACT", `${namespace}/${tool.name} has no capability`);
      }
    }
  }
  if (typeof artifact.generated_from !== "string") {
    fail("ARTIFACT", "the catalog does not name what generated it");
  } else {
    for (const source of [SPEC, POLICY].filter(Boolean)) {
      if (!artifact.generated_from.includes(source)) {
        fail("ARTIFACT", `generated_from does not name ${source}`);
      }
    }
  }
  if (typeof artifact.version !== "number") {
    fail("ARTIFACT", "the catalog has no numeric version");
  }
}

if (POLICY && existsSync(join(root, POLICY)) && artifactFamilies) {
  const policy = JSON.parse(read(POLICY));
  for (const family of policy.exclude_families ?? []) {
    // A family the policy excludes may still be documented: the manifest covers
    // the whole agent-relevant API, and a sensitive family is exactly what an
    // agent should read about before it can reach it. The artifact, not the
    // manifest, is what must omit it.
    if (artifactFamilies.has(family)) {
      fail("ARTIFACT", `${family} is excluded by the policy but published by the catalog`);
    }
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
pointer("DERIVATION", "tool_derivation.spec", SPEC);
pointer("DERIVATION", "tool_derivation.adapter", ADAPTER);
pointer("DERIVATION", "tool_derivation.artifact", ARTIFACT);
pointer("DERIVATION", "tool_derivation.policy", POLICY);

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
        operations,
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
    `agent manifest OK: ${documented.length} namespaces documented and all present, ` +
      `${operations} spec reads, ${tools} MCP tools, every pointer resolves`,
  );
}

process.exit(problems.length === 0 ? 0 : 1);
