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
import { readFileSync, writeFileSync, existsSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";

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
      case "--digests": out.digests = value(); break;
      case "--service": out.services = [...(out.services ?? []), value()]; break;
      case "--merge": out.merge = value(); break;
      case "--context": out.context = value(); break;
      case "--dry-run": out.dryRun = true; break;
      case "-h":
      case "--help":
        console.log(
          "usage: images.mjs --version X [--image-base ghcr.io/org] [--platforms a,b] " +
            "[--service core-api] [--manifest release.json] [--digests images.json] [--dry-run]\n" +
            "       images.mjs --merge <dir> --digests images.json",
        );
        process.exit(0);
        break;
      default: throw new Error(`unknown flag ${flag}`);
    }
  }
  if (out.merge) return out;
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

/**
 * The images this run builds.
 *
 * All four by default. `--service` narrows it so a CI matrix can build one image
 * per runner: four legs in parallel rather than four emulated builds in a row,
 * which is the difference between minutes and an hour for the `arm64` half. An
 * unknown name is refused rather than ignored, because a typo that silently
 * builds nothing would publish a tag that does not exist.
 */
export function selectServices(asked) {
  if (!asked || asked.length === 0) return SERVICES;
  const unknown = asked.filter((name) => !SERVICES.some((spec) => spec.service === name));
  if (unknown.length > 0) {
    throw new Error(
      `unknown --service ${unknown.join(", ")}: known are ${SERVICES.map((s) => s.service).join(", ")}`,
    );
  }
  return SERVICES.filter((spec) => asked.includes(spec.service));
}

/** Merge the digests into the release manifest, keyed by service. */
export function withImages(manifest, digests) {
  return { ...manifest, images: digests };
}

/**
 * One digest map from the several files a CI matrix produces.
 *
 * Every service must be there. A registry install refuses to plan without a
 * digest for each service it starts, so a partial merge would produce a manifest
 * that cannot install — better to fail in the job that has the list.
 */
export function mergeDigests(documents) {
  const merged = Object.assign({}, ...documents);
  const digest = /@sha256:[0-9a-f]{64}$/;
  for (const [service, reference] of Object.entries(merged)) {
    if (typeof reference !== "string" || !digest.test(reference)) {
      throw new Error(`the digest for ${service} is ${JSON.stringify(reference)}, which pins no digest`);
    }
  }
  const missing = SERVICES.map((spec) => spec.service).filter(
    (name) => typeof merged[name] !== "string",
  );
  if (missing.length > 0) {
    throw new Error(`no digest for ${missing.join(", ")}: every service needs one`);
  }
  return merged;
}

/** The bare `service → reference` map, which `assemble.mjs --images` reads. */
export function digestsDocument(digests) {
  return `${JSON.stringify(digests, null, 2)}\n`;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  // The matrix legs each write their own file; this joins them. Nothing is
  // built and no registry is contacted, so it runs on any runner.
  if (args.merge) {
    const files = readdirSync(args.merge).filter((name) => name.endsWith(".json"));
    if (files.length === 0) throw new Error(`no digest files in ${args.merge}`);
    const merged = mergeDigests(
      files.map((name) => JSON.parse(readFileSync(join(args.merge, name), "utf8"))),
    );
    if (!args.digests) throw new Error("--merge needs --digests to say where to write the result");
    writeFileSync(args.digests, digestsDocument(merged));
    console.log(`merged ${files.length} digest file(s) into ${args.digests}`);
    return;
  }
  const selected = selectServices(args.services);
  for (const spec of selected) {
    if (!existsSync(resolve(args.context, spec.dockerfile))) {
      throw new Error(`missing ${spec.dockerfile}: this must run from the repository root`);
    }
  }

  const digests = {};
  for (const spec of selected) {
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

  // The bare map exists so the digests can be published before the release
  // manifest is written: the manifest step signs what it writes, and a digest
  // added afterwards sits outside that signature.
  if (args.digests) {
    if (args.dryRun) {
      console.log(`would write ${Object.keys(digests).length} digests into ${args.digests}`);
    } else {
      if (Object.keys(digests).length !== selected.length) {
        throw new Error("refusing to write an incomplete image set");
      }
      writeFileSync(args.digests, digestsDocument(digests));
      console.log(`wrote ${Object.keys(digests).length} digests into ${args.digests}`);
    }
  }

  if (args.manifest) {
    if (args.dryRun) {
      console.log(`would write images into ${args.manifest}`);
      return;
    }
    if (Object.keys(digests).length !== selected.length) {
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
  const reference = (service) => `ghcr.io/marulagh/marula-${service}@sha256:${"a".repeat(64)}`;
  const full = mergeDigests([
    { "core-api": reference("core-api"), worker: reference("worker") },
    { migrate: reference("migrate"), web: reference("web") },
  ]);
  assert(Object.keys(full).length === 4, "a merge joins the matrix legs into one map");
  let partialRefused = false;
  try {
    mergeDigests([{ "core-api": reference("core-api") }]);
  } catch (error) {
    partialRefused = error.message.includes("worker");
  }
  assert(partialRefused, "a merge that is missing a service names it and refuses");
  assert(selectServices().length === 4, "a bare run builds all four images");
  const one = selectServices(["web"]);
  assert(one.length === 1 && one[0].service === "web", "--service narrows the set to the one asked for");
  let unknownRefused = false;
  try {
    selectServices(["core"]);
  } catch {
    unknownRefused = true;
  }
  assert(unknownRefused, "an unknown --service is refused rather than ignored");
  const bare = JSON.parse(digestsDocument({ "core-api": "ghcr.io/marulagh/marula-core-api@sha256:" + "a".repeat(64) }));
  assert(bare["core-api"].endsWith("@sha256:" + "a".repeat(64)), "the bare map is what the manifest step reads");

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
