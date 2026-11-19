#!/usr/bin/env node
/**
 * Build and publish the release images (`docs/release-images.md`).
 *
 * Four images, one prefix, one version tag each, built natively per architecture
 * and published as one manifest per image. The digest is the identity: a tag can
 * be moved, and `--images registry` refuses to plan without a digest for every
 * service it starts. Each build reports the digest it pushed through buildx's
 * metadata file, and the leg writes that reference into its artifact.
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
 *   node scripts/release/images.mjs --merge legs --version 0.4.0 \
 *     --digests images.json --digest-manifest image-digests.json
 *
 * Needs `docker` with buildx and a logged-in registry (`docker login ghcr.io`
 * with a token carrying write:packages). Nothing else: no CI, no dashboard.
 */
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, readdirSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

const SERVICES = [
  { service: "core-api", image: "marula-core-api", dockerfile: "deploy/Dockerfile.rust", bin: "core-api" },
  { service: "worker", image: "marula-worker", dockerfile: "deploy/Dockerfile.rust", bin: "worker" },
  { service: "migrate", image: "marula-migrate", dockerfile: "deploy/Dockerfile.rust", bin: "marula-core-cli" },
  { service: "web", image: "marula-web", dockerfile: "deploy/Dockerfile.web", bin: null },
];

/** One version format for the flag parser and the digest manifest writer. */
const isReleaseVersion = (version) => /^\d+\.\d+\.\d+$/.test(version ?? "");

export function parseArgs(argv) {
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
      case "--leg-digests": out.legDigests = value(); break;
      case "--publish": out.publish = true; break;
      case "--matrix": out.matrix = true; break;
      case "--manifest": out.manifest = value(); break;
      case "--digests": out.digests = value(); break;
      case "--digest-manifest": out.digestManifest = value(); break;
      case "--service": out.services = [...(out.services ?? []), value()]; break;
      case "--merge": out.merge = value(); break;
      case "--context": out.context = value(); break;
      case "--dry-run": out.dryRun = true; break;
      case "-h":
      case "--help":
        console.log(
          "usage: images.mjs --version X [--image-base ghcr.io/org] [--platforms a,b] " +
            "[--service core-api] [--manifest release.json] [--digests images.json] [--dry-run]\n" +
            "       images.mjs --version X --service core-api --platform linux/arm64 --tag-suffix arm64 --leg-digests legs\n" +
            "       images.mjs --version X --service core-api --publish --platforms a,b [--leg-digests legs] [--digests images.json]\n" +
            "       images.mjs --matrix [--platforms a,b]\n" +
            "       images.mjs --merge <dir> --version X --digests images.json [--digest-manifest image-digests.json]",
        );
        process.exit(0);
        break;
      default: throw new Error(`unknown flag ${flag}`);
    }
  }
  if (out.merge || out.matrix) return out;
  if (!out.version) throw new Error("--version is required");
  if (!isReleaseVersion(out.version)) throw new Error(`--version must be a semver release (got ${out.version})`);
  if (out.tagSuffix && out.platforms.includes(",")) {
    throw new Error(
      `--tag-suffix names one architecture (got ${out.tagSuffix}) but --platforms is ${out.platforms}: ` +
        "build each architecture on its own runner, then --publish joins them",
    );
  }
  if (out.tagSuffix) {
    // The suffix names the tag and the publish leg's digest file, so it has to
    // be the platform's own architecture: a mismatch here would push an arm64
    // image under `-amd64` and produce a manifest that lists one architecture
    // twice.
    out.tagSuffix = normalizeArch(out.tagSuffix);
    if (platformArch(out.platforms) !== out.tagSuffix) {
      throw new Error(`--tag-suffix ${out.tagSuffix} does not match --platform ${out.platforms}`);
    }
  }
  if (out.tagSuffix && (out.digests || out.manifest)) {
    throw new Error("--tag-suffix pushes a build leg, not a release image: the digest to pin comes from --publish");
  }
  if (out.legDigests && !out.tagSuffix && !out.publish) {
    throw new Error("--leg-digests belongs to a build leg (--tag-suffix) or to --publish");
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

/** Aliases containerd accepts, folded to the name the runner map and tags use. */
const ARCH_ALIASES = { aarch64: "arm64" };

export function normalizeArch(arch) {
  return ARCH_ALIASES[arch] ?? arch;
}

/** `linux/arm64` → `arm64`, refusing anything that is not a Linux platform. */
function platformArch(platform) {
  const [os, arch] = platform.split("/");
  if (os !== "linux" || !arch) throw new Error(`platform ${platform} is not linux/<arch>`);
  return normalizeArch(arch);
}

/**
 * The platforms named, each paired with its normalized architecture.
 *
 * Two entries for one architecture are refused here rather than after the
 * builds: both legs would push the same tag, and the publish step would then
 * reject a run that had already spent twenty minutes compiling.
 */
function platformList(platforms) {
  const entries = platforms.split(",").map((entry) => entry.trim()).filter(Boolean);
  if (entries.length === 0) throw new Error("no platforms named");
  const legs = entries.map((platform) => ({ platform, arch: platformArch(platform) }));
  const seen = new Set();
  for (const { platform, arch } of legs) {
    if (seen.has(arch)) {
      throw new Error(`--platforms names ${arch} twice (${platform}): one image per architecture`);
    }
    seen.add(arch);
  }
  return legs;
}

/**
 * `linux/amd64` → `amd64`. The suffix is also what names the arch tag and
 * selects the runner, so one parse decides all three.
 */
export function archSuffixes(platforms) {
  return platformList(platforms).map((leg) => leg.arch);
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
    for (const { platform, arch } of legs) {
      // Own properties only: `RUNNERS["constructor"]` would inherit a function
      // that JSON.stringify drops, producing a leg with no runner at all.
      if (!Object.hasOwn(RUNNERS, arch)) {
        throw new Error(
          `no hosted runner builds ${platform} natively: known are ${Object.keys(RUNNERS).join(", ")}`,
        );
      }
      include.push({ service: spec.service, platform, arch, runner: RUNNERS[arch] });
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
export function publishArgs(ref, legs, metadata) {
  return ["buildx", "imagetools", "create", "--tag", ref, "--metadata-file", metadata, ...legs];
}

/**
 * The digest a build leg pushed, in one file per service and architecture.
 *
 * Publish joins these references rather than resolving the `-arm64` tag again.
 * A failed publish can be re-run on its own, and by then a later dispatch may
 * have moved that tag to a different build; the digest file still names the
 * image this run compiled.
 */
export function legDigestPath(dir, service, suffix) {
  return join(dir, `${service}-${suffix}.digest`);
}

function writeLegDigest(dir, service, suffix, reference) {
  mkdirSync(dir, { recursive: true });
  writeFileSync(legDigestPath(dir, service, suffix), `${reference}\n`);
}

function readLegDigest(dir, service, suffix) {
  const path = legDigestPath(dir, service, suffix);
  if (!existsSync(path)) {
    throw new Error(`no ${service} ${suffix} leg digest at ${path}: its build leg did not record one`);
  }
  const reference = readFileSync(path, "utf8").trim();
  if (!/@sha256:[0-9a-f]{64}$/.test(reference)) {
    throw new Error(`the ${service} ${suffix} leg digest is ${JSON.stringify(reference)}, which pins no digest`);
  }
  return reference;
}

/**
 * The digest `buildx build --metadata-file` reports for what it pushed.
 *
 * This is the race-free source: the file is written by the build that pushed,
 * so a tag moved afterwards cannot change what the leg records.
 */
export function digestFromMetadata(document) {
  const digest = String(document?.["containerimage.digest"] ?? "");
  if (!/^sha256:[0-9a-f]{64}$/.test(digest)) {
    throw new Error(`the build metadata reports ${JSON.stringify(digest)} instead of a digest`);
  }
  return digest;
}

/**
 * The index digest `buildx imagetools create --metadata-file` reports.
 *
 * Same rule as the build legs: the digest comes from the tool that wrote the
 * manifest, not from reading the tag afterwards, because a tag can move between
 * those two calls. Buildx 0.32.0 introduced this metadata; the publish job pins
 * a version that has it.
 */
export function digestFromCreateMetadata(document) {
  const digest = String(document?.["containerimage.descriptor"]?.digest ?? "");
  if (!/^sha256:[0-9a-f]{64}$/.test(digest)) {
    throw new Error(`the create metadata reports ${JSON.stringify(digest)} instead of an index digest`);
  }
  return digest;
}

function buildArgs(spec, args, ref, metadata) {
  const out = [
    "buildx", "build",
    "--platform", args.platforms,
    "--file", spec.dockerfile,
    "--tag", ref,
    "--provenance=true",
    "--sbom=true",
    "--push",
    "--metadata-file", metadata,
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
 * Every service must be there, and nothing else may be. A registry install
 * refuses to plan without a digest for each service it starts, so a partial
 * merge would produce a manifest that cannot install — better to fail in the job
 * that has the list. An extra component is refused for the same reason: the
 * published set is four services, and a name outside it is either a typo or a
 * service that was never added to the release.
 */
export function mergeDigests(documents) {
  return completeDigests(Object.assign({}, ...documents));
}

/** Every digest pins one of the four published services. */
function completeDigests(digests) {
  const digest = /@sha256:[0-9a-f]{64}$/;
  const known = SERVICES.map((spec) => spec.service);
  const spare = Object.keys(digests).filter((service) => !known.includes(service));
  if (spare.length > 0) {
    throw new Error(
      `${spare.join(", ")} is not a published service: the set is ${known.join(", ")}`,
    );
  }
  for (const [service, reference] of Object.entries(digests)) {
    if (typeof reference !== "string" || !digest.test(reference)) {
      throw new Error(`the digest for ${service} is ${JSON.stringify(reference)}, which pins no digest`);
    }
  }
  const missing = known.filter((name) => typeof digests[name] !== "string");
  if (missing.length > 0) {
    throw new Error(`no digest for ${missing.join(", ")}: every service needs one`);
  }
  return digests;
}

/**
 * Keys in one order, so the same digests always serialize to the same bytes.
 *
 * Code-unit order, not locale order: `localeCompare` depends on the ICU data of
 * the machine, and an artifact whose bytes move with the runner's locale is not
 * a deterministic artifact.
 */
function sortedDigests(digests) {
  return Object.fromEntries(
    Object.entries(digests).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
  );
}

/** The bare `service → reference` map, which `assemble.mjs --images` reads. */
export function digestsDocument(digests) {
  return `${JSON.stringify(sortedDigests(digests), null, 2)}\n`;
}

/**
 * The published digest manifest: one file that names every released image.
 *
 * This is the artifact a consumer outside this repository's own pipeline reads,
 * so it carries the release version and a schema name beside the map, and it is
 * byte-identical for the same release data: sorted keys, two-space indent, one
 * trailing newline. The references are the ones the publish legs took from the
 * create call's metadata, never a tag resolved later.
 */
export function digestManifest(version, digests) {
  if (!isReleaseVersion(version)) {
    throw new Error(`the digest manifest needs a semver release (got ${version})`);
  }
  const document = {
    schema: "marula.images/1",
    version,
    images: sortedDigests(completeDigests(digests)),
  };
  return `${JSON.stringify(document, null, 2)}\n`;
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
    if (args.digestManifest) {
      // The release version is required here rather than defaulted: a manifest
      // that does not say which release it describes can be attached to the
      // wrong tag without anyone noticing.
      if (!args.version) throw new Error("--digest-manifest needs --version to name the release it describes");
      writeFileSync(args.digestManifest, digestManifest(args.version, merged));
      console.log(`wrote the digest manifest for ${args.version} into ${args.digestManifest}`);
    }
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
    const pushed = buildImage(spec, args, ref);
    if (args.tagSuffix) {
      if (!pushed) {
        if (args.legDigests) console.log(`would record the ${spec.service} ${args.tagSuffix} leg digest in ${args.legDigests}`);
        continue;
      }
      // Publish joins the digest this build reported, never the tag, so neither
      // a re-run nor another writer that moves `-<arch>` can change the image.
      if (args.legDigests) {
        writeLegDigest(args.legDigests, spec.service, args.tagSuffix, pushed);
        console.log(`  ${spec.service} → ${pushed} (leg digest recorded)`);
      } else {
        console.log(`  ${spec.service} → ${pushed} (build leg; --publish records the digest)`);
      }
      continue;
    }
    if (!pushed) {
      console.log(`would read the digest of ${ref}`);
      continue;
    }
    digests[spec.service] = pushed;
    console.log(`  ${spec.service} → ${digests[spec.service]}`);
  }

  writeDigests(args, digests, selected.length);
}

/**
 * Build, push, and report the digest buildx pushed.
 *
 * The digest comes from `--metadata-file`, not from inspecting the tag
 * afterwards: between a push and an inspect, the tag can be moved, and the
 * recorded artifact would then name a different build. Metadata is written by
 * the same build that did the push, so it cannot drift.
 */
function buildImage(spec, args, ref) {
  const dir = mkdtempSync(join(tmpdir(), "marula-image-"));
  const metadata = join(dir, "metadata.json");
  try {
    run(buildArgs(spec, args, ref, metadata), args.dryRun);
    if (args.dryRun) return null;
    const document = JSON.parse(readFileSync(metadata, "utf8"));
    return `${args.imageBase}/${spec.image}@${digestFromMetadata(document)}`;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/** Join each service's arch tags into one manifest and record its digest. */
function publish(selected, args) {
  const suffixes = archSuffixes(args.platforms);
  const digests = {};
  for (const spec of selected) {
    const ref = buildReference(spec, args, undefined);
    if (args.legDigests) console.log(`reading ${spec.service} leg digests from ${args.legDigests}`);
    // A dry run rehearses the command it would print, so it reads the leg files
    // too and names the one that is missing instead of previewing a join that
    // could not run.
    const legs = args.legDigests
      ? suffixes.map((suffix) => readLegDigest(args.legDigests, spec.service, suffix))
      : suffixes.map((suffix) => buildReference(spec, args, suffix));
    console.log(`publishing ${ref} from ${legs.length} architecture leg(s)`);
    const joined = publishImage(spec, args, ref, legs);
    if (!joined) continue;
    digests[spec.service] = joined;
    console.log(`  ${spec.service} → ${digests[spec.service]}`);
  }
  writeDigests(args, digests, selected.length);
}

/**
 * Create the manifest and report the index digest it wrote.
 *
 * The digest comes from the create call's own metadata for the same reason the
 * build legs take theirs from the build: the tag this creates can be moved
 * between the create and a follow-up inspect.
 */
function publishImage(spec, args, ref, legs) {
  const dir = mkdtempSync(join(tmpdir(), "marula-publish-"));
  const metadata = join(dir, "metadata.json");
  try {
    run(publishArgs(ref, legs, metadata), args.dryRun);
    if (args.dryRun) return null;
    const document = JSON.parse(readFileSync(metadata, "utf8"));
    return `${args.imageBase}/${spec.image}@${digestFromCreateMetadata(document)}`;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
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

  // Both digests come from the tool that wrote the image, never from a tag read
  // afterwards: the tag can be moved in between, the metadata cannot.
  assert(
    digestFromMetadata({ "containerimage.digest": digest }) === digest,
    "the build metadata is where the pushed digest comes from",
  );
  let noMetadataDigest = false;
  try {
    digestFromMetadata({ "containerimage.name": "ghcr.io/marulahq/marula-web" });
  } catch {
    noMetadataDigest = true;
  }
  assert(noMetadataDigest, "metadata without a digest is refused instead of guessed");

  assert(
    digestFromCreateMetadata({ "containerimage.descriptor": { digest } }) === digest,
    "the create metadata is where the joined index digest comes from",
  );
  let noIndexDigest = false;
  try {
    digestFromCreateMetadata({ "containerimage.descriptor": { mediaType: "application/vnd.oci.image.index.v1+json" } });
  } catch {
    noIndexDigest = true;
  }
  assert(noIndexDigest, "create metadata without an index digest is refused instead of guessed");

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
  let spareRefused = false;
  try {
    mergeDigests([{ ...full, cli: reference("cli") }]);
  } catch (error) {
    spareRefused = error.message.includes("cli");
  }
  assert(spareRefused, "a digest map carrying a component outside the four is refused");

  // The published set is a contract, not an implementation detail. The
  // comparison is against a literal so an edit to SERVICES that renames a
  // service, an image or a binary fails here instead of shipping an image no
  // chart or installer can name.
  const canonical = [
    ["core-api", "marula-core-api", "core-api"],
    ["worker", "marula-worker", "worker"],
    ["migrate", "marula-migrate", "marula-core-cli"],
    ["web", "marula-web", null],
  ];
  assert(
    JSON.stringify(SERVICES.map((spec) => [spec.service, spec.image, spec.bin])) === JSON.stringify(canonical),
    "the four published services, their images and their binaries are the canonical set",
  );

  // The digest manifest is a published artifact, so its bytes are the test:
  // the same digests in a different insertion order must not move a byte.
  const forwards = digestManifest("1.2.3", {
    "core-api": reference("core-api"),
    worker: reference("worker"),
    migrate: reference("migrate"),
    web: reference("web"),
  });
  const backwards = digestManifest("1.2.3", {
    web: reference("web"),
    migrate: reference("migrate"),
    worker: reference("worker"),
    "core-api": reference("core-api"),
  });
  assert(forwards === backwards, "the digest manifest is byte-identical whatever order the digests arrive in");
  assert(forwards.endsWith("}\n") && !forwards.endsWith("\n\n"), "the digest manifest ends in exactly one newline");
  const parsedManifest = JSON.parse(forwards);
  assert(parsedManifest.schema === "marula.images/1", "the digest manifest names its schema");
  assert(parsedManifest.version === "1.2.3", "the digest manifest names the release it describes");
  assert(
    Object.keys(parsedManifest.images).join(",") === "core-api,migrate,web,worker",
    "the digest manifest lists the four components in one sorted order",
  );
  assert(
    parsedManifest.images.migrate === reference("migrate"),
    "the digest manifest pins the image each service actually published",
  );
  let manifestWithoutVersion = false;
  try {
    digestManifest(undefined, full);
  } catch {
    manifestWithoutVersion = true;
  }
  assert(manifestWithoutVersion, "the digest manifest refuses to be written without a release version");
  let manifestPartial = false;
  try {
    digestManifest("1.2.3", { "core-api": reference("core-api") });
  } catch (error) {
    manifestPartial = error.message.includes("worker");
  }
  assert(manifestPartial, "the digest manifest refuses an incomplete image set");

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
    publishArgs("ref", ["ref-amd64", "ref-arm64"], "/tmp/meta.json").join(" ") ===
      "buildx imagetools create --tag ref --metadata-file /tmp/meta.json ref-amd64 ref-arm64",
    "publish joins the arch tags and records the create metadata",
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

  assert(archSuffixes("linux/aarch64").join(",") === "arm64", "the aarch64 alias folds to arm64");
  let duplicate = false;
  try {
    matrixFor("linux/amd64,linux/amd64");
  } catch {
    duplicate = true;
  }
  assert(duplicate, "a repeated architecture is refused before any leg is scheduled");
  let inherited = false;
  try {
    matrixFor("linux/constructor");
  } catch {
    inherited = true;
  }
  assert(inherited, "an inherited object property is not mistaken for a configured runner");

  // A leg records the digest it pushed, and publish joins that reference rather
  // than the tag a later dispatch could move.
  const legs = mkdtempSync(join(tmpdir(), "marula-legs-"));
  const legRef = `ghcr.io/marulahq/marula-web@sha256:${"b".repeat(64)}`;
  writeLegDigest(legs, "web", "arm64", legRef);
  assert(legDigestPath(legs, "web", "arm64").endsWith("web-arm64.digest"), "a leg digest is named by service and suffix");
  assert(readLegDigest(legs, "web", "arm64") === legRef, "a recorded leg digest is read back");
  let missingLeg = false;
  try {
    readLegDigest(legs, "web", "amd64");
  } catch (error) {
    missingLeg = error.message.includes("amd64");
  }
  assert(missingLeg, "a missing leg digest names the architecture it is missing");
  rmSync(legs, { recursive: true, force: true });

  let mismatched = false;
  try {
    parseArgs(["--version", "1.2.3", "--platform", "linux/arm64", "--tag-suffix", "amd64"]);
  } catch {
    mismatched = true;
  }
  assert(mismatched, "a suffix that is not the platform's architecture is refused");
  assert(
    parseArgs(["--version", "1.2.3", "--platform", "linux/aarch64", "--tag-suffix", "aarch64"]).tagSuffix === "arm64",
    "the suffix folds through the same alias",
  );
  let multiSuffix = false;
  try {
    parseArgs(["--version", "1.2.3", "--platforms", "linux/amd64,linux/arm64", "--tag-suffix", "arm64"]);
  } catch {
    multiSuffix = true;
  }
  assert(multiSuffix, "a suffix on a multi-architecture run is refused");

  // The failure this replaced: one job built both architectures, so the whole
  // job waited on emulated arm64 and hit its timeout. Emulation must not creep
  // back in through the workflow YAML.
  const workflow = readFileSync(".github/workflows/release-images.yml", "utf8");
  assert(!workflow.includes("setup-qemu-action"), "the image workflow does not set QEMU up for a foreign architecture");
  assert(workflow.includes("--matrix"), "the workflow takes its runners from the matrix this script plans");
  assert(workflow.includes("--leg-digests"), "the workflow joins the digests the build legs recorded");
  assert(workflow.includes("--digest-manifest"), "the workflow writes the published digest manifest");
  // Named without the `marula-` prefix: the release workflow downloads
  // `marula-*` into dist/release and attaches every file there, while the
  // signed SHA256SUMS does not cover this file.
  assert(workflow.includes("name: image-digests\n"), "the digest manifest travels as its own workflow artifact");

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
