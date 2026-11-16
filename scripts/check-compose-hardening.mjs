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
// COMPOSE COMPOSITION
// -------------------
// A service's settings are not always written where the service is. Compose
// resolves three indirections before a container exists, and this checker
// follows all three:
//
//   extends: {file: base.yml, service: base}  inherits from another file
//   extends: base                             inherits from this file
//   include: [other.yml]                      adds whole files to the project
//   include: [{path: [a.yml, b.yml]}]         adds an ordered merge model
//
// Resolution is recursive (a base can extend a base), memoised, and guarded
// against cycles. Each finding is reported in the file where the setting is
// written, not in the file that pulled it in, because that is where a reader
// has to change it, and the message names the file that reached it.
//
// The merge follows Compose: a local setting wins over an inherited one, and
// `volumes` is keyed by container target, so a local mount replaces an inherited
// one that lands on the same target. Each effective mount keeps its own origin,
// so an exemption written above a local `volumes:` speaks only for the mounts in
// that list, never for one inherited from a base.
//
// A reference that cannot be resolved, or a cycle in either `extends` or
// `include`, is a finding: a config this check cannot read is not a config
// known to be safe.
//
// WHAT IT DOES NOT EVALUATE
// -------------------------
// A `${VAR}` or `${VAR:-default}` in a protected setting is reported, not
// substituted. Compose resolves it before the container exists, so
// `network_mode: "${NET_MODE:-host}"` is host networking wherever NET_MODE is
// unset, and reads as an ordinary string here. A value this file cannot know is
// not a value known to be safe.
//
// EXEMPTIONS
// ----------
// A setting is exempt when the comment directly above its key carries
// `EXEMPTION (compose)` with a reason. The parser attaches exactly that comment
// block to the key, so a marker higher up the file cannot drift onto a setting
// somebody adds later. That is the convention the files already use for
// `EXEMPTION (read_only)`.
//
// USAGE
//   node scripts/check-compose-hardening.mjs             # scan deploy/
//   node scripts/check-compose-hardening.mjs --self-test # prove it can fail

import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { LineCounter, isMap, parseDocument } from 'yaml';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const DEFAULT_SCAN_DIR = join(ROOT, 'deploy');
const MARKER = 'EXEMPTION (compose)';
const COMPOSE_NAMES = ['compose.yaml', 'compose.yml', 'docker-compose.yaml', 'docker-compose.yml'];

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
const SOCKET_REASON = 'mounts the Docker socket, which is root on the host';

const rel = (path) =>
  path.startsWith(ROOT + sep) ? relative(ROOT, path).split(sep).join('/') : path;

/** A failure that belongs in the report rather than in a stack trace. */
class Refusal extends Error {}

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

/** The Pair for `key` in a mapping, or undefined. */
function pairInMap(map, key) {
  if (!isMap(map)) return undefined;
  return map.items.find((item) => String(item.key?.value ?? '') === key);
}

/** True when the comment block directly above `pair` carries the marker. */
function exempted(pair) {
  if (!pair) return false;
  const comment = pair.key?.commentBefore ?? pair.key?.comment ?? pair.commentBefore ?? '';
  return String(comment).includes(MARKER);
}

function lineOf(node, lineCounter) {
  const offset = node?.key?.range?.[0] ?? node?.range?.[0];
  return offset == null ? 1 : lineCounter.linePos(offset).line;
}

function socketsIn(value) {
  if (typeof value === 'string') return value.includes(SOCKET_PATH);
  if (Array.isArray(value)) return value.some(socketsIn);
  if (value && typeof value === 'object') return socketsIn(value.source ?? value.target ?? '');
  return false;
}

/**
 * Find the file a reference names. A directory means the compose file inside
 * it, which is what `include: []` and a bare `extends` file both accept.
 */
function resolveRef(fromPath, reference) {
  const base = resolve(dirname(fromPath), reference);
  if (existsSync(base) && statSync(base).isDirectory()) {
    for (const name of COMPOSE_NAMES) {
      const candidate = join(base, name);
      if (existsSync(candidate)) return candidate;
    }
    throw new Refusal(`includes a directory with no compose file in it (${reference})`);
  }
  if (!existsSync(base)) throw new Refusal(`refers to a file that does not exist (${reference})`);
  return base;
}

/** Minimal glob: a `*` matches within a segment and `**` crosses segments. */
function expandGlob(fromPath, pattern) {
  if (!pattern.includes('*')) return [resolveRef(fromPath, pattern)];
  const base = resolve(dirname(fromPath), pattern);
  const segments = base.split(sep);
  const firstWild = segments.findIndex((segment) => segment.includes('*'));
  const root = segments.slice(0, firstWild).join(sep) || sep;
  const rest = segments.slice(firstWild);
  const matcher = new RegExp(`^${rest.join('/')
    .replace(/[.+^${}()|[\]\\]/g, '\\$&')
    .replace(/\*\*/g, '\u0000')
    .replace(/\*/g, '[^/]*')
    .replace(/\u0000/g, '.*')}$`);
  const hits = [];
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      if (name === 'node_modules' || name.startsWith('.')) continue;
      const path = join(dir, name);
      const tail = path.slice(root.length + 1).split(sep).join('/');
      if (statSync(path).isDirectory()) walk(path);
      else if (matcher.test(tail)) hits.push(path);
    }
  };
  walk(root);
  return hits.sort();
}

/** Compose file paths a document includes. */
function includesOf(doc, path) {
  const raw = doc.toJS()?.include;
  if (raw === undefined) return [];
  const entries = Array.isArray(raw) ? raw : [raw];
  const specs = [];
  for (const entry of entries) {
    // `include:` takes a path, a list of paths (one merged application model),
    // or a map whose `path` is either of those.
    const spec = typeof entry === 'string' ? entry : entry?.path;
    if (typeof spec === 'string') specs.push(spec);
    else if (Array.isArray(spec) && spec.every((item) => typeof item === 'string')) specs.push(...spec);
    else throw new Refusal('has an `include:` entry with no path');
  }
  const paths = [];
  for (const spec of specs) {
    for (const hit of expandGlob(path, spec)) paths.push(hit);
  }
  return paths;
}

/** Reference a service inherits from, or undefined. */
function extendsOf(config, path) {
  const raw = config?.extends;
  if (raw === undefined) return undefined;
  if (typeof raw === 'string') return { path, name: raw };
  if (raw && typeof raw === 'object' && typeof raw.service === 'string') {
    return {
      path: typeof raw.file === 'string' ? resolveRef(path, raw.file) : path,
      name: raw.service,
    };
  }
  throw new Refusal('has an `extends:` that names no service');
}

/** One loaded document, with the node tree kept for lines and comments. */
function load(path, cache) {
  if (cache.has(path)) return cache.get(path);
  const lineCounter = new LineCounter();
  const doc = parseDocument(readFileSync(path, 'utf8'), { lineCounter, merge: true });
  if (doc.errors.length > 0) {
    throw new Refusal(`is not valid YAML — ${doc.errors[0].message.split('\n')[0]}`);
  }
  const configs = doc.toJS()?.services;
  const servicesNode = deref(doc.get('services', true), doc);
  const index = new Map();
  if (isMap(servicesNode)) {
    for (const pair of servicesNode.items) {
      index.set(String(pair.key?.value ?? ''), { pair, map: deref(pair.value, doc) });
    }
  }
  const entry = {
    path,
    doc,
    lineCounter,
    configs: configs && typeof configs === 'object' ? configs : {},
    index,
  };
  cache.set(path, entry);
  return entry;
}

function asArray(value) {
  if (value === undefined || value === null) return [];
  return Array.isArray(value) ? value : [value];
}

/**
 * The container target of a volume, which is what Compose keys mounts by.
 *
 * Short syntax is `[source:]target[:mode]`, so one segment is a target, two
 * make the second the target, and three or more keep the second. Long syntax
 * names it.
 */
function volumeTarget(mount) {
  if (typeof mount === 'string') {
    const parts = mount.split(':');
    return parts.length === 1 ? parts[0] : parts[1];
  }
  if (mount && typeof mount === 'object' && typeof mount.target === 'string') return mount.target;
  return undefined;
}

/**
 * Merge volumes the way Compose does: a local mount replaces an inherited one
 * with the same container target, and the origin follows the effective mount.
 * Concatenating instead would report a socket the local file deliberately
 * replaced.
 */
function mergeVolumes(inherited, inheritedOrigins, local, localOrigin) {
  const volumes = [...inherited];
  const volumeOrigins = [...inheritedOrigins];
  for (const mount of local) {
    const target = volumeTarget(mount);
    const at = target === undefined
      ? -1
      : volumes.findIndex((candidate) => volumeTarget(candidate) === target);
    if (at >= 0) {
      volumes[at] = mount;
      volumeOrigins[at] = localOrigin;
    } else {
      volumes.push(mount);
      volumeOrigins.push(localOrigin);
    }
  }
  return { volumes, volumeOrigins };
}

/**
 * A service's effective settings, plus where each protected key was written.
 *
 * `extends` inherits first and the local file wins, except `volumes`, which
 * Compose appends to. Volume provenance is tracked per mount rather than per
 * key, because one list can carry a local mount and an inherited one, and an
 * exemption above the local `volumes:` must not speak for the inherited mount.
 */
function resolveService(path, name, cache, stack = []) {
  const key = `${path}::${name}`;
  if (stack.includes(key)) throw new Refusal(`extends itself in a cycle (${name})`);
  const file = load(path, cache);
  const own = file.configs[name];
  if (!own) throw new Refusal(`names a service that does not exist (${name})`);

  let config = {};
  const origin = new Map();
  let inheritedVolumes = [];
  let inheritedVolumeOrigins = [];
  const parent = extendsOf(own, path);
  if (parent) {
    const base = resolveService(parent.path, parent.name, cache, [...stack, key]);
    config = { ...base.config };
    delete config[SOCKET_KEY];
    for (const [setting, where] of base.origin) origin.set(setting, where);
    inheritedVolumes = base.volumes;
    inheritedVolumeOrigins = base.volumeOrigins;
  }

  const located = file.index.get(name);
  const localVolumes = asArray(own[SOCKET_KEY]);
  const localVolumeOrigin = {
    path,
    pair: pairInMap(located?.map, SOCKET_KEY),
    lineCounter: file.lineCounter,
  };

  for (const [setting, value] of Object.entries(own)) {
    if (setting === 'extends' || setting === SOCKET_KEY) continue;
    config[setting] = value;
    if (SETTINGS.some((candidate) => candidate.key === setting)) {
      origin.set(setting, {
        path,
        pair: pairInMap(located?.map, setting),
        lineCounter: file.lineCounter,
      });
    }
  }

  const volumes = mergeVolumes(inheritedVolumes, inheritedVolumeOrigins, localVolumes, localVolumeOrigin);
  if (volumes.volumes.length > 0) config[SOCKET_KEY] = volumes.volumes;

  return {
    config,
    origin,
    volumes: volumes.volumes,
    volumeOrigins: volumes.volumeOrigins,
  };
}

/** Every compose file reachable from `path` through `include:`, recursively. */
function projectFiles(path, cache, seen = new Set(), stack = [], onError = () => {}) {
  // A file that includes one of its own ancestors is a cycle. A file reached
  // twice without being an ancestor is a diamond, which is fine and is scanned
  // once — `seen` handles that, the stack handles the cycle.
  if (stack.includes(path)) {
    onError(stack[stack.length - 1] ?? path, new Refusal(`includes ${rel(path)} in a cycle`));
    return seen;
  }
  if (seen.has(path)) return seen;
  seen.add(path);
  let included = [];
  try {
    included = includesOf(load(path, cache).doc, path);
  } catch (error) {
    if (!(error instanceof Refusal)) throw error;
    onError(path, error);
    return seen;
  }
  for (const next of included) {
    try {
      load(next, cache);
    } catch (error) {
      if (!(error instanceof Refusal)) throw error;
      onError(path, error);
      continue;
    }
    projectFiles(next, cache, seen, [...stack, path], onError);
  }
  return seen;
}

function scanFile(path, cache, findings, reported) {
  const file = load(path, cache);
  const report = (kind, target, line, text, reason) => {
    const dedupe = `${kind}:${target}:${line}:${text}:${reason}`;
    if (reported.has(dedupe)) return;
    reported.add(dedupe);
    findings.push({ kind, path: target, line, text, reason });
  };

  for (const name of Object.keys(file.configs)) {
    let resolvedService;
    try {
      resolvedService = resolveService(path, name, cache);
    } catch (error) {
      if (!(error instanceof Refusal)) throw error;
      // A config this check cannot read is not a config known to be safe.
      report('reference', path, lineOf(file.index.get(name)?.pair, file.lineCounter), '', error.message);
      continue;
    }
    const { config, origin } = resolvedService;
    for (const setting of SETTINGS) {
      const value = config[setting.key];
      if (value === undefined || value === null) continue;
      const substitution = typeof value === 'string' && value.includes('${');
      if (!substitution && !setting.hit(value)) continue;
      const where = origin.get(setting.key) ?? { path, pair: undefined, lineCounter: file.lineCounter };
      if (exempted(where.pair)) continue;
      const through = where.path === path ? '' : ` (reached from ${rel(path)})`;
      report(
        'setting',
        where.path,
        lineOf(where.pair, where.lineCounter),
        `${setting.key}: ${value}`,
        (substitution
          ? 'is a Compose substitution, which this check cannot evaluate'
          : setting.reason) + through,
      );
    }

    const volumes = config[SOCKET_KEY] ?? [];
    const volumeOrigins = resolvedService.volumeOrigins ?? [];
    volumes.forEach((mount, index) => {
      if (!socketsIn(mount)) return;
      // Each mount answers to its own origin: an exemption above the local
      // `volumes:` says nothing about a mount that came from the base.
      const where = volumeOrigins[index] ?? { path, pair: undefined, lineCounter: file.lineCounter };
      if (exempted(where.pair)) return;
      const through = where.path === path ? '' : ` (reached from ${rel(path)})`;
      report('setting', where.path, lineOf(where.pair, where.lineCounter),
        `${SOCKET_KEY}: ${SOCKET_PATH}`, SOCKET_REASON + through);
    });
  }
}

function composeFiles(directory) {
  const found = [];
  // By name, not by directory: deploy/helm holds Go-templated chart files that
  // are not valid YAML until rendered, and they are not Compose.
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

function scan(directory) {
  const cache = new Map();
  const findings = [];
  const reported = new Set();
  const scanned = new Set();

  for (const root of composeFiles(directory)) {
    const roots = projectFiles(root, cache, new Set(), [], (from, error) => {
      findings.push({ kind: 'reference', path: from, line: 1, text: '', reason: error.message });
    });
    for (const path of roots) {
      // A file reachable twice (two roots including it, or a chain) is scanned
      // once; `reported` also dedupes a setting that is both written and
      // inherited inside the same project.
      if (scanned.has(path)) continue;
      scanned.add(path);
      try {
        scanFile(path, cache, findings, reported);
      } catch (error) {
        if (!(error instanceof Refusal)) throw error;
        findings.push({ kind: 'reference', path, line: 1, text: '', reason: error.message });
      }
    }
  }
  findings.sort((a, b) => a.path.localeCompare(b.path) || a.line - b.line);
  return { findings, scanned: scanned.size };
}

function run(directory) {
  const { findings, scanned } = scan(directory);
  for (const finding of findings) {
    process.stderr.write(`${rel(finding.path)}:${finding.line}: ${finding.text || 'reference'}\n`);
    if (finding.kind === 'reference') {
      // An unreadable reference cannot be exempted; it has to be fixed.
      process.stderr.write(`    It ${finding.reason}.\n`);
    } else {
      process.stderr.write(
        `    It ${finding.reason}. Remove it, or record why with \`# ${MARKER}: ...\` above it.\n`,
      );
    }
  }
  if (findings.length === 0) {
    process.stdout.write(
      `compose hardening: clean (${scanned} compose file(s) under ${rel(directory) || '.'}/)\n`,
    );
    return 0;
  }
  process.stderr.write(`compose hardening: ${findings.length} finding(s)\n`);
  return 1;
}

const FIXTURES = {
  clean: { 'docker-compose.yml': `services:
  web:
    image: alpine:3.20
    read_only: true
    security_opt: ["no-new-privileges"]
    # a comment mentioning privileged: true sets nothing
` },
  exempt: { 'docker-compose.yml': `services:
  proxy:
    image: alpine:3.20
    # EXEMPTION (compose): this fixture binds the host network on purpose.
    network_mode: host
` },
  bad: { 'docker-compose.yml': `services:
  escape:
    image: alpine:3.20
    privileged: true
    network_mode: host
    pid: host
    ipc: host
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
` },
  inlineComment: { 'docker-compose.yml': `services:
  escape:
    image: alpine:3.20
    privileged: true   # required temporarily
` },
  flowStyle: { 'docker-compose.yml': `services: {web: {image: alpine:3.20, privileged: true}}
` },
  quotedKey: { 'docker-compose.yml': `services:
  escape:
    image: alpine:3.20
    "network_mode": "host"
` },
  commandText: { 'docker-compose.yml': `services:
  web:
    image: alpine:3.20
    command: ["echo", "privileged: true is forbidden"]
  other:
    image: alpine:3.20
    command:
      - "privileged: true"
` },
  mixed: { 'docker-compose.yml': `services: {sneaky: {command: ["privileged: true is forbidden"], privileged: true}}
` },
  merged: { 'docker-compose.yml': `services:
  shared: &base
    image: alpine:3.20
    privileged: true
  web:
    <<: *base
` },
  staleMarker: { 'docker-compose.yml': `services:
  web:
    image: alpine:3.20
    # EXEMPTION (compose): stale marker
    command: ["true"]
    privileged: true
` },
  aliasedServices: { 'docker-compose.yml': `x-services: &all
  web:
    image: alpine:3.20
    # EXEMPTION (compose): this proxy needs host networking.
    network_mode: host
services: *all
` },
  substitution: { 'docker-compose.yml': `services:
  web:
    image: alpine:3.20
    network_mode: "\${NET_MODE:-host}"
` },
  benignSubstitution: { 'docker-compose.yml': `services:
  web:
    image: alpine:3.20
    privileged: "\${PRIV:-false}"
` },
  // extends across files: the setting is written in base.yml and reported there.
  extendsFile: {
    'docker-compose.yml': `services:
  web:
    image: alpine:3.20
    extends:
      file: base.yml
      service: base
`,
    'base.yml': `services:
  base:
    image: alpine:3.20
    privileged: true
`,
  },
  // A chain, two files deep.
  extendsChain: {
    'docker-compose.yml': `services:
  web:
    extends: {file: mid.yml, service: mid}
`,
    'mid.yml': `services:
  mid:
    extends: {file: deep.yml, service: deep}
`,
    'deep.yml': `services:
  deep:
    image: alpine:3.20
    pid: host
`,
  },
  // Same file, by name.
  extendsSameFile: { 'docker-compose.yml': `services:
  base:
    image: alpine:3.20
    privileged: true
  web:
    extends: base
` },
  // The exemption lives where the setting is written, and still applies.
  extendsExempt: {
    'docker-compose.yml': `services:
  web:
    image: alpine:3.20
    extends: {file: base.yml, service: base}
`,
    'base.yml': `services:
  base:
    image: alpine:3.20
    # EXEMPTION (compose): base.yml reviewed, the proxy binds 80/443.
    network_mode: host
`,
  },
  // include: another file, and a directory.
  includeFile: {
    'docker-compose.yml': `include:
  - extra.yml
services:
  web:
    image: alpine:3.20
`,
    'extra.yml': `services:
  extra:
    image: alpine:3.20
    privileged: true
`,
  },
  includeDirectory: {
    'docker-compose.yml': `include:
  - sub
services:
  web:
    image: alpine:3.20
`,
    'sub/compose.yaml': `services:
  nested:
    image: alpine:3.20
    ipc: host
`,
  },
  // A setting written locally wins over an inherited one. `base.yml` is a
  // fragment, not a project entry, so its unattached service is not deployable
  // on its own and is not reported.
  extendsLocalWins: {
    'docker-compose.yml': `services:
  web:
    extends: {file: base.yml, service: base}
    privileged: false
`,
    'base.yml': `services:
  base:
    privileged: true
`,
  },
  // References that cannot be read are findings, not silence.
  missingExtendsFile: { 'docker-compose.yml': `services:
  web:
    extends: {file: nowhere.yml, service: base}
` },
  extendsCycle: {
    'docker-compose.yml': `services:
  a:
    extends: {file: b.yml, service: b}
`,
    'b.yml': `services:
  b:
    extends: {file: docker-compose.yml, service: a}
`,
  },
  // An exemption above the LOCAL volumes: must not speak for a mount that came
  // from the base. The inherited mount is still reported, in the base file.
  inheritedSocketExemptLocal: {
    'docker-compose.yml': `services:
  web:
    extends: {file: base.yml, service: base}
    # EXEMPTION (compose): local volumes reviewed, nothing here
    volumes: []
`,
    'base.yml': `services:
  base:
    image: alpine:3.20
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
`,
  },
  // ... while an exemption above the base's own volumes: does apply.
  inheritedSocketExemptBase: {
    'docker-compose.yml': `services:
  web:
    extends: {file: base.yml, service: base}
    volumes: []
`,
    'base.yml': `services:
  base:
    image: alpine:3.20
    # EXEMPTION (compose): base reviewed, the socket mount is required.
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
`,
  },
  // A file whose only content is `include:` — no local services at all.
  includeOnly: {
    'docker-compose.yml': `include:
  - extra.yml
`,
    'extra.yml': `services:
  extra:
    image: alpine:3.20
    privileged: true
`,
  },
  // Included services merged alongside local ones: both are part of the
  // project, so both are scanned.
  mergedInclude: {
    'docker-compose.yml': `include: [extra.yml]
services:
  web:
    image: alpine:3.20
`,
    'extra.yml': `services:
  extra:
    image: alpine:3.20
    network_mode: host
`,
  },
  // include of a file that includes a third.
  nestedInclude: {
    'docker-compose.yml': `include: [mid.yml]
`,
    'mid.yml': `include: [deep.yml]
`,
    'deep.yml': `services:
  deep:
    image: alpine:3.20
    ipc: host
`,
  },
  // include cycles are findings, like extends cycles.
  includeCycle: {
    'docker-compose.yml': `include: [other.yml]
services:
  web:
    image: alpine:3.20
`,
    'other.yml': `include: [docker-compose.yml]
services:
  other:
    image: alpine:3.20
`,
  },
  // include supplied through a YAML merge key at the document root.
  mergedIncludeYamlMerge: {
    'docker-compose.yml': `x-comp: &comp
  include:
    - extra.yml
<<: *comp
services:
  web:
    image: alpine:3.20
`,
    'extra.yml': `services:
  extra:
    image: alpine:3.20
    privileged: true
`,
  },
  // `include:` with an ordered path list, which is one merged model.
  includePathList: {
    'docker-compose.yml': `include:
  - path: [one.yml, two.yml]
services:
  web:
    image: alpine:3.20
`,
    'one.yml': `services:
  one:
    image: alpine:3.20
`,
    'two.yml': `services:
  two:
    image: alpine:3.20
    pid: host
`,
  },
  // An exempted base plus a local socket mount at a different target: the
  // base's exemption speaks for the base's mount, not this one.
  baseExemptLocalSocket: {
    'docker-compose.yml': `services:
  web:
    extends: {file: base.yml, service: base}
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
`,
    'base.yml': `services:
  base:
    image: alpine:3.20
    # EXEMPTION (compose): base reviewed, /cache is a tmpfs.
    volumes:
      - /tmp/cache:/cache
`,
  },
  // A local mount with the same container target replaces the inherited one,
  // so the socket in the base never reaches a container.
  volumeTargetOverride: {
    'docker-compose.yml': `services:
  web:
    extends: {file: base.yml, service: base}
    volumes:
      - /tmp/empty:/socket
`,
    'base.yml': `services:
  base:
    image: alpine:3.20
    volumes:
      - /var/run/docker.sock:/socket
`,
  },
};

const EXPECTED = {
  clean: 0, exempt: 0, bad: 5, inlineComment: 1, flowStyle: 1, quotedKey: 1,
  commandText: 0, mixed: 1, merged: 2, staleMarker: 1, aliasedServices: 0,
  substitution: 1, benignSubstitution: 1,
  extendsFile: 1, extendsChain: 1, extendsSameFile: 1,
  extendsExempt: 0, includeFile: 1, includeDirectory: 1,
  extendsLocalWins: 0, missingExtendsFile: 1, extendsCycle: 1,
  inheritedSocketExemptLocal: 1, inheritedSocketExemptBase: 0,
  includeOnly: 1, mergedInclude: 1, nestedInclude: 1, includeCycle: 1,
  mergedIncludeYamlMerge: 1, includePathList: 1,
  baseExemptLocalSocket: 1, volumeTargetOverride: 0,
};

function selfTest() {
  const failures = [];
  for (const [name, files] of Object.entries(FIXTURES)) {
    const dir = mkdtempSync(join(tmpdir(), 'compose-hardening-'));
    try {
      for (const [file, body] of Object.entries(files)) {
        const target = join(dir, file);
        mkdirp(dirname(target));
        writeFileSync(target, body);
      }
      const { findings } = scan(dir);
      if (findings.length !== EXPECTED[name]) {
        const detail = findings.map((f) => `${relative(dir, f.path)}:${f.line}`).join(', ');
        failures.push(`${name}: reported ${findings.length} (${detail || 'none'}), expected ${EXPECTED[name]}`);
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }
  for (const failure of failures) process.stderr.write(`FAIL: ${failure}\n`);
  if (failures.length > 0) return 1;
  process.stdout.write(
    `compose hardening self-test: ${Object.keys(FIXTURES).length} fixtures behave as intended\n`,
  );
  return 0;
}

function mkdirp(dir) {
  if (existsSync(dir)) return;
  mkdirp(dirname(dir));
  mkdirSync(dir);
}

const argv = process.argv.slice(2);
if (argv.includes('--self-test')) process.exit(selfTest());
const target = argv.find((arg) => !arg.startsWith('-'));
process.exit(run(target ? resolve(target) : DEFAULT_SCAN_DIR));
