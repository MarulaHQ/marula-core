// Prune the pnpm store down to the packages the standalone server resolves.
//
// `next build` writes .next/standalone with node_modules entries that are
// symlinks into apps/web/node_modules/.pnpm, and deploy/web-standalone-remap.cjs
// re-anchors them onto the full root store at /repo/node_modules/.pnpm.
// Dockerfile.web then copies that whole store into the runtime image so the
// symlinks resolve — which also copies everything the build needed and the
// server never loads: both libc builds of @next/swc, turbopack, typescript,
// the Tauri CLI, and their platform binaries.
//
// This walks the symlinks from the standalone tree to a fixed point, then
// deletes the store entries nothing points at. Nothing moves, so every
// surviving link keeps its absolute target and still resolves.
//
// Usage: node prune-pnpm-store.cjs [standaloneDir] [storeDir]

const fs = require("node:fs");
const path = require("node:path");

const standalone = process.argv[2] || "/repo/apps/web/.next/standalone";
const store = path.resolve(process.argv[3] || "/repo/node_modules/.pnpm");

// A store entry is the path segment right after `.pnpm/`, e.g.
// `next@15.5.28_...` or `@next+swc-linux-arm64-gnu@15.5.28`. Names that are
// not package entries (`.pnpm/node_modules`, `lock.yaml`) are never pruned:
// leaving them costs nothing and keeps pnpm's own layout intact.
function entryOf(absTarget) {
  const rel = path.relative(store, absTarget);
  if (rel.startsWith("..") || path.isAbsolute(rel)) return null;
  const first = rel.split(path.sep)[0];
  if (!first || first.startsWith(".") || first === "node_modules") return null;
  return first;
}

// Collect the store entries referenced by symlinks anywhere under `dir`.
// Packages link their dependencies as siblings inside their own
// node_modules/, so each kept entry is scanned there rather than across its
// whole tree.
function collectLinks(dir, into) {
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const e of entries) {
    const p = path.join(dir, e.name);
    if (e.isSymbolicLink()) {
      const name = entryOf(path.resolve(dir, fs.readlinkSync(p)));
      if (name) into.add(name);
    } else if (e.isDirectory()) {
      collectLinks(p, into);
    }
  }
}

function dirSize(dir) {
  let total = 0;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isSymbolicLink()) continue;
    if (e.isDirectory()) total += dirSize(p);
    else total += fs.statSync(p).size;
  }
  return total;
}

if (!fs.existsSync(standalone)) {
  console.error(`prune-pnpm-store: standalone tree not found at ${standalone}`);
  process.exit(1);
}
if (!fs.existsSync(store)) {
  console.error(`prune-pnpm-store: pnpm store not found at ${store}`);
  process.exit(1);
}

const keep = new Set();
collectLinks(standalone, keep);

// Walk the dependency links of everything kept until nothing new appears.
// The queue can grow while it drains, so it is not a for..of.
const queue = [...keep];
while (queue.length) {
  const entry = queue.pop();
  const before = keep.size;
  collectLinks(path.join(store, entry, "node_modules"), keep);
  if (keep.size !== before) queue.push(...[...keep].slice(before));
}

let removed = 0;
let freed = 0;
let kept = 0;
for (const e of fs.readdirSync(store, { withFileTypes: true })) {
  if (!e.isDirectory() || e.isSymbolicLink()) continue;
  if (e.name.startsWith(".") || e.name === "node_modules") continue;
  const p = path.join(store, e.name);
  if (keep.has(e.name)) {
    kept += 1;
    continue;
  }
  freed += dirSize(p);
  fs.rmSync(p, { recursive: true, force: true });
  removed += 1;
}

const mb = (n) => (n / 1e6).toFixed(1);
console.log(
  `prune-pnpm-store: kept ${kept} entries, removed ${removed} (${mb(freed)} MB freed)`,
);

// Report links the prune left dangling. Next's tracing records some optional
// platform packages that are legitimately absent, so this is a warning rather
// than a failure — but an empty list is the expected shape, and a long one
// means the closure walk under-collected and the image is short a dependency.
const dangling = [];
(function check(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isSymbolicLink()) {
      const target = path.resolve(dir, fs.readlinkSync(p));
      if (!fs.existsSync(target)) dangling.push(path.relative(standalone, p));
    } else if (e.isDirectory()) {
      check(p);
    }
  }
})(standalone);
if (dangling.length) {
  console.warn(
    `prune-pnpm-store: WARN ${dangling.length} standalone link(s) do not resolve:`,
  );
  for (const d of dangling.slice(0, 20)) console.warn(`  ${d}`);
} else {
  console.log("prune-pnpm-store: every standalone link resolves");
}
