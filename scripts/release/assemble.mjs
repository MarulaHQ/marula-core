#!/usr/bin/env node
/**
 * assemble.mjs — turn a built tree into the directory a release ships.
 *
 * Wave 60 lane W60-4 (`p60-installer-artifact`). One release leg of the
 * matrix builds two things (the `marula` CLI and the installer app) and this
 * script lays them out the way the rest of the release path expects:
 *
 *   dist/release/marula-<version>-<os>-<arch>/
 *     marula                     the CLI (marula.exe on Windows)
 *     installers/<kind>/…        the Tauri bundles the platform produced
 *     apps/installer/dist/       the wizard front-end, at the path
 *                                `marula install --ui` looks in by default
 *     deploy/                    the compose files and everything they mount
 *     LICENSE, README.md         what the AGPL asks a distribution to carry
 *     release.json               what this directory is, for a machine
 *     SHA256SUMS                 every file above, sorted, `sha256sum -c` shaped
 *
 * and, beside the directory, the two files the download path needs:
 *
 *   marula-<version>-<os>-<arch>.tar.gz   the directory, archived
 *   SHA256SUMS-<os>-<arch>                one line for that archive, for the
 *                                         release job to sign (this script
 *                                         holds no key and signs nothing)
 *
 * Why the tree carries its own SHA256SUMS *and* the archive gets a separate
 * one-line file: the tree-level list is what `sha256sum -c` checks after an
 * extraction, while the archive needs a checksum that a signature can cover
 * before anything is unpacked. One signed list per platform keeps each
 * download verifiable on its own; the release job also publishes a combined
 * list over every asset.
 *
 *   node scripts/release/assemble.mjs --version 0.3.1 --os linux --arch x64
 *   node scripts/release/assemble.mjs --version 0.3.1 --os windows --arch x64 \
 *        --target-triple x86_64-pc-windows-msvc
 *   node scripts/release/assemble.mjs --self-test
 *
 * Idempotent: a re-run replaces the directory and reproduces the same file
 * set and the same SHA256SUMS. It refuses, naming the absolute path, when any
 * required input is absent or empty (a release that silently omits the wizard
 * or the installer is worse than a red job).
 *
 * Non-purpose: it builds nothing, signs nothing, and knows no network. The
 * workflow calls it after cargo; `offline-bundle.sh` calls it indirectly by
 * consuming its output.
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  closeSync,
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readdirSync,
  readFileSync,
  readSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";

const ROOT = resolve(import.meta.dirname, "..", "..");

/** The two names the release path uses, and nothing else. */
const OS_NAMES = ["linux", "macos", "windows"];
const ARCH_NAMES = ["arm64", "x64"];

/** Which Tauri bundle directories each platform's config can produce. */
const BUNDLE_DIRS = {
  linux: ["deb", "rpm", "appimage"],
  macos: ["macos", "dmg"],
  windows: ["nsis", "msi"],
};

/**
 * The one bundle directory a platform cannot ship without. `deb` on Linux,
 * the `.app` on macOS, and the NSIS setup on Windows are the installers the
 * docs teach; RPM/AppImage/DMG/MSI are extras when the runner can make them.
 */
const PRIMARY_BUNDLE = { linux: "deb", macos: "macos", windows: "nsis" };

/** Paths under `deploy/` a release never carries. */
const DEPLOY_EXCLUDED = new Set(["dev.env"]);

/** Raised when an input the release cannot guess at is absent. */
export class MissingInput extends Error {
  constructor(what, path) {
    super(`${what} is missing: ${path}`);
    this.name = "MissingInput";
    this.what = what;
    this.path = path;
  }
}

function requireFile(path, what) {
  if (!existsSync(path) || !statSync(path).isFile()) throw new MissingInput(what, path);
  return path;
}

function requireDir(path, what) {
  if (!existsSync(path) || !statSync(path).isDirectory()) throw new MissingInput(what, path);
  return path;
}

/** sha256 of a file, read in chunks so a 100 MB binary stays off the heap. */
export function sha256File(path) {
  const hash = createHash("sha256");
  const fd = openSync(path, "r");
  const buffer = Buffer.allocUnsafe(1 << 20);
  try {
    for (;;) {
      const read = readSync(fd, buffer, 0, buffer.length, null);
      if (read === 0) break;
      hash.update(buffer.subarray(0, read));
    }
  } finally {
    closeSync(fd);
  }
  return hash.digest("hex");
}

/** Every regular file under `dir`, relative paths in posix form, sorted. */
export function walkFiles(dir) {
  const found = [];
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const abs = join(current, entry.name);
      if (entry.isDirectory()) walk(abs);
      else if (entry.isFile()) found.push(abs);
      // Symlinks are neither: the sums are over bytes, and a link has no bytes
      // of its own. Nothing in the inputs is expected to be one.
      else if (entry.isSymbolicLink()) {
        throw new Error(`${abs} is a symlink; the release tree is plain files only`);
      }
    }
  };
  walk(dir);
  return found
    .map((abs) => relative(dir, abs).split(sep).join("/"))
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/** `sha256sum` format: hash, two spaces, path — sorted, one line per file. */
export function renderSums(pairs) {
  return `${pairs.map(([hash, path]) => `${hash}  ${path}`).join("\n")}\n`;
}

/** Parse a sums file back into pairs, refusing a line it cannot read. */
export function parseSums(text) {
  const lines = text.split("\n").filter((line) => line.trim() !== "");
  return lines.map((line) => {
    const match = /^([0-9a-f]{64}) [ *](.+)$/.exec(line);
    if (!match) throw new Error(`not a sha256sum line: ${JSON.stringify(line)}`);
    return [match[1], match[2]];
  });
}

function defaultTargetDir(root) {
  const configured = process.env.CARGO_TARGET_DIR;
  if (configured && configured.trim() !== "") {
    return resolve(root, configured);
  }
  return join(root, "target");
}

/**
 * Assemble one platform's release directory.
 *
 * @param {object} options
 * @param {string} options.version    semver without the `v`
 * @param {string} options.os         linux | macos | windows
 * @param {string} options.arch       arm64 | x64
 * @param {string} [options.root]     repository root
 * @param {string} [options.out]      parent directory, default `<root>/dist/release`
 * @param {string} [options.targetDir] cargo target directory
 * @param {string} [options.triple]   cross-compilation triple, when used
 * @param {string} [options.binary]   explicit path to the CLI binary
 * @param {string} [options.commit]   commit the artifacts were built from
 * @param {boolean} [options.archive] write the `.tar.gz`, default true
 */
export function assemble(options) {
  const { version, os, arch } = options;
  if (!OS_NAMES.includes(os)) {
    throw new Error(`--os must be one of ${OS_NAMES.join(", ")} (got ${JSON.stringify(os)})`);
  }
  if (!ARCH_NAMES.includes(arch)) {
    throw new Error(`--arch must be one of ${ARCH_NAMES.join(", ")} (got ${JSON.stringify(arch)})`);
  }
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version ?? "")) {
    throw new Error(`--version must be a semver release (got ${JSON.stringify(version)})`);
  }

  const root = resolve(options.root ?? ROOT);
  const out = resolve(options.out ?? join(root, "dist", "release"));
  const targetDir = resolve(options.targetDir ?? defaultTargetDir(root));
  const profileDir = options.triple
    ? join(targetDir, options.triple, "release")
    : join(targetDir, "release");

  const name = `marula-${version}-${os}-${arch}`;
  const binaryName = os === "windows" ? "marula.exe" : "marula";

  // ── inputs, each one named if it is not there ───────────────────────────
  const binary = requireFile(
    resolve(options.binary ?? join(profileDir, binaryName)),
    `the ${binaryName} binary (build it with ` +
      "`cargo build --release -p marula-core-cli`)",
  );
  const uiDist = requireDir(
    join(root, "apps", "installer", "dist"),
    "the wizard front-end (build it with `pnpm --dir apps/installer build`)",
  );
  requireFile(join(uiDist, "index.html"), "the wizard front-end entry point");
  const compose = requireFile(
    join(root, "deploy", "docker-compose.yml"),
    "the base compose file",
  );
  const composeAllInOne = requireFile(
    join(root, "deploy", "docker-compose.all-in-one.yml"),
    "the all-in-one compose override",
  );
  const license = requireFile(join(root, "LICENSE"), "the licence text");
  const readme = requireFile(join(root, "README.md"), "the README");

  const bundleRoot = join(profileDir, "bundle");
  const presentBundles = BUNDLE_DIRS[os].filter((kind) => {
    const dir = join(bundleRoot, kind);
    return existsSync(dir) && readdirSync(dir).length > 0;
  });
  const primary = PRIMARY_BUNDLE[os];
  requireDir(
    join(bundleRoot, primary),
    `the ${primary} installer bundle (build it with ` +
      "`pnpm exec tauri bundle`; the platform-specific tauri.<os>.conf.json " +
      "decides which bundles exist)",
  );
  if (!presentBundles.includes(primary)) {
    throw new MissingInput(`the ${primary} installer bundle`, join(bundleRoot, primary));
  }

  // ── staging tree, so a failed run leaves the previous release intact ─────
  mkdirSync(out, { recursive: true });
  const staging = join(out, `.assemble-${process.pid}-${name}`);
  rmSync(staging, { recursive: true, force: true });
  mkdirSync(staging, { recursive: true });

  try {
    cpSync(binary, join(staging, binaryName));
    for (const kind of presentBundles) {
      cpSync(join(bundleRoot, kind), join(staging, "installers", kind), { recursive: true });
    }
    cpSync(uiDist, join(staging, "apps", "installer", "dist"), { recursive: true });
    cpSync(join(root, "deploy"), join(staging, "deploy"), {
      recursive: true,
      filter: (source) => {
        const rel = relative(join(root, "deploy"), source).split(sep).join("/");
        return !DEPLOY_EXCLUDED.has(rel);
      },
    });
    cpSync(license, join(staging, "LICENSE"));
    cpSync(readme, join(staging, "README.md"));
    for (const extra of options.extra ?? []) {
      const file = requireFile(resolve(extra), `the extra file ${extra}`);
      cpSync(file, join(staging, basename(file)));
    }

    const manifest = {
      schema: "marula.release/1",
      version,
      os,
      arch,
      binary: binaryName,
      bundles: presentBundles,
      compose: [
        "deploy/docker-compose.yml",
        "deploy/docker-compose.all-in-one.yml",
      ],
      ui: "apps/installer/dist",
      ...(options.commit ? { commit: options.commit } : {}),
    };
    writeFileSync(join(staging, "release.json"), `${JSON.stringify(manifest, null, 2)}\n`);

    // ── the tree's own sums: everything above, sorted ─────────────────────
    const pairs = walkFiles(staging)
      .filter((rel) => rel !== "SHA256SUMS")
      .map((rel) => [sha256File(join(staging, rel)), rel]);
    writeFileSync(join(staging, "SHA256SUMS"), renderSums(pairs));

    const final = join(out, name);
    rmSync(final, { recursive: true, force: true });
    renameSync(staging, final);

    // ── the archive, and the one-line list a release job signs ────────────
    let archive = null;
    let archiveSums = null;
    if (options.archive !== false) {
      archive = join(out, `${name}.tar.gz`);
      rmSync(archive, { force: true });
      // `tar` is on macOS, Linux and Windows 10+ (bsdtar). The path is passed
      // relative to `-C` so the archive unpacks into a directory named after
      // the release rather than into a developer's home directory.
      execFileSync("tar", ["-czf", archive, "-C", out, name], { stdio: "inherit" });
      archiveSums = join(out, `SHA256SUMS-${os}-${arch}`);
      writeFileSync(archiveSums, renderSums([[sha256File(archive), basename(archive)]]));
    }

    return {
      name,
      dir: final,
      archive,
      archiveSums,
      files: pairs.length,
      bundles: presentBundles,
      composeFiles: [compose, composeAllInOne],
    };
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}

/**
 * Write the release manifest and the combined checksum list.
 *
 * Called once, by the publishing job, after every platform leg has assembled
 * its directory and archived it into one output directory. The manifest is
 * discovery data (which version this directory is, which files belong to which
 * platform); the security decisions live in the signed `SHA256SUMS-<os>-<arch>`
 * files, each of which covers exactly the archive a downloader fetches.
 *
 * @param {object} options
 * @param {string} options.version  semver without the `v`
 * @param {string} [options.commit] commit the artifacts were built from
 * @param {string} [options.out]    directory holding the assets
 */

export function writeManifest(options) {
  const { version } = options;
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version ?? "")) {
    throw new Error(`--version must be a semver release (got ${JSON.stringify(version)})`);
  }
  const out = resolve(options.out ?? join(ROOT, "dist", "release"));
  requireDir(out, "the directory holding the release assets");

  /** @type {Record<string, {archive: string, sums: string, signature: string, offline?: string}>} */
  const assets = {};
  for (const os of OS_NAMES) {
    for (const arch of ARCH_NAMES) {
      const key = `${os}-${arch}`;
      const archive = join(out, `marula-${version}-${os}-${arch}.tar.gz`);
      if (!existsSync(archive)) continue;
      const entry = {
        archive: basename(archive),
        sums: `SHA256SUMS-${os}-${arch}`,
        signature: `SHA256SUMS-${os}-${arch}.sig`,
      };
      const offline = join(out, `marula-${version}-offline-${os}-${arch}.tar.zst`);
      if (existsSync(offline)) entry.offline = basename(offline);
      assets[key] = entry;
    }
  }
  const platforms = Object.keys(assets);
  if (platforms.length === 0) {
    throw new MissingInput(
      "at least one platform archive",
      join(out, `marula-${version}-<os>-<arch>.tar.gz`),
    );
  }

  const manifest = {
    schema: "marula.release/1",
    version,
    ...(options.commit ? { commit: options.commit } : {}),
    assets,
  };
  writeFileSync(join(out, "release.json"), `${JSON.stringify(manifest, null, 2)}\n`);

  // The combined list covers every asset a person downloads by hand, the
  // per-platform sums a bootstrap fetches, and the manifest itself. Detached
  // signatures are not listed: each one is checked against the file it signs.
  const listed = walkFiles(out).filter(
    (rel) =>
      rel !== "SHA256SUMS" &&
      !rel.endsWith(".sig") &&
      (rel === "release.json" ||
        rel.startsWith(`marula-${version}-`) ||
        rel.startsWith("SHA256SUMS-")),
  );
  const pairs = listed.map((rel) => [sha256File(join(out, rel)), rel]);
  writeFileSync(join(out, "SHA256SUMS"), renderSums(pairs));

  return { out, platforms, files: pairs.length, manifest };
}

/** The fewest assertions that prove the layout rules, without cargo. */
function selfTest() {  const assert = (condition, message) => {
    if (!condition) throw new Error(`self-test: ${message}`);
  };
  const log = (line) => process.stdout.write(`  ${line}\n`);
  const fail = (fn, expected) => {
    try {
      fn();
    } catch (error) {
      if (error instanceof MissingInput && error.message.includes(expected)) return error.path;
      throw error;
    }
    throw new Error(`self-test: expected a refusal naming ${expected}`);
  };

  const temp = join(
    process.env.TMPDIR ?? "/tmp",
    `assemble-selftest-${process.pid}-${Date.now()}`,
  );
  const fake = join(temp, "repo");
  const out = join(temp, "out");
  mkdirSync(join(fake, "target", "release", "bundle", "deb"), { recursive: true });
  mkdirSync(join(fake, "apps", "installer", "dist"), { recursive: true });
  mkdirSync(join(fake, "deploy", "pooler"), { recursive: true });
  writeFileSync(join(fake, "target", "release", "marula"), "#!/bin/sh\necho fake\n", {
    mode: 0o755,
  });
  writeFileSync(join(fake, "target", "release", "bundle", "deb", "marula.deb"), "deb\n");
  writeFileSync(join(fake, "apps", "installer", "dist", "index.html"), "<!doctype html>\n");
  writeFileSync(join(fake, "apps", "installer", "dist", "app.js"), "console.log(1)\n");
  writeFileSync(join(fake, "deploy", "docker-compose.yml"), "services: {}\n");
  writeFileSync(join(fake, "deploy", "docker-compose.all-in-one.yml"), "services: {}\n");
  writeFileSync(join(fake, "deploy", "dev.env"), "MARULA_DEV_TOOLS=1\n");
  writeFileSync(join(fake, "deploy", "pooler", "pgbouncer.ini"), "[databases]\n");
  writeFileSync(join(fake, "LICENSE"), "AGPL-3.0\n");
  writeFileSync(join(fake, "README.md"), "# fake\n");

  const base = { version: "9.9.9", os: "linux", arch: "x64", root: fake, out };

  try {
    // A missing input is refused by name, and the name is a real path.
    const binaryPath = join(fake, "target", "release", "marula");
    renameSync(binaryPath, `${binaryPath}.away`);
    const named = fail(() => assemble(base), binaryPath);
    log(`missing binary refused, naming ${named}`);
    renameSync(`${binaryPath}.away`, binaryPath);

    const distPath = join(fake, "apps", "installer", "dist");
    renameSync(distPath, `${distPath}.away`);
    fail(() => assemble(base), distPath);
    log("missing front-end refused, naming its directory");
    renameSync(`${distPath}.away`, distPath);

    const bundlePath = join(fake, "target", "release", "bundle", "deb");
    renameSync(bundlePath, `${bundlePath}.away`);
    fail(() => assemble(base), bundlePath);
    log("missing installer bundle refused, naming its directory");
    renameSync(`${bundlePath}.away`, bundlePath);

    // The happy path.
    const first = assemble({ ...base, commit: "0".repeat(40) });
    assert(first.name === "marula-9.9.9-linux-x64", "release directory name");
    for (const rel of [
      "marula",
      "installers/deb/marula.deb",
      "apps/installer/dist/index.html",
      "deploy/docker-compose.yml",
      "deploy/docker-compose.all-in-one.yml",
      "deploy/pooler/pgbouncer.ini",
      "LICENSE",
      "README.md",
      "release.json",
      "SHA256SUMS",
    ]) {
      assert(existsSync(join(first.dir, rel)), `expected ${rel} in the release directory`);
    }
    assert(!existsSync(join(first.dir, "deploy", "dev.env")), "dev.env is not shipped");
    assert(
      (statSync(join(first.dir, "marula")).mode & 0o111) !== 0,
      "the binary keeps its execute bit",
    );
    log("layout is complete, and dev.env stays behind");

    // The sums are sha256sum-shaped, sorted, and match the bytes.
    const parsed = parseSums(readFileSync(join(first.dir, "SHA256SUMS"), "utf8"));
    assert(parsed.length === first.files, "one line per file");
    const paths = parsed.map(([, path]) => path);
    assert(
      paths.join("\n") === [...paths].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)).join("\n"),
      "the sums are sorted",
    );
    for (const [hash, path] of parsed) {
      assert(sha256File(join(first.dir, path)) === hash, `hash of ${path}`);
    }
    assert(!paths.includes("SHA256SUMS"), "the sums never list themselves");
    log(`${parsed.length} files summed, sorted, and re-hashing clean`);

    // Idempotent: a second run reproduces the same list.
    const second = assemble({ ...base, commit: "0".repeat(40) });
    assert(
      readFileSync(join(first.dir, "SHA256SUMS"), "utf8") ===
        readFileSync(join(second.dir, "SHA256SUMS"), "utf8"),
      "a re-run reproduces the same SHA256SUMS",
    );
    log("a re-run reproduces the same SHA256SUMS");

    // The archive and its one-line list agree.
    assert(existsSync(first.archive), "the archive exists");
    const [line] = parseSums(readFileSync(first.archiveSums, "utf8"));
    assert(line[0] === sha256File(first.archive), "the archive line matches the archive");
    assert(line[1] === basename(first.archive), "the archive line names the archive");
    log("the archive line matches the archive");

    // `sha256sum -c` (or macOS's `shasum`) accepts the tree list. This is the
    // claim the docs make, so the self-test makes it against the tool at hand.
    const checker = ["sha256sum", "shasum"].find((tool) => {
      try {
        execFileSync("sh", ["-c", `command -v ${tool}`], { stdio: "ignore" });
        return true;
      } catch {
        return false;
      }
    });
    if (checker) {
      const args = checker === "sha256sum" ? ["-c", "SHA256SUMS"] : ["-a", "256", "-c", "SHA256SUMS"];
      execFileSync(checker, args, { cwd: first.dir, stdio: ["ignore", "ignore", "pipe"] });
      log(`${checker} -c accepts the tree's SHA256SUMS`);
    } else {
      log("neither sha256sum nor shasum is here; skipped the -c check");
    }

    // The publishing step: every platform leg's archive in one directory, one
    // manifest, one combined list.
    writeFileSync(join(out, "marula-9.9.9-macos-arm64.tar.gz"), "fake-macos-archive\n");
    writeFileSync(join(out, "marula-9.9.9-offline-linux-x64.tar.zst"), "fake-offline\n");
    const published = writeManifest({ version: "9.9.9", out, commit: "0".repeat(40) });
    assert(published.platforms.length === 2, "two platform archives are discovered");
    const manifest = JSON.parse(readFileSync(join(out, "release.json"), "utf8"));
    assert(
      manifest.assets["linux-x64"].archive === "marula-9.9.9-linux-x64.tar.gz",
      "the manifest names linux-x64's archive",
    );
    assert(
      manifest.assets["linux-x64"].offline === "marula-9.9.9-offline-linux-x64.tar.zst",
      "the manifest names the offline bundle when there is one",
    );
    assert(manifest.assets["macos-arm64"], "the manifest names macos-arm64");
    fail(() => writeManifest({ version: "8.8.8", out }), "marula-8.8.8-");
    log("the manifest covers every platform it finds and refuses when there are none");

    const combined = parseSums(readFileSync(join(out, "SHA256SUMS"), "utf8"));
    assert(
      combined.some(([, path]) => path === "release.json"),
      "the combined list covers the manifest",
    );
    assert(
      !combined.some(([, path]) => path.endsWith(".sig")),
      "the combined list never claims a signature's hash",
    );
    for (const [hash, path] of combined) {
      assert(sha256File(join(out, path)) === hash, `combined hash of ${path}`);
    }
    if (checker) {
      const args = checker === "sha256sum" ? ["-c", "SHA256SUMS"] : ["-a", "256", "-c", "SHA256SUMS"];
      execFileSync(checker, args, { cwd: out, stdio: ["ignore", "ignore", "pipe"] });
      log(`${checker} -c accepts the combined SHA256SUMS`);
    }
    process.stdout.write("self-test: ok\n");
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
}

function parseArgs(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = () => {
      const next = argv[index + 1];
      if (next === undefined || next.startsWith("--")) {
        throw new Error(`${flag} needs a value`);
      }
      index += 1;
      return next;
    };
    switch (flag) {
      case "--version":
        options.version = value();
        break;
      case "--os":
        options.os = value();
        break;
      case "--arch":
        options.arch = value();
        break;
      case "--root":
        options.root = value();
        break;
      case "--out":
        options.out = value();
        break;
      case "--target-dir":
        options.targetDir = value();
        break;
      case "--target-triple":
        options.triple = value();
        break;
      case "--binary":
        options.binary = value();
        break;
      case "--commit":
        options.commit = value();
        break;
      case "--extra":
        // A file the binary needs beside it that the workspace does not build:
        // on Windows the OpenSSL DLLs the CLI links against. Repeatable, and it
        // lands in the checksum list like every other file.
        options.extra = [...(options.extra ?? []), value()];
        break;
      case "--no-archive":
        options.archive = false;
        break;
      case "--manifest":
        options.manifest = true;
        break;
      case "--self-test":
        options.selfTest = true;
        break;
      case "-h":
      case "--help":
        options.help = true;
        break;
      default:
        throw new Error(`unknown flag ${flag}`);
    }
  }
  return options;
}

function usage() {
  return [
    "usage: assemble.mjs --version <semver> --os <linux|macos|windows> --arch <arm64|x64>",
    "                    [--out <dir>] [--root <repo>] [--target-dir <dir>]",
    "                    [--target-triple <triple>] [--binary <path>] [--commit <sha>]",
    "                    [--no-archive]",
    "       assemble.mjs --manifest --version <semver> [--out <dir>] [--commit <sha>]",
    "       assemble.mjs --self-test",
  ].join("\n");
}

if (import.meta.filename === process.argv[1]) {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n${usage()}\n`);
    process.exit(2);
  }
  if (options.help) {
    process.stdout.write(`${usage()}\n`);
    process.exit(0);
  }
  if (options.selfTest) {
    try {
      selfTest();
    } catch (error) {
      process.stderr.write(`${error.message}\n`);
      process.exit(1);
    }
    process.exit(0);
  }
  if (options.manifest) {
    if (!options.version) {
      process.stderr.write(`--manifest needs --version\n${usage()}\n`);
      process.exit(2);
    }
    try {
      const published = writeManifest(options);
      process.stdout.write(
        `release.json names ${published.platforms.join(", ")}\n` +
          `combined SHA256SUMS covers ${published.files} files in ${published.out}\n`,
      );
    } catch (error) {
      process.stderr.write(`assemble: ${error.message}\n`);
      process.exit(1);
    }
    process.exit(0);
  }
  if (!options.version || !options.os || !options.arch) {
    process.stderr.write(`--version, --os and --arch are required\n${usage()}\n`);
    process.exit(2);
  }
  try {
    const result = assemble(options);
    process.stdout.write(
      [
        `assembled ${result.files} files into ${result.dir}`,
        `bundles: ${result.bundles.join(", ")}`,
        result.archive ? `archive: ${result.archive}` : "archive: skipped",
        result.archiveSums ? `sums for the release job to sign: ${result.archiveSums}` : "",
        `verify with: (cd ${result.dir} && sha256sum -c SHA256SUMS)`,
      ]
        .filter(Boolean)
        .join("\n") + "\n",
    );
  } catch (error) {
    process.stderr.write(`assemble: ${error.message}\n`);
    process.exit(1);
  }
}
