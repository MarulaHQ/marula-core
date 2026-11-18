#!/usr/bin/env node
/**
 * Build and publish the release images (`docs/release-images.md`).
 *
 * Four images, one prefix, one version tag each, built natively per architecture
 * and published as one manifest per image. The digest is the identity: a tag can
 * be moved, and `--images registry` refuses to plan without a digest for every
 * service it starts. So this reads each digest back after pushing and writes it
 * into `release.json`, which the release lane then signs.
 *
 * The three Rust services share `deploy/Dockerfile.rust` and differ by `BIN`;
 * the web image has its own Dockerfile and a Next.js export. Those pairs mirror
 * `deploy/docker-compose.yml` and a test keeps them from drifting.
 *
 * In CI each architecture builds on its own native runner (`--matrix` plans the
 * legs), pushes an arch-suffixed tag, and `--publish` joins those tags into the
 * multi-arch manifest. Nothing is emulated: compiling this workspace under QEMU
 * ran past the image job's timeout.
 *
 *   node scripts/release/images.mjs --version 0.4.0
 *   node scripts/release/images.mjs --version 0.4.0 --manifest dist/release/release.json
 *   node scripts/release/images.mjs --version 0.4.0 --dry-run
 *   node scripts/release/images.mjs --matrix --platforms linux/amd64,linux/arm64
 *   node scripts/release/images.mjs --version 0.4.0 --service core-api \
 *     --platform linux/arm64 --tag-suffix arm64
 *   node scripts/release/images.mjs --version 0.4.0 --service core-api --publish \
 *     --platforms linux/amd64,linux/arm64 --digests images-core-api.json
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
    imageBase: "ghcr.io/marulahq",
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
      case "--platform": out.platforms = value(); break;
      case "--platforms": out.platforms = value(); break;
      case "--tag-suffix": out.tagSuffix = value(); break;
      case "--publish": out.publish = true; break;
      case "--matrix": out.matrix = true; break;
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
            "       images.mjs --version X --service core-api --platform linux/arm64 --tag-suffix arm64\n" +
            "       images.mjs --version X --service core-api --publish --platforms a,b [--digests images.json]\n" +
            "       images.mjs --matrix [--platforms a,b]\n" +
            "       images.mjs --merge <dir> --digests images.json",
        );
        process.exit(0);
        break;
      default: throw new Error(`unknown flag ${flag}`);
    }
  }
  if (out.merge || out.matrix) return out;
  if (!out.version) throw new Error("--version is required");
  if (!/^\d+\.\d+\.\d+$/.test(out.version)) throw new Error(`--version must be a semver release (got ${out.version})`);
  if (out.tagSuffix && out.platforms.includes(",")) {
    throw new Error(
      `--tag-suffix names one architecture (got ${out.tagSuffix}) but --platforms is ${out.platforms}: ` +
        "build each architecture on its own runner, then --publish joins them",
    );
  }
  if (out.tagSuffix && (out.digests || out.manifest)) {
    throw new Error("--tag-suffix pushes a build leg, not a release image: the digest to pin comes from --publish");
  }
  return out;
}

const isBuild = (args) => args[0] === "buildx" && args[1] === "build";

const run = (args, dryRun) => {
  const line = ["docker", ...args].join(" ");
  if (dryRun) {
    console.log(`would run: ${line}`);
    return "";
  }
  // Inherited stdio: a build that takes forty minutes narrates those minutes
  // into the job log. Piping it into this process is how the first failing runs
  // sat silent until the timeout with no way to tell compiling from hung.
  if (isBuild(args)) return execFileSync("docker", args, { stdio: "inherit" });
  return execFileSync("docker", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
};

/** The hosted runner that builds each architecture natively, keyed by arch. */
const RUNNERS = { amd64: "ubuntu-24.04", arm64: "ubuntu-24.04-arm" };

/** `linux/arm64` → `arm64`, refusing anything that is not a Linux platform. */
function platformArch(platform) {
  const [os, arch] = platform.split("/");
  if (os !== "linux" || !arch) throw new Error(`platform ${platform} is not linux/<arch>`);
  return arch;
}

function platformList(platforms) {
  const entries = platforms.split(",").map((entry) => entry.trim()).filter(Boolean);
  if (entries.length === 0) throw new Error("no platforms named");
  return entries;
}

/**
 * `linux/amd64` → `amd64`. The suffix is also what names the arch tag and
 * selects the runner, so one parse decides all three.
 */
export function archSuffixes(platforms) {
  return platformList(platforms).map(platformArch);
}

/**
 * The CI matrix: one leg per service and architecture.
 *
 * The runner is the whole point. `ubuntu-24.04-arm` builds arm64 natively, and
 * the image job never asks QEMU to pretend — the emulated half is what ran past
 * the 90-minute limit while the web image finished in minutes.
 * An architecture with no native runner is refused here rather than emulated.
 */
export function matrixFor(platforms, services = SERVICES) {
  const include = [];
  const legs = platformList(platforms);
  for (const spec of services) {
    for (const platform of legs) {
      const arch = platformArch(platform);
      const runner = RUNNERS[arch];
      if (!runner) {
        throw new Error(
          `no hosted runner builds ${platform} natively: known are ${Object.keys(RUNNERS).join(", ")}`,
        );
      }
      include.push({ service: spec.service, platform, arch, runner });
    }
  }
  return { include };
}

/** The release image (no suffix) or one architecture's build leg (suffixed). */
export function buildReference(spec, args, suffix) {
  const tag = suffix ? `${args.version}-${suffix}` : args.version;
  return `${args.imageBase}/${spec.image}:${tag}`;
}

/** Join the per-architecture legs into one manifest list under the release tag. */
export function publishArgs(ref, legs) {
  return ["buildx", "imagetools", "create", "--tag", ref, ...legs];
}

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
 * and architecture per runner: eight native legs rather than one job waiting on
 * an emulated `arm64` compile. An unknown name is refused rather than ignored,
 * because a typo that silently builds nothing would publish a tag that does not
 * exist.
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
  // The matrix is pure: it names the runners the rest of the workflow uses, so
  // the workflow cannot drift into emulation by editing its own YAML.
  if (args.matrix) {
    console.log(JSON.stringify(matrixFor(args.platforms)));
    return;
  }
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

  if (args.publish) {
    publish(selected, args);
    return;
  }

  const digests = {};
  for (const spec of selected) {
    // A build leg pushes `<version>-<arch>` and stops: its digest is not the
    // release's identity, because the installer pins the joined manifest.
    const ref = buildReference(spec, args, args.tagSuffix);
    console.log(`building ${ref} for ${args.platforms}`);
    run(buildArgs(spec, args, ref), args.dryRun);
    if (args.tagSuffix) {
      console.log(`  ${spec.service} → ${ref} (build leg; --publish records the digest)`);
      continue;
    }
    if (args.dryRun) {
      console.log(`would read the digest of ${ref}`);
      continue;
    }
    digests[spec.service] = readDigest(ref, spec, args);
    console.log(`  ${spec.service} → ${digests[spec.service]}`);
  }

  writeDigests(args, digests, selected.length);
}

/** Join each service's arch tags into one manifest and record its digest. */
function publish(selected, args) {
  const suffixes = archSuffixes(args.platforms);
  if (new Set(suffixes).size !== suffixes.length) {
    throw new Error(`--platforms repeats an architecture: ${args.platforms}`);
  }
  const digests = {};
  for (const spec of selected) {
    const ref = buildReference(spec, args, undefined);
    const legs = suffixes.map((suffix) => buildReference(spec, args, suffix));
    console.log(`publishing ${ref} from ${legs.length} architecture leg(s)`);
    run(publishArgs(ref, legs), args.dryRun);
    if (args.dryRun) continue;
    digests[spec.service] = readDigest(ref, spec, args);
    console.log(`  ${spec.service} → ${digests[spec.service]}`);
  }
  writeDigests(args, digests, selected.length);
}

function readDigest(ref, spec, args) {
  const inspected = run(
    ["buildx", "imagetools", "inspect", ref, "--format", "{{json .Manifest.Digest}}"],
    false,
  );
  return `${args.imageBase}/${spec.image}@${digestFromInspect(inspected)}`;
}

function writeDigests(args, digests, expected) {
  // The bare map exists so the digests can be published before the release
  // manifest is written: the manifest step signs what it writes, and a digest
  // added afterwards sits outside that signature.
  if (args.digests) {
    if (args.dryRun) {
      console.log(`would write ${Object.keys(digests).length} digests into ${args.digests}`);
    } else {
      if (Object.keys(digests).length !== expected) {
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
    if (Object.keys(digests).length !== expected) {
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
  const reference = (service) => `ghcr.io/marulahq/marula-${service}@sha256:${"a".repeat(64)}`;
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
  const bare = JSON.parse(digestsDocument({ "core-api": "ghcr.io/marulahq/marula-core-api@sha256:" + "a".repeat(64) }));
  assert(bare["core-api"].endsWith("@sha256:" + "a".repeat(64)), "the bare map is what the manifest step reads");

  const args = { imageBase: "ghcr.io/marulahq", version: "1.2.3" };
  const web = SERVICES.find((spec) => spec.service === "web");
  assert(
    buildReference(web, args, undefined) === "ghcr.io/marulahq/marula-web:1.2.3",
    "a release build pushes the version tag",
  );
  assert(
    buildReference(web, args, "arm64") === "ghcr.io/marulahq/marula-web:1.2.3-arm64",
    "an architecture leg pushes its own suffixed tag",
  );
  assert(archSuffixes("linux/amd64,linux/arm64").join(",") === "amd64,arm64", "the suffix is the platform's arch");
  assert(archSuffixes("linux/arm64").length === 1, "a one-architecture run publishes a one-leg manifest");
  assert(
    publishArgs("ref", ["ref-amd64", "ref-arm64"]).join(" ") ===
      "buildx imagetools create --tag ref ref-amd64 ref-arm64",
    "publish joins the arch tags under the release tag",
  );

  const planned = matrixFor("linux/amd64,linux/arm64");
  assert(planned.include.length === 8, "four services on two architectures plan eight legs");
  const armLeg = planned.include.find((leg) => leg.service === "core-api" && leg.arch === "arm64");
  assert(armLeg.runner === "ubuntu-24.04-arm", "the arm64 leg runs on a native arm64 runner");
  const amdLeg = planned.include.find((leg) => leg.service === "core-api" && leg.arch === "amd64");
  assert(amdLeg.runner === "ubuntu-24.04", "the amd64 leg runs on a native amd64 runner");
  let noRunner = false;
  try {
    matrixFor("linux/riscv64");
  } catch {
    noRunner = true;
  }
  assert(noRunner, "an architecture with no native runner is refused instead of emulated");

  // The failure this replaced: one job built both architectures, so the whole
  // job waited on emulated arm64 and hit its timeout. Emulation must not creep
  // back in through the workflow YAML.
  const workflow = readFileSync(".github/workflows/release-images.yml", "utf8");
  assert(!workflow.includes("setup-qemu-action"), "the image workflow does not set QEMU up for a foreign architecture");
  assert(workflow.includes("--matrix"), "the workflow takes its runners from the matrix this script plans");

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
