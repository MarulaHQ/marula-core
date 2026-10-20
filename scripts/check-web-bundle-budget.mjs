#!/usr/bin/env node
// Bundle budget gate for `apps/web`.
//
// Purpose: make "the frontend got heavier" a diff instead of an argument. The
// script reads the production build's route -> chunk manifest, gzips every
// chunk a route loads before it is interactive (the same set Next prints as
// "First Load JS"), and checks each route against the numbers committed in
// `apps/web/perf/bundle-budget.json`: a recorded baseline, a ceiling derived
// from it, and a ceiling per surface group.
//
// Non-purpose: it never builds. It reads an existing `.next` output, so the
// gate costs a second after the group gate's single `next build`.
// `--update-baseline` is the only sanctioned way to accept a new number, and
// it rewrites the JSON so the acceptance shows up as a diff in review.
//
// Usage:
//   node scripts/check-web-bundle-budget.mjs                  # verify
//   node scripts/check-web-bundle-budget.mjs --report         # table, no verdict
//   node scripts/check-web-bundle-budget.mjs --update-baseline
//   node scripts/check-web-bundle-budget.mjs --app apps/web --config <path>
//   node scripts/check-web-bundle-budget.mjs --json
//
// Exit codes: 0 inside every ceiling and baseline; 1 a breach or regression;
// 2 the build output or config is missing/unreadable.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { gzipSync } from "node:zlib";

/** Baseline drift allowed before a route counts as regressed. Gzip size for a
 *  fixed source tree is deterministic, so this only absorbs toolchain noise. */
export const DEFAULT_TOLERANCE_BYTES = 4 * 1024;
/** Ceilings derived from the baseline leave this much room for real growth. */
export const DEFAULT_HEADROOM_PCT = 5;
export const DEFAULT_MIN_HEADROOM_BYTES = 1024;

function die(message) {
  console.error(`bundle-budget: ${message}`);
  process.exit(2);
}

/** Read `.next/app-build-manifest.json` and resolve every route's first-load
 *  chunks to { raw, gzip } bytes. Chunk paths are `.next`-relative. */
export function computeRouteSizes(appDir) {
  const nextDir = path.join(appDir, ".next");
  const manifestPath = path.join(nextDir, "app-build-manifest.json");
  let manifest;
  try {
    manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  } catch (error) {
    die(`cannot read ${manifestPath} (${error.message}). Run \`pnpm --filter @marula-core/web build\` first.`);
  }
  const sizes = new Map();
  const chunkCache = new Map();
  for (const [pageKey, files] of Object.entries(manifest.pages ?? {})) {
    if (!pageKey.endsWith("/page")) continue; // layouts are shared, not routes
    const route = pageKey.slice(0, -"/page".length) || "/";
    let raw = 0;
    let gzip = 0;
    for (const file of files) {
      let size = chunkCache.get(file);
      if (size === undefined) {
        const absolute = path.join(nextDir, file);
        let buffer;
        try {
          buffer = readFileSync(absolute);
        } catch (error) {
          die(`chunk referenced by the manifest is missing: ${file} (${error.message})`);
        }
        size = { raw: buffer.length, gzip: gzipSync(buffer, { level: 9 }).length };
        chunkCache.set(file, size);
      }
      raw += size.raw;
      gzip += size.gzip;
    }
    sizes.set(route, { raw, gzip });
  }
  return sizes;
}

/** The ceiling a route inherits from the config: an explicit override wins,
 *  otherwise baseline + headroom, rounded up to 4 KiB so the number reads
 *  like a budget rather than a measurement. */
export function derivedCeiling(baselineBytes, config) {
  const pct = config.headroom_pct ?? DEFAULT_HEADROOM_PCT;
  const floor = config.min_headroom_bytes ?? DEFAULT_MIN_HEADROOM_BYTES;
  const headroom = Math.max(floor, Math.round((baselineBytes * pct) / 100));
  const step = 4 * 1024;
  return Math.ceil((baselineBytes + headroom) / step) * step;
}

/** Compare measured sizes against the committed config. Returns display rows
 *  plus the failures that decide the exit code. */
export function evaluateBudget(sizes, config, options = {}) {
  const tolerance = options.toleranceBytes ?? config.tolerance_bytes ?? DEFAULT_TOLERANCE_BYTES;
  const baseline = config.baseline ?? {};
  const overrides = config.ceilings ?? {};
  const failures = [];
  const warnings = [];
  const rows = [];

  for (const [route, base] of Object.entries(baseline)) {
    const measured = sizes.get(route);
    if (measured === undefined) {
      // A deleted route cannot be over budget; the config diff is where a
      // rename shows up, so this is a note rather than a gate failure.
      warnings.push(`${route} is in the baseline but no longer in the build — prune it with --update-baseline if the removal was intended`);
      rows.push({ route, gzip: null, baseline: base, ceiling: null, verdict: "GONE" });
      continue;
    }
    const ceiling = overrides[route] ?? derivedCeiling(base, config);
    let verdict = "ok";
    if (measured.gzip > ceiling) {
      verdict = "OVER CEILING";
      failures.push({ kind: "ceiling", route, detail: `${measured.gzip} > ${ceiling} bytes gzip (baseline ${base})` });
    } else if (measured.gzip > base + tolerance) {
      verdict = "REGRESSED";
      failures.push({ kind: "regression", route, detail: `${measured.gzip} > baseline ${base} + ${tolerance} tolerance` });
    }
    rows.push({ route, gzip: measured.gzip, baseline: base, ceiling, verdict });
  }

  for (const route of Object.keys(overrides)) {
    if (!(route in baseline)) {
      warnings.push(`ceiling override for ${route} has no baseline entry and is ignored`);
    }
  }

  for (const group of config.groups ?? []) {
    const routes = group.routes ?? [];
    let worst = null;
    for (const route of routes) {
      const size = sizes.get(route);
      if (size !== undefined && (worst === null || size.gzip > worst.gzip)) worst = { route, gzip: size.gzip };
    }
    if (worst === null) {
      warnings.push(`group ${group.name} matches no route in the build output`);
      rows.push({ route: `group:${group.name}`, gzip: null, baseline: null, ceiling: group.ceiling_bytes ?? null, verdict: "EMPTY" });
      continue;
    }
    let verdict = "ok";
    if (typeof group.ceiling_bytes === "number" && worst.gzip > group.ceiling_bytes) {
      verdict = "OVER CEILING";
      failures.push({
        kind: "group-ceiling",
        route: group.name,
        detail: `heaviest route ${worst.route} at ${worst.gzip} > group ceiling ${group.ceiling_bytes} bytes gzip`,
      });
    }
    rows.push({ route: `group:${group.name} (${worst.route})`, gzip: worst.gzip, baseline: null, ceiling: group.ceiling_bytes ?? null, verdict });
  }

  return { ok: failures.length === 0, failures, warnings, rows };
}

function kb(bytes) {
  return bytes === null || bytes === undefined ? "—" : (bytes / 1024).toFixed(1);
}

/** Human table: one line per route plus the group aggregates. */
export function formatReport(result) {
  const lines = [];
  const nameWidth = Math.max(8, ...result.rows.map((row) => row.route.length));
  lines.push(
    `${"route".padEnd(nameWidth)}  ${"gzip kB".padStart(9)}  ${"baseline".padStart(9)}  ${"delta".padStart(7)}  ${"ceiling".padStart(8)}  verdict`,
  );
  for (const row of result.rows) {
    const delta =
      row.gzip !== null && row.baseline !== null
        ? `${row.gzip >= row.baseline ? "+" : "−"}${kb(Math.abs(row.gzip - row.baseline))}`
        : "—";
    lines.push(
      `${row.route.padEnd(nameWidth)}  ${kb(row.gzip).padStart(9)}  ${kb(row.baseline).padStart(9)}  ${delta.padStart(7)}  ${kb(row.ceiling).padStart(8)}  ${row.verdict}`,
    );
  }
  return lines.join("\n");
}

function parseArgs(argv) {
  const options = {
    app: "apps/web",
    config: null,
    report: false,
    json: false,
    updateBaseline: false,
    toleranceBytes: null,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--app") options.app = argv[++i];
    else if (arg === "--config") options.config = argv[++i];
    else if (arg === "--report") options.report = true;
    else if (arg === "--json") options.json = true;
    else if (arg === "--update-baseline") options.updateBaseline = true;
    else if (arg === "--tolerance") options.toleranceBytes = Number(argv[++i]);
    else if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: node scripts/check-web-bundle-budget.mjs [--app apps/web] [--config <path>] [--report] [--json] [--update-baseline] [--tolerance <bytes>]",
      );
      process.exit(0);
    } else {
      die(`unknown argument ${arg}`);
    }
  }
  options.config ??= path.join(options.app, "perf", "bundle-budget.json");
  return options;
}

function readJson(file) {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch (error) {
    die(`cannot read ${file} (${error.message})`);
  }
}

function updateBaseline(sizes, config, file) {
  const next = {
    $comment: config.$comment,
    metric: config.metric ?? "first-load-js-gzip-bytes",
    next_version: config.next_version ?? null,
    tolerance_bytes: config.tolerance_bytes ?? DEFAULT_TOLERANCE_BYTES,
    headroom_pct: config.headroom_pct ?? DEFAULT_HEADROOM_PCT,
    min_headroom_bytes: config.min_headroom_bytes ?? DEFAULT_MIN_HEADROOM_BYTES,
    ceilings: config.ceilings ?? {},
    groups: config.groups ?? [],
    baseline: {},
  };
  for (const route of Object.keys(config.baseline ?? {})) {
    const measured = sizes.get(route);
    if (measured !== undefined) next.baseline[route] = measured.gzip;
  }
  writeFileSync(file, `${JSON.stringify(next, null, 2)}\n`);
  return Object.keys(next.baseline).length;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const sizes = computeRouteSizes(options.app);

  if (options.report) {
    const rows = [...sizes.entries()]
      .sort((a, b) => b[1].gzip - a[1].gzip)
      .map(([route, size]) => ({ route, gzip: size.gzip, baseline: null, ceiling: null, verdict: "" }));
    console.log(formatReport({ rows }));
    return;
  }

  const config = readJson(options.config);
  if (options.updateBaseline) {
    const count = updateBaseline(sizes, config, options.config);
    console.log(`bundle-budget: baseline rewritten in ${options.config} (${count} routes)`);
    return;
  }

  const result = evaluateBudget(sizes, config, options);
  if (options.json) {
    console.log(JSON.stringify({ ok: result.ok, failures: result.failures, warnings: result.warnings, rows: result.rows }, null, 2));
  } else {
    console.log(formatReport(result));
    for (const warning of result.warnings) console.log(`note: ${warning}`);
    if (!result.ok) {
      console.error(`\nbundle-budget: ${result.failures.length} failure(s)`);
      for (const failure of result.failures) console.error(`  - ${failure.kind}: ${failure.route} — ${failure.detail}`);
    } else {
      console.log("\nbundle-budget: every route inside its ceiling and baseline.");
    }
  }
  process.exit(result.ok ? 0 : 1);
}

if (process.argv[1] !== undefined && import.meta.url === new URL(`file://${path.resolve(process.argv[1])}`).href) {
  main();
}
