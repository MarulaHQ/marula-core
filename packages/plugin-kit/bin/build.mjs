#!/usr/bin/env node
// @marula-core/plugin-kit — validate + build a Marula plugin bundle.
//
// Usage: node bin/build.mjs <plugin-dir>
//
// Validates <plugin-dir>/manifest.json against the v2 shape (closed
// capability vocabulary, closed surface vocabularies, JSON-Schema
// subsets) the SAME way crates/plugin-host does at ingest, then zips
// manifest.json + plugin.wasm into <plugin-dir>/dist/<name>-<version>.zip
// — the layout `ingest_local_zip` / registry entries consume.
//
// Exit 0 = clean bundle; any validation failure prints and exits 1
// (fail-closed: a bundle the host would refuse is never built).

import { createHash } from "node:crypto";
import { readFileSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { basename, join } from "node:path";

//
// The closed vocabulary is `PluginCapability` in
// crates/core-domain/src/entities/plugins.rs — the HOST is the source of
// truth. This Set is the mirror it validates against, and it must agree with
// the host exactly: a capability in the host but missing here means a valid
// manifest is rejected by this local pre-validation while the host's
// `PluginCapability::parse` accepts it, so the two disagree about what a
// valid plugin is.
//
// The parity that used to be a promise in this comment is now enforced:
// `plugin_capability_mirror_matches_the_plugin_kit` in the plugins.rs test
// module reads this file and asserts the Set equals `PluginCapability::ALL`.
// A capability added to the Rust enum without its wire string here fails that
// test, and so does a wire string here with no enum variant.
const KNOWN_CAPABILITIES = new Set([
  "tickets.read", "tickets.write", "clients.read", "clients.write",
  "time.read", "time.write", "projects.read", "projects.write",
  "kb.read", "kb.write", "documents.read", "documents.write",
  "reporting.read", "financial.read", "vault:ingress",
  "routing.matrix", "routing.route", "routing.geocode", "routing.isochrone",
  "ui.widgets", "automation.actions", "reports.render", "fields.validate",
  "automation.triggers", "automation.reads", "events.emit",
  "service_accounts.manage", "org_usage.read",
  // Wave 45 RAG namespace, the Wave 47 A8 domain surface, the Wave 47 A7
  // suspension notice, the Wave 52 R12 observability read scope, and the
  // Wave 52 R16 directory pair. All 35 wire strings must be here.
  "rag.read", "rag.write", "org_custom_domain.manage", "org_suspension.read",
  "observability.read", "directory_groups.read", "directory_groups.write",
]);
const SLOTS = new Set(["dashboard", "ticket_pane"]);
const WIDGET_KINDS = new Set(["stat", "metric", "list"]);
const EFFECTS = new Set(["add_internal_comment"]);
const SCHEMA_TYPES = new Set(["string", "number", "integer", "boolean"]);
const INTERFACE_VERSION = 1;
const MAX_PER_SECTION = 8;

const fail = (msg) => {
  console.error(`plugin-kit: ${msg}`);
  process.exit(1);
};

const idOk = (id) =>
  /^[a-z][a-z0-9_]{0,63}$/.test(id);

const labelOk = (label) =>
  typeof label === "string" && label.trim().length > 0 && Buffer.byteLength(label) <= 120;

/** The JSON-Schema subset SettingsSchema accepts server-side. */
function checkSchema(schema, what) {
  if (schema === undefined || schema === null) return;
  if (typeof schema !== "object" || Array.isArray(schema)) fail(`${what}: must be an object`);
  if (schema.type !== "object") fail(`${what}: must declare "type": "object"`);
  const props = schema.properties ?? {};
  for (const [key, spec] of Object.entries(props)) {
    if (!SCHEMA_TYPES.has(spec?.type)) fail(`${what}.${key}: unsupported or missing type`);
    const { minLength, maxLength, minimum, maximum } = spec;
    if (minLength !== undefined && maxLength !== undefined && maxLength < minLength)
      fail(`${what}.${key}: maxLength < minLength`);
    if (minimum !== undefined && maximum !== undefined && maximum < minimum)
      fail(`${what}.${key}: maximum < minimum`);
  }
}

const dir = process.argv[2];
if (!dir) fail("usage: node bin/build.mjs <plugin-dir>");
const manifestPath = join(dir, "manifest.json");
if (!existsSync(manifestPath)) fail(`${manifestPath} not found`);

let m;
try {
  m = JSON.parse(readFileSync(manifestPath, "utf8"));
} catch (e) {
  fail(`manifest.json is not valid JSON: ${e.message}`);
}

if (typeof m.name !== "string" || m.name.trim() === "") fail("manifest.name must be a non-empty string");
if (typeof m.version !== "string" || !/^\d+\.\d+\.\d+$/.test(m.version)) fail("manifest.version must be semver");
if (m.interface_version !== INTERFACE_VERSION) fail(`manifest.interface_version must be ${INTERFACE_VERSION}`);
if (!Array.isArray(m.capabilities) || m.capabilities.length === 0) fail("manifest.capabilities must be a non-empty array");
for (const cap of m.capabilities) {
  if (!KNOWN_CAPABILITIES.has(cap)) fail(`unknown capability ${JSON.stringify(cap)} — the vocabulary is closed`);
}
if (m.connector_profile) fail("surface plugins must not declare a connector_profile");

const profile = m.surface_profile ?? {};
const sections = ["widgets", "actions", "render_kinds", "field_validators"];
for (const section of sections) {
  const entries = profile[section];
  if (entries === undefined) continue;
  if (!Array.isArray(entries)) fail(`surface_profile.${section} must be an array`);
  if (entries.length > MAX_PER_SECTION) fail(`surface_profile.${section}: max ${MAX_PER_SECTION} entries`);
}
const seen = new Set();
for (const w of profile.widgets ?? []) {
  if (!idOk(w.id)) fail(`widget id ${JSON.stringify(w.id)} must match [a-z][a-z0-9_]{0,63}`);
  if (seen.has(`w:${w.id}`)) fail(`widget id ${w.id} declared twice`);
  seen.add(`w:${w.id}`);
  if (!labelOk(w.title)) fail(`widget ${w.id}: title must be 1..120 bytes`);
  if (!SLOTS.has(w.slot)) fail(`widget ${w.id}: slot must be dashboard or ticket_pane`);
  if (!WIDGET_KINDS.has(w.kind)) fail(`widget ${w.id}: kind must be stat, metric, or list`);
  checkSchema(w.props_schema, `widget ${w.id}.props_schema`);
  if (w.default_props !== undefined) {
    if (w.props_schema === undefined) fail(`widget ${w.id}: default_props without props_schema`);
    const props = Object.entries(w.default_props ?? {});
    const specs = w.props_schema?.properties ?? {};
    for (const [k, v] of props) {
      const spec = specs[k];
      if (spec === undefined) fail(`widget ${w.id}: default_props.${k} is not in props_schema`);
      const t = spec?.type;
      const ok =
        (t === "string" && typeof v === "string") ||
        ((t === "number" || t === "integer") && typeof v === "number") ||
        (t === "boolean" && typeof v === "boolean");
      if (!ok) fail(`widget ${w.id}: default_props.${k} violates its declared type`);
    }
  }
}
for (const a of profile.actions ?? []) {
  if (!idOk(a.id)) fail(`action id ${JSON.stringify(a.id)} must match [a-z][a-z0-9_]{0,63}`);
  if (seen.has(`a:${a.id}`)) fail(`action id ${a.id} declared twice`);
  seen.add(`a:${a.id}`);
  if (!labelOk(a.label)) fail(`action ${a.id}: label must be 1..120 bytes`);
  if (typeof a.description !== "string" || a.description.trim() === "" || a.description.length > 400)
    fail(`action ${a.id}: description must be 1..400 bytes`);
  if (!EFFECTS.has(a.effect)) fail(`action ${a.id}: effect must be add_internal_comment`);
  checkSchema(a.params_schema, `action ${a.id}.params_schema`);
}
for (const r of profile.render_kinds ?? []) {
  if (!idOk(r.id)) fail(`render kind id ${JSON.stringify(r.id)} must match [a-z][a-z0-9_]{0,63}`);
  if (seen.has(`r:${r.id}`)) fail(`render kind id ${r.id} declared twice`);
  seen.add(`r:${r.id}`);
  if (!labelOk(r.label)) fail(`render kind ${r.id}: label must be 1..120 bytes`);
  if (typeof r.description !== "string" || r.description.trim() === "") fail(`render kind ${r.id}: description required`);
}
for (const v of profile.field_validators ?? []) {
  if (!idOk(v.id)) fail(`field validator id ${JSON.stringify(v.id)} must match [a-z][a-z0-9_]{0,63}`);
  if (seen.has(`v:${v.id}`)) fail(`field validator id ${v.id} declared twice`);
  seen.add(`v:${v.id}`);
  if (!labelOk(v.label)) fail(`field validator ${v.id}: label must be 1..120 bytes`);
  checkSchema(v.schema, `field validator ${v.id}.schema`);
}

const wasmPath = join(dir, "plugin.wasm");
if (!existsSync(wasmPath)) fail(`plugin.wasm not found — build it first (see README)`);
const wasm = readFileSync(wasmPath);
if (wasm.length < 4 || wasm.subarray(0, 4).toString("hex") !== "0061736d")
  fail("plugin.wasm is not a WASM module (missing \\0asm magic)");
if (wasm.length > 4 * 1024 * 1024) fail("plugin.wasm exceeds the 4 MiB member cap");

const slug = m.name.trim().toLowerCase().replace(/[^a-z0-9_-]/g, "-");
const outDir = join(dir, "dist");
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
const out = join(outDir, `${slug}-${m.version}.zip`);
execFileSync("zip", ["-j", "-q", out, manifestPath, wasmPath]);
const zip = readFileSync(out);
const digest = createHash("sha256").update(zip).digest("hex");
console.log(`plugin-kit: built ${out}`);
console.log(`plugin-kit: sha256 ${digest}`);
