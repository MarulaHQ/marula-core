#!/usr/bin/env node
// Alert pack check (Wave 52 lane R7b).
//
// Purpose: fail the build when the alert pack stops agreeing with the code it
// watches. The pack is a set of YAML files that name series by string, and a
// metric rename, a copy edit in a runbook or a severity typo is invisible until
// someone is paged about a series that no longer exists. So this checks the
// things a YAML parser cannot:
//
//   1. Every series a rule reads is a contract metric, derived from
//      `crates/core-otel/contract/metrics.v1.json` with the same name
//      translation the OTel collector applies. A rename in the contract is a
//      failure here rather than an orphaned rule in production.
//   2. Every alert cites the contract metric it reads in its `metric`
//      annotation, and that metric's exported name appears in the expression.
//      A rule whose annotation and expression disagree is a rule nobody can
//      review.
//   3. Every alert has a severity of `page` or `ticket`, a `for`, a summary, and
//      a runbook that exists and names the alert back.
//   4. The runbook index agrees with the rule files on every alert's severity,
//      and the coverage table in the README names every metric in the contract,
//      so no metric is silently unread and no rule is silently undocumented.
//   5. `promtool check rules` and `promtool test rules` run when docker is
//      available. Syntax and firing behaviour belong to Prometheus, not to the
//      parser below.
//
// Non-purpose: it does not evaluate expressions and does not know whether a
// threshold is right. Firing behaviour is `promtool test rules`
// (`prometheus/tests/`), and the reasoning behind each number is a comment in
// the rule file.
//
// Usage:
//   node scripts/check-alert-rules.mjs            # human summary, exit 1 on fail
//   node scripts/check-alert-rules.mjs --json     # machine-readable verdict
//   node scripts/check-alert-rules.mjs --root DIR
//   node scripts/check-alert-rules.mjs --no-promtool   # structural checks only

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, relative, sep } from "node:path";

// Resolve the newest published contract rather than naming a version here: a
// contract bump used to leave this checker reading the previous file, so a rule
// could name a metric the current contract had renamed and the gate would pass.
const CONTRACT_DIR = "crates/core-otel/contract";
const CONTRACT_FILE = (() => {
  const versions = readdirSync(CONTRACT_DIR)
    .map((f) => /^metrics\.v(\d+)\.json$/.exec(f))
    .filter(Boolean)
    .map((m) => Number(m[1]))
    .sort((a, b) => b - a);
  if (versions.length === 0) return `${CONTRACT_DIR}/metrics.v1.json`;
  return `${CONTRACT_DIR}/metrics.v${versions[0]}.json`;
})();
const OBS = "deploy/observability";
const RULES_DIR = `${OBS}/prometheus/rules`;
const OPTIN_DIR = `${OBS}/prometheus/rules-optin`;
const TESTS_FILE = `${OBS}/prometheus/tests/marula-core.test.yaml`;
const README_FILE = `${OBS}/README.md`;
const RUNBOOK_INDEX = `${OBS}/runbooks/README.md`;
const PROMETHEUS_IMAGE =
  "prom/prometheus@sha256:6559acbd5d770b15bb3c954629ce190ac3cbbdb2b7f1c30f0385c4e05104e218";

// The OTel unit vocabulary, from the Prometheus translator
// (`pkg/translator/prometheus`, `unitMap`). A unit here appends its word to the
// metric name unless the name already contains that word.
const UNIT_MAP = {
  d: "days",
  h: "hours",
  min: "minutes",
  s: "seconds",
  ms: "milliseconds",
  us: "microseconds",
  ns: "nanoseconds",
  By: "bytes",
  KiBy: "kibibytes",
  MiBy: "mebibytes",
  GiBy: "gibibytes",
  TiBy: "tibibytes",
  KBy: "kilobytes",
  MBy: "megabytes",
  GBy: "gigabytes",
  TBy: "terabytes",
  m: "meters",
  V: "volts",
  A: "amperes",
  J: "joules",
  W: "watts",
  g: "grams",
  Cel: "celsius",
  Hz: "hertz",
  "1": "",
  "%": "percent",
};

// Series Prometheus derives itself rather than receiving from the product.
const DERIVED_SERIES = new Set(["up", "ALERTS", "ALERTS_FOR_STATE"]);

const SEVERITIES = new Set(["page", "ticket"]);

const args = process.argv.slice(2);
const asJson = args.includes("--json");
const noPromtool = args.includes("--no-promtool");
const rootAt = args.indexOf("--root");
const root = rootAt === -1 ? process.cwd() : args[rootAt + 1];

const failures = [];
const notes = [];

function fail(check, message) {
  failures.push({ check, message });
}

function read(relPath) {
  const path = join(root, relPath.split("/").join(sep));
  if (!existsSync(path)) {
    return null;
  }
  return readFileSync(path, "utf8");
}

// ---------------------------------------------------------------------------
// The name translation. Mirrors the collector's `normalizeName`, so a reader
// can check this against the upstream source rather than trusting it.
// ---------------------------------------------------------------------------

function cleanUp(value) {
  return value
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .join("_");
}

function exportedNames(metric) {
  let tokens = metric.name.split(/[^A-Za-z0-9]+/).filter(Boolean);
  const unit = String(metric.unit ?? "").split("/")[0].trim();
  if (unit && !/[{}]/.test(unit)) {
    const word = cleanUp(UNIT_MAP[unit] ?? unit);
    if (word && !tokens.includes(word)) {
      tokens.push(word);
    }
  }
  if (metric.kind === "counter") {
    tokens = tokens.filter((t) => t !== "total");
    tokens.push("total");
  }
  if (metric.unit === "1" && metric.kind === "gauge") {
    tokens = tokens.filter((t) => t !== "ratio");
    tokens.push("ratio");
  }
  const base = tokens.join("_");
  return metric.kind === "histogram"
    ? [base, `${base}_bucket`, `${base}_sum`, `${base}_count`]
    : [base];
}

// ---------------------------------------------------------------------------
// A YAML reader for the subset these rule files use: nested maps, lists of
// maps, folded and literal block scalars, quoted and plain scalars, comments.
// It refuses anything it does not recognise rather than guessing, because a
// silent misparse here would turn every downstream check green.
// ---------------------------------------------------------------------------

function stripComment(line) {
  let inSingle = false;
  let inDouble = false;
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i];
    if (ch === "'" && !inDouble) inSingle = !inSingle;
    else if (ch === '"' && !inSingle) inDouble = !inDouble;
    else if (ch === "#" && !inSingle && !inDouble) {
      if (i === 0 || line[i - 1] === " " || line[i - 1] === "\t") {
        return line.slice(0, i);
      }
    }
  }
  return line;
}

function scalar(raw) {
  const text = raw.trim();
  if (text === "") return null;
  if (text === "{}") return {};
  if (text === "[]") return [];
  if (text.startsWith('"') && text.endsWith('"') && text.length > 1) {
    return JSON.parse(text.replace(/\\'/g, "'"));
  }
  if (text.startsWith("'") && text.endsWith("'") && text.length > 1) {
    return text.slice(1, -1).replace(/''/g, "'");
  }
  return text;
}

function parseYaml(source, label) {
  const raw = source.split("\n");
  let index = 0;

  const indentOf = (line) => line.length - line.trimStart().length;
  const isBlank = (line) => line.trim() === "" || line.trim().startsWith("#");

  function nextSignificant(from) {
    let i = from;
    while (i < raw.length && isBlank(raw[i])) i += 1;
    return i;
  }

  function blockScalar(at, parentIndent, style) {
    const folded = style.startsWith(">");
    const strip = style.endsWith("-");
    const collected = [];
    let i = at;
    let blockIndent = null;
    while (i < raw.length) {
      const line = raw[i];
      if (line.trim() === "") {
        collected.push("");
        i += 1;
        continue;
      }
      const ind = indentOf(line);
      if (ind <= parentIndent) break;
      if (blockIndent === null) blockIndent = ind;
      collected.push(line.slice(blockIndent));
      i += 1;
    }
    while (collected.length && collected[collected.length - 1] === "") collected.pop();
    const joined = folded ? collected.join(" ").replace(/\s+/g, " ").trim() : collected.join("\n");
    return { value: strip ? joined : `${joined}\n`, next: i };
  }

  function parseBlock(startIndex, indent) {
    const first = nextSignificant(startIndex);
    if (first >= raw.length || indentOf(raw[first]) < indent) {
      return { value: null, next: first };
    }
    // The caller knows the parent's indentation, not this block's. Blocks are
    // indented by an arbitrary amount in valid YAML, so the child's own
    // indentation is the one to recurse with.
    const actual = indentOf(raw[first]);
    if (raw[first].trimStart().startsWith("- ")) {
      return parseList(first, actual);
    }
    return parseMap(first, actual);
  }

  function parseMap(startIndex, indent) {
    const out = {};
    let i = startIndex;
    while (i < raw.length) {
      const line = raw[i];
      if (isBlank(line)) {
        i += 1;
        continue;
      }
      const ind = indentOf(line);
      if (ind < indent) break;
      if (ind > indent) {
        throw new Error(`${label}:${i + 1}: unexpected indentation inside a mapping`);
      }
      const text = stripComment(line).trim();
      if (text.startsWith("- ")) break;
      const colon = text.indexOf(":");
      if (colon === -1) {
        throw new Error(`${label}:${i + 1}: expected "key: value", found ${JSON.stringify(text)}`);
      }
      const key = text.slice(0, colon).trim();
      const rest = text.slice(colon + 1).trim();
      if (/^[>|][+-]?$/.test(rest)) {
        const { value, next } = blockScalar(i + 1, ind, rest);
        out[key] = value;
        i = next;
        continue;
      }
      if (rest === "") {
        const { value, next } = parseBlock(i + 1, ind + 1);
        out[key] = value;
        i = next;
        continue;
      }
      out[key] = scalar(rest);
      i += 1;
    }
    return { value: out, next: i };
  }

  function parseList(startIndex, indent) {
    const out = [];
    let i = startIndex;
    while (i < raw.length) {
      const line = raw[i];
      if (isBlank(line)) {
        i += 1;
        continue;
      }
      const ind = indentOf(line);
      if (ind < indent) break;
      if (ind > indent) {
        throw new Error(`${label}:${i + 1}: unexpected indentation inside a sequence`);
      }
      const text = stripComment(line).trim();
      if (!text.startsWith("- ")) break;
      const itemText = text.slice(2).trim();
      const itemIndent = ind + 2;
      if (itemText === "") {
        const { value, next } = parseBlock(i + 1, itemIndent);
        out.push(value);
        i = next;
        continue;
      }
      if (itemText.includes(": ")) {
        // A map item: re-read this line as the first key of a mapping whose
        // indentation starts where the item text starts.
        raw[i] = " ".repeat(itemIndent) + itemText;
        const { value, next } = parseMap(i, itemIndent);
        out.push(value);
        i = next;
        continue;
      }
      out.push(scalar(itemText));
      i += 1;
    }
    return { value: out, next: i };
  }

  const { value } = parseBlock(index, 0);
  return value;
}

// ---------------------------------------------------------------------------
// Load the rule files and flatten them into alerts and recording rules.
// ---------------------------------------------------------------------------

function ruleFiles() {
  const files = [];
  for (const dir of [RULES_DIR, OPTIN_DIR]) {
    const absolute = join(root, dir.split("/").join(sep));
    if (!existsSync(absolute)) {
      fail("layout", `${dir} is missing`);
      continue;
    }
    for (const entry of readdirSync(absolute).sort()) {
      if (entry.endsWith(".rules.yaml")) {
        files.push(`${dir}/${entry}`);
      }
    }
  }
  if (files.length === 0) {
    fail("layout", `${RULES_DIR} has no *.rules.yaml files`);
  }
  return files;
}

const contractSource = read(CONTRACT_FILE);
if (!contractSource) {
  fail("contract", `${CONTRACT_FILE} is missing`);
}
const contract = contractSource ? JSON.parse(contractSource) : { metrics: [] };
const metricsByName = new Map(contract.metrics.map((m) => [m.name, m]));
const exportedToContract = new Map();
for (const metric of contract.metrics) {
  for (const name of exportedNames(metric)) {
    exportedToContract.set(name, metric.name);
  }
}

const alerts = [];
const recording = new Set();
const recordingExprs = new Map();
const files = ruleFiles();

for (const relPath of files) {
  const text = read(relPath);
  if (text === null) {
    fail("layout", `${relPath} is missing`);
    continue;
  }
  let doc;
  try {
    doc = parseYaml(text, relPath);
  } catch (error) {
    fail("yaml", error.message);
    continue;
  }
  const groups = Array.isArray(doc?.groups) ? doc.groups : [];
  if (groups.length === 0) {
    fail("yaml", `${relPath} has no groups`);
  }
  for (const group of groups) {
    const rules = Array.isArray(group?.rules) ? group.rules : [];
    if (rules.length === 0) {
      fail("yaml", `${relPath}: group "${group?.name}" has no rules`);
    }
    for (const rule of rules) {
      if (rule?.record) {
        recording.add(rule.record);
        if (!rule.expr) {
          fail("rule", `${relPath}: recording rule ${rule.record} has no expr`);
        } else {
          recordingExprs.set(rule.record, String(rule.expr));
        }
        continue;
      }
      if (!rule?.alert) {
        fail("rule", `${relPath}: a rule in group "${group?.name}" has neither alert nor record`);
        continue;
      }
      alerts.push({ file: relPath, group: group.name, ...rule });
    }
  }
}

// ---------------------------------------------------------------------------
// Check 1 and 2: the series and the citation.
// ---------------------------------------------------------------------------

function seriesIn(expr) {
  const found = new Set();
  for (const match of expr.matchAll(/\bmarula_[a-z0-9_]+/g)) {
    found.add(match[0]);
  }
  // Recording rules carry colons (`marula:sli_http_error:ratio_rate1h`), which
  // the series pattern above cannot match. An alert reading one of those reads
  // whatever that recording rule reads, and the citation check follows it.
  for (const match of expr.matchAll(/\bmarula:[a-z0-9_:]+[a-z0-9_]/g)) {
    found.add(match[0]);
  }
  for (const match of expr.matchAll(/\b(up|ALERTS|ALERTS_FOR_STATE)\b/g)) {
    found.add(match[1]);
  }
  return found;
}

for (const alert of alerts) {
  const where = `${alert.file}: ${alert.alert}`;
  if (!alert.expr) {
    fail("rule", `${where} has no expr`);
    continue;
  }
  if (!alert.for) {
    fail("rule", `${where} has no \`for\``);
  }
  const severity = alert.labels?.severity;
  if (!SEVERITIES.has(severity)) {
    fail("severity", `${where} severity is ${JSON.stringify(severity)}, not page or ticket`);
  }
  if (!alert.annotations?.summary) {
    fail("rule", `${where} has no annotations.summary`);
  }
  if (!alert.annotations?.description) {
    fail("rule", `${where} has no annotations.description`);
  }

  for (const series of seriesIn(alert.expr)) {
    if (DERIVED_SERIES.has(series) || recording.has(series)) continue;
    if (!exportedToContract.has(series)) {
      fail(
        "series",
        `${where} reads ${series}, which no contract metric exports. ` +
          `Derived from ${CONTRACT_FILE}, the pack may only read those names.`,
      );
    }
  }

  const cited = alert.annotations?.metric;
  if (!cited) {
    fail("citation", `${where} has no annotations.metric naming the contract metric it reads`);
  } else if (!metricsByName.has(cited) && !DERIVED_SERIES.has(cited)) {
    fail("citation", `${where} cites ${cited}, which is not in the contract`);
  } else if (metricsByName.has(cited)) {
    const names = exportedNames(metricsByName.get(cited));
    // An alert may read a recording rule rather than the raw series, which is
    // how the burn-rate rules work. Follow one level of that indirection: the
    // citation is satisfied when the alert reads a recording rule whose own
    // expression reads the cited metric.
    const viaRecording = [...seriesIn(alert.expr)]
      .filter((series) => recordingExprs.has(series))
      .some((series) => names.some((name) => recordingExprs.get(series).includes(name)));
    if (!names.some((name) => alert.expr.includes(name)) && !viaRecording) {
      fail(
        "citation",
        `${where} cites ${cited} but neither its expression nor the recording rule it reads touches ${names[0]}`,
      );
    }
  }

  const runbook = alert.annotations?.runbook;
  if (!runbook) {
    fail("runbook", `${where} has no annotations.runbook`);
  } else {
    const text = read(runbook);
    if (text === null) {
      fail("runbook", `${where} points at ${runbook}, which does not exist`);
    } else if (!text.includes(`\`${alert.alert}\``)) {
      fail("runbook", `${where} points at ${runbook}, which never names the alert`);
    }
  }
}

// ---------------------------------------------------------------------------
// Check 3: the runbook index, and the reverse direction on runbook links.
// ---------------------------------------------------------------------------

const index = read(RUNBOOK_INDEX);
if (index === null) {
  fail("layout", `${RUNBOOK_INDEX} is missing`);
} else {
  for (const alert of alerts) {
    if (!index.includes(`\`${alert.alert}\``)) {
      fail("index", `${alert.alert} is missing from ${RUNBOOK_INDEX}`);
      continue;
    }
    const row = index
      .split("\n")
      .find((line) => line.includes(`\`${alert.alert}\``) && line.trim().startsWith("|"));
    if (!row) {
      fail("index", `${alert.alert} has no table row in ${RUNBOOK_INDEX}`);
      continue;
    }
    const severity = alert.labels?.severity;
    if (severity && !row.includes(`| ${severity} |`)) {
      fail(
        "index",
        `${alert.alert} is ${severity} in its rule file but the index row does not say so`,
      );
    }
  }

  const alertNames = new Set(alerts.map((a) => a.alert));
  const runbookDir = join(root, `${OBS}/runbooks`.split("/").join(sep));
  for (const entry of readdirSync(runbookDir)) {
    if (!entry.endsWith(".md") || entry === "README.md") continue;
    const relPath = `${OBS}/runbooks/${entry}`;
    const text = read(relPath);
    const named = new Set();
    for (const match of text.matchAll(/`(Marula[A-Za-z]+)`/g)) {
      named.add(match[1]);
    }
    for (const name of named) {
      if (!alertNames.has(name)) {
        fail("runbook", `${relPath} names ${name}, which is not a rule in the pack`);
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Check 4: every contract metric is either read or explained. The README's
// coverage table is the explanation, so a metric absent from it is a metric
// nobody noticed the pack ignores.
// ---------------------------------------------------------------------------

const readme = read(README_FILE);
if (readme === null) {
  fail("layout", `${README_FILE} is missing`);
} else {
  for (const metric of contract.metrics) {
    if (!readme.includes(metric.name)) {
      fail(
        "coverage",
        `${metric.name} is in the contract and absent from the coverage table in ${README_FILE}`,
      );
    }
  }
}

// ---------------------------------------------------------------------------
// Check 5: hand syntax and firing behaviour to Prometheus when it is here.
// ---------------------------------------------------------------------------

let promtool = "skipped";
if (!noPromtool) {
  // Two mounts, for two questions. `check config` is run with the pack mounted
  // where the shipped Prometheus expects it, so the rule-file globs in
  // prometheus.yml resolve exactly as they do in the container. `test rules`
  // mounts the pack at /p, which is the path the test file's `rule_files`
  // entries use.
  const promRoot = `${OBS}/prometheus`;
  const packMount = `${join(root, promRoot.split("/").join(sep))}:/etc/prometheus:ro`;
  const testMount = `${join(root, promRoot.split("/").join(sep))}:/p:ro`;
  const run = (mount, argv) =>
    execFileSync("docker", ["run", "--rm", "--entrypoint", "promtool", "-v", mount, PROMETHEUS_IMAGE, ...argv], {
      stdio: ["ignore", "pipe", "pipe"],
      encoding: "utf8",
    });
  try {
    run(packMount, ["check", "config", "/etc/prometheus/prometheus.yml"]);
    run(testMount, ["test", "rules", "/p/tests/marula-core.test.yaml"]);
    promtool = "passed";
  } catch (error) {
    const detail = `${error.stdout ?? ""}${error.stderr ?? ""}`.trim() || error.message;
    if (/Cannot connect to the Docker daemon|docker: not found|ENOENT/.test(detail)) {
      promtool = "skipped (no docker)";
      notes.push(
        "docker was not available, so `promtool check rules` and `promtool test rules` did not run",
      );
    } else {
      promtool = "failed";
      fail("promtool", detail.split("\n").slice(-25).join("\n"));
    }
  }
} else {
  promtool = "skipped (--no-promtool)";
}

// ---------------------------------------------------------------------------
// Verdict.
// ---------------------------------------------------------------------------

const summary = {
  ok: failures.length === 0,
  alerts: alerts.length,
  page: alerts.filter((a) => a.labels?.severity === "page").length,
  ticket: alerts.filter((a) => a.labels?.severity === "ticket").length,
  recordingRules: recording.size,
  ruleFiles: files,
  contractMetrics: contract.metrics.length,
  exportedSeries: exportedToContract.size,
  promtool,
  notes,
  failures,
};

if (asJson) {
  console.log(JSON.stringify(summary, null, 2));
} else {
  console.log(`alert pack: ${summary.alerts} alerts (${summary.page} page, ${summary.ticket} ticket)`);
  console.log(`recording rules: ${summary.recordingRules}`);
  console.log(`rule files: ${files.map((f) => relative(root, join(root, f.split("/").join(sep)))).join(", ")}`);
  console.log(
    `contract: ${summary.contractMetrics} metrics, ${summary.exportedSeries} exported series names`,
  );
  console.log(`promtool: ${summary.promtool}`);
  for (const note of notes) {
    console.log(`  note: ${note}`);
  }
  if (failures.length === 0) {
    console.log("\nOK — every rule reads a contract metric, cites it, and has a runbook that names it");
  } else {
    console.log("");
    for (const failure of failures) {
      console.log(`  [${failure.check}] ${failure.message}`);
    }
    console.log(`\n${failures.length} failure(s)`);
  }
}

process.exit(failures.length === 0 ? 0 : 1);
