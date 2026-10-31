#!/usr/bin/env node
/**
 * Build and publish the release images (`docs/release-images.md`).
 *
 * Four images, one prefix, one version tag each, built for both architectures
 * in a single buildx run per image and pushed by digest. The digest is the
 * identity: a tag can be moved, and `--images registry` refuses to plan without
 * a digest for every service it starts. So this reads each digest back after
 * pushing and writes it into `release.json`, which the release lane then signs.
 *
 * The three Rust services share `deploy/Dockerfile.rust` and differ by `BIN`;
 * the web image has its own Dockerfile and a Next.js export. Those pairs mirror
 * `deploy/docker-compose.yml` and a test keeps them from drifting.
 *
 *   node scripts/release/images.mjs --version 0.4.0
 *   node scripts/release/images.mjs --version 0.4.0 --manifest dist/release/release.json
 *   node scripts/release/images.mjs --version 0.4.0 --dry-run
 *
 * Needs `docker` with buildx and a logged-in registry (`docker login ghcr.io`
 * with a token carrying write:packages). Nothing else: no CI, no dashboard.
 */
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const SERVICES = [
  { service: "core-api", image: "marula-core-api", dockerfile: "deploy/Dockerfile.rust", bin: "core-api" },
  { service: "worker", image: "marula-worker", dockerfile: "deploy/Dockerfile.rust", bin: "worker" },
  { service: "migrate", image: "marula-migrate", dockerfile: "deploy/Dockerfile.rust", bin: "marula-core-cli" },
  { service: "web", image: "marula-web", dockerfile: "deploy/Dockerfile.web", bin: null },
];

function parseArgs(argv) {
  const out = {
    imageBase: "ghcr.io/marulagh",
    platforms: "linux/amd64,linux/arm64",
    dryRun: false,
    context: ".",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = () => {
      const next = argv[i + 1];
      if (next === undefined || next.startsWith("--")) throw new Error(`${flag} needs a value`);
      i += 1;
      return next;
    };
    switch (flag) {
      case "--version": out.version = value(); break;
      case "--image-base": out.imageBase = value(); break;
      case "--platforms": out.platforms = value(); break;
      case "--manifest": out.manifest = value(); break;
      case "--context": out.context = value(); break;
      case "--dry-run": out.dryRun = true; break;
      case "-h":
      case "--help":
        console.log("usage: images.mjs --version X [--image-base ghcr.io/org] [--platforms a,b] [--manifest release.json] [--dry-run]");
        process.exit(0);
        break;
      default: throw new Error(`unknown flag ${flag}`);
    }
  }
  if (!out.version) throw new Error("--version is required");
  if (!/^\d+\.\d+\.\d+$/.test(out.version)) throw new Error(`--version must be a semver release (got ${out.version})`);
  return out;
}

const run = (args, dryRun) => {
  const line = ["docker", ...args].join(" ");
  if (dryRun) {
    console.log(`would run: ${line}`);
    return "";
  }
  return execFileSync("docker", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
};

/** `buildx imagetools inspect` answers with a JSON string holding the digest. */
export function digestFromInspect(stdout) {
  const trimmed = stdout.trim();
  let value;
  try {
    value = JSON.parse(trimmed);
  } catch {
    value = trimmed;
  }
  const digest = String(value).replace(/^"|"$/g, "");
  if (!/^sha256:[0-9a-f]{64}$/.test(digest)) {
    throw new Error(`imagetools did not report a digest (got ${JSON.stringify(stdout.slice(0, 80))})`);
  }
  return digest;
}

function buildArgs(spec, args, ref) {
  const out = [
    "buildx", "build",
    "--platform", args.platforms,
    "--file", spec.dockerfile,
    "--tag", ref,
    "--provenance=true",
    "--sbom=true",
    "--push",
  ];
  if (spec.bin) out.push("--build-arg", `BIN=${spec.bin}`);
  out.push(args.context);
  return out;
}

/** Merge the digests into the release manifest, keyed by service. */
export function withImages(manifest, digests) {
  return { ...manifest, images: digests };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  for (const spec of SERVICES) {
    if (!existsSync(resolve(args.context, spec.dockerfile))) {
      throw new Error(`missing ${spec.dockerfile}: this must run from the repository root`);
    }
  }

  const digests = {};
  for (const spec of SERVICES) {
    const ref = `${args.imageBase}/${spec.image}:${args.version}`;
    console.log(`building ${ref} for ${args.platforms}`);
    run(buildArgs(spec, args, ref), args.dryRun);
    if (args.dryRun) {
      console.log(`would read the digest of ${ref}`);
      continue;
    }
    const inspected = run(["buildx", "imagetools", "inspect", ref, "--format", "{{json .Manifest.Digest}}"], false);
    digests[spec.service] = `${args.imageBase}/${spec.image}@${digestFromInspect(inspected)}`;
    console.log(`  ${spec.service} → ${digests[spec.service]}`);
  }

  if (args.manifest) {
    if (args.dryRun) {
      console.log(`would write images into ${args.manifest}`);
      return;
    }
    if (Object.keys(digests).length !== SERVICES.length) {
      throw new Error("refusing to write a manifest with an incomplete image set");
    }
    const manifest = JSON.parse(readFileSync(args.manifest, "utf8"));
    writeFileSync(args.manifest, `${JSON.stringify(withImages(manifest, digests), null, 2)}\n`);
    console.log(`wrote ${Object.keys(digests).length} digests into ${args.manifest}`);
  }
}

const invokedDirectly = process.argv[1] && process.argv[1].endsWith("images.mjs");
if (invokedDirectly) {
  if (process.argv.includes("--self-test")) {
    selfTest();
  } else {
    try {
      main();
    } catch (error) {
      console.error(`image publish failed: ${error.message}`);
      process.exit(1);
    }
  }
}

/**
 * The parses and the pairs, proven without a network.
 *
 * The drift check matters most: the compose file decides how an install builds
 * these images, and a BIN that differs from the one an install passes produces
 * an image that runs the wrong binary. Nothing else would catch that until a
 * release was pulled and started.
 */
export function selfTest() {
  const digest = `sha256:${"a".repeat(64)}`;
  assert(digestFromInspect(JSON.stringify(digest)) === digest, "a quoted JSON digest is read");
  assert(digestFromInspect(digest) === digest, "a bare digest is read");
  let threw = false;
  try {
    digestFromInspect("not-a-digest");
  } catch {
    threw = true;
  }
  assert(threw, "a value that is not a digest is refused");

  const merged = withImages({ assets: { "linux-x64": {} } }, { "core-api": "x" });
  assert(merged.assets["linux-x64"], "merging keeps what the manifest already had");
  assert(merged.images["core-api"] === "x", "merging adds the digests");

  const compose = readFileSync("deploy/docker-compose.yml", "utf8");
  for (const spec of SERVICES) {
    assert(compose.includes(`dockerfile: ${spec.dockerfile}`), `${spec.service} builds from ${spec.dockerfile}`);
    if (spec.bin) {
      assert(
        compose.includes(`args: { BIN: ${spec.bin} }`),
        `${spec.service} is built with BIN=${spec.bin} in the compose file too`,
      );
    }
  }
  console.log("self-test: ok");
}

function assert(condition, message) {
  if (!condition) {
    console.error(`  ✗ ${message}`);
    process.exit(1);
  }
}
