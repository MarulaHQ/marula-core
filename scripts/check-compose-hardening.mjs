#!/usr/bin/env node
// check-compose-hardening.mjs — the compose rules the container gate enforces.
//
// WHY THIS EXISTS
// ---------------
// deploy/ ships compose files, and nothing read them. Trivy's config scan
// handles Dockerfiles, Kubernetes manifests and Helm charts; pointed at a
// docker-compose.yml it reports `Detected config files num=0`. hadolint reads
// Dockerfiles. So `privileged: true` could land in a compose file and ship.
//
// WHAT IT CHECKS
// --------------
// Five settings, each a container reaching past its own boundary:
//
//   privileged: true          every capability on the host
//   network_mode: host        the host's network stack
//   pid: host                 visibility into host processes
//   ipc: host                 host shared memory
//   /var/run/docker.sock      control of the daemon, which is root on the host
//
// `cap_add` and `read_only` are deliberately not checked. The first is a narrow
// grant the edge file uses for NET_BIND_SERVICE; the second already carries an
// `EXEMPTION (read_only)` convention in the files themselves.
//
// HOW IT READS THE FILES
// ----------------------
// With the `yaml` parser, not a regex. An earlier scanner missed three idiomatic
// spellings (a trailing comment, flow style, a quoted key) and then reported a
// command string as configuration. A parser settles all of it: the settings are
// read from the service mappings that Compose itself would read.
//
// Detection runs on the *resolved* document, so a setting reached through an
// anchor or a `<<` merge key counts too, and the node tree supplies the line
// for the report. A compose file that is not valid YAML fails the gate rather
// than being skipped.
//
// Files are selected by name (`*compose*.yml`), because `deploy/helm` holds
// Go-templated chart files that are not valid YAML until rendered and are not
// Compose in any case.
//
// EXEMPTIONS
// ----------
// A setting is exempt when the comment directly above its key carries
// `EXEMPTION (compose)` with a reason. The parser attaches exactly that
// comment block to the key, so a marker higher up the file cannot drift onto a
// setting somebody adds later. That is the same convention the files already
// use for `EXEMPTION (read_only)`.
//
// USAGE
//   node scripts/check-compose-hardening.mjs             # scan deploy/
//   node scripts/check-compose-hardening.mjs --self-test # prove it can fail

import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { LineCounter, isMap, isScalar, isSeq, parseDocument } from 'yaml';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const DEFAULT_SCAN_DIR = join(ROOT, 'deploy');
const MARKER = 'EXEMPTION (compose)';

// Compose reads `true`/`yes`/`on`/`1` as a boolean; the YAML core schema keeps
// the last three as strings, so both spellings are accepted here.
const TRUTHY = new Set(['true', 'yes', 'on', '1']);
const isTruthy = (value) => value === true || TRUTHY.has(String(value).toLowerCase());
const isHost = (value) => String(value).toLowerCase() === 'host';

const SETTINGS = [
  { key: 'privileged', hit: isTruthy,
    reason: 'hands the container every capability the host has' },
  { key: 'network_mode', hit: isHost,
    reason: 'puts the container on the host\'s network stack' },
  { key: 'pid', hit: isHost,
    reason: 'lets the container see and signal host processes' },
  { key: 'ipc', hit: isHost,
    reason: 'shares host shared memory' },
];

const SOCKET_KEY = 'volumes';
const SOCKET_PATH = '/var/run/docker.sock';
const SOCKET_REASON = 'the Docker socket is root on the host, and a mount hands it over';

/** Resolve an alias to its target; any other node comes back unchanged. */
function deref(node, doc) {
  let current = node;
  for (let hop = 0; hop < 16 && current && current.resolve; hop += 1) {
    const next = current.resolve(doc);
    if (next === current) break;
    current = next;
  }
  return current;
}

/** Map of service name -> { pair, map } for the document's `services:`. */
function servicesOf(doc) {
  const services = doc.get('services', true);
  if (!isMap(services)) return new Map();
  const byName = new Map();
  for (const pair of services.items) {
    const name = String(pair.key?.value ?? '');
    const map = deref(pair.value, doc);
    byName.set(name, { pair, map: isMap(map) ? map : null });
  }
  return byName;
}

/** The Pair for `key` in a service mapping, or undefined when it came from a merge. */
function pairFor(service, key) {
  if (!service.map) return undefined;
  return service.map.items.find((item) => String(item.key?.value ?? '') === key);
}

/** True when the comment block directly above `pair` carries the marker. */
function exempted(pair) {
  if (!pair) return false;
  const comment = pair.key?.commentBefore ?? pair.key?.comment ?? pair.commentBefore ?? '';
  return String(comment).includes(MARKER);
}

/** Source line of a node, falling back to the service's own key. */
function lineOf(node, service, lineCounter) {
  const offset = node?.key?.range?.[0] ?? node?.range?.[0] ?? service?.pair?.key?.range?.[0];
  return offset == null ? 1 : lineCounter.linePos(offset).line;
}

function socketsIn(value) {
  if (typeof value === 'string') return value.includes(SOCKET_PATH);
  if (Array.isArray(value)) return value.some(socketsIn);
  if (value && typeof value === 'object') return socketsIn(value.source ?? value.target ?? '');
  return false;
}

function scanText(text, label) {
  const lineCounter = new LineCounter();
  const doc = parseDocument(text, { lineCounter, merge: true });
  if (doc.errors.length > 0) {
    return { error: `${label}: not valid YAML — ${doc.errors[0].message}` };
  }

  const services = servicesOf(doc);
  const resolved = doc.toJS()?.services;
  if (!resolved || typeof resolved !== 'object') return { findings: [] };

  const findings = [];
  for (const [name, service] of Object.entries(resolved)) {
    if (!service || typeof service !== 'object') continue;
    const located = services.get(name);

    for (const setting of SETTINGS) {
      if (!setting.hit(service[setting.key])) continue;
      const pair = pairFor(located, setting.key);
      if (exempted(pair)) continue;
      const where = pair ? undefined : ' (reached through a merge key or alias)';
      findings.push({
        line: lineOf(pair, located, lineCounter),
        text: `${setting.key}: ${service[setting.key]}`,
        reason: `${setting.reason}${where}`,
      });
    }

    if (socketsIn(service[SOCKET_KEY])) {
      const pair = pairFor(located, SOCKET_KEY);
      if (!exempted(pair)) {
        const where = pair ? '' : ' (reached through a merge key or alias)';
        findings.push({
          line: lineOf(pair, located, lineCounter),
          text: `${SOCKET_KEY}: ${SOCKET_PATH}${where}`,
          reason: SOCKET_REASON,
        });
      }
    }
  }
  findings.sort((a, b) => a.line - b.line);
  return { findings };
}

function composeFiles(directory) {
  const found = [];
  // By name, not by directory: deploy/helm holds Go-templated chart files that
  // are not valid YAML until rendered, and they are not Compose. Every compose
  // file in this tree carries `compose` in its basename.
  const walk = (current) => {
    for (const name of readdirSync(current)) {
      const path = join(current, name);
      if (statSync(path).isDirectory()) walk(path);
      else if (/\.ya?ml$/.test(name) && /compose/i.test(name)) found.push(path);
    }
  };
  walk(directory);
  return found.sort();
}

function run(directory) {
  const files = composeFiles(directory);
  let total = 0;
  for (const path of files) {
    const label = relative(ROOT, path).split(sep).join('/');
    const result = scanText(readFileSync(path, 'utf8'), label);
    if (result.error) {
      process.stderr.write(`${result.error}\n`);
      total += 1;
      continue;
    }
    for (const finding of result.findings) {
      process.stderr.write(`${label}:${finding.line}: ${finding.text}\n`);
      process.stderr.write(
        `    It ${finding.reason}. Remove it, or record why with ` +
        `\`# ${MARKER}: ...\` above it.\n`,
      );
      total += 1;
    }
  }
  if (total === 0) {
    process.stdout.write(`compose hardening: clean (${files.length} compose file(s) under ${relative(ROOT, directory) || '.'}/)\n`);
    return 0;
  }
  process.stderr.write(`compose hardening: ${total} finding(s)\n`);
  return 1;
}

const FIXTURES = {
  clean: `services:
  web:
    image: alpine:3.20
    read_only: true
    security_opt: ["no-new-privileges"]
    # a comment mentioning privileged: true sets nothing
`,
  exempt: `services:
  proxy:
    image: alpine:3.20
    # EXEMPTION (compose): this fixture binds the host network on purpose.
    network_mode: host
`,
  bad: `services:
  escape:
    image: alpine:3.20
    privileged: true
    network_mode: host
    pid: host
    ipc: host
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
`,
  // The three spellings the old scanner missed.
  inlineComment: `services:
  escape:
    image: alpine:3.20
    privileged: true   # required temporarily
`,
  flowStyle: `services: {web: {image: alpine:3.20, privileged: true}}
`,
  quotedKey: `services:
  escape:
    image: alpine:3.20
    "network_mode": "host"
`,
  // Command text is not configuration.
  commandText: `services:
  web:
    image: alpine:3.20
    command: ["echo", "privileged: true is forbidden"]
  other:
    image: alpine:3.20
    command:
      - "privileged: true"
`,
  // The case the second scanner missed: quoted text first, a real setting after.
  mixed: `services: {sneaky: {command: ["privileged: true is forbidden"], privileged: true}}
`,
  // A setting that only exists after the merge key is resolved.
  merged: `services:
  shared: &base
    image: alpine:3.20
    privileged: true
  web:
    <<: *base
`,
  // A marker one line above a different key must not cover this one.
  staleMarker: `services:
  web:
    image: alpine:3.20
    # EXEMPTION (compose): stale marker
    command: ["true"]
    privileged: true
`,
};

const EXPECTED = {
  clean: 0,
  exempt: 0,
  bad: 5,
  inlineComment: 1,
  flowStyle: 1,
  quotedKey: 1,
  commandText: 0,
  mixed: 1,
  // `shared` sets it for real, and `web` inherits it through the merge key.
  merged: 2,
  staleMarker: 1,
};

function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), 'compose-hardening-'));
  const failures = [];
  try {
    for (const [name, body] of Object.entries(FIXTURES)) {
      const path = join(dir, `${name}.yml`);
      writeFileSync(path, body);
      const result = scanText(readFileSync(path, 'utf8'), `${name}.yml`);
      const count = result.error ? `error: ${result.error}` : result.findings.length;
      if (count !== EXPECTED[name]) {
        failures.push(`${name}: reported ${count}, expected ${EXPECTED[name]}`);
      }
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  for (const failure of failures) process.stderr.write(`FAIL: ${failure}\n`);
  if (failures.length > 0) return 1;
  process.stdout.write(`compose hardening self-test: ${Object.keys(FIXTURES).length} fixtures behave as intended\n`);
  return 0;
}

const argv = process.argv.slice(2);
if (argv.includes('--self-test')) process.exit(selfTest());
const target = argv.find((arg) => !arg.startsWith('-'));
process.exit(run(target ? resolve(target) : DEFAULT_SCAN_DIR));
