#!/usr/bin/env node
/**
 * Publish a desktop update to the channel (ADR 0028 §5).
 *
 * One release is three things: the signed artifact in R2, its signature, and a
 * pointer the updater reads on every launch. The pointer is last, because a
 * pointer to a file that is not there is a broken update, not a slow one.
 *
 * Needs `wrangler` on the path and CLOUDFLARE_API_TOKEN in the environment.
 * Nothing here reads the signing key: Tauri already signed the artifact when it
 * bundled it, and this only carries the `.sig` into the pointer.
 *
 *   node scripts/release/updates.mjs --version 0.4.0 \
 *     --platform darwin-aarch64 \
 *     --artifact .kilo/target-shared/release/bundle/macos/Marula.app.tar.gz \
 *     --notes "Fixes a worker token that could be logged at debug level"
 *
 *   --stage 60      announce, but serve nothing for the first hour
 *   --rollback 0.3.1  re-point at a version already in the bucket
 *   --dry-run       print the commands and write nothing
 *
 * Versions are immutable: the object carries a year-long cache and the edge
 * will serve it. Re-uploading the same version means purging that URL first,
 * which is a deliberate act rather than a silent overwrite. The first publish
 * of 0.3.1 hit the other edge of this: the pointer went live before the object
 * existed, the edge cached the 404, and the URL answered 404 for minutes after
 * the upload -- which is why the pointer is written last and why a failed
 * upload leaves the channel saying 204 rather than offering a dead link.
 */
import { readFileSync, existsSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { basename, dirname, join } from "node:path";
import { tmpdir } from "node:os";

const BUCKET = "marula-updates";
const NAMESPACE = "e0e29e8523ff4385a983fe4ad3b39a57";
const ARTIFACT_HOST = "https://artifacts.getmarula.com";

function parseArgs(argv) {
  const out = { platform: "darwin-aarch64", channel: "stable", dryRun: false, stage: 0 };
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
      case "--platform": out.platform = value(); break;
      case "--artifact": out.artifact = value(); break;
      case "--notes": out.notes = value(); break;
      case "--channel": out.channel = value(); break;
      case "--stage": out.stage = Number(value()); break;
      case "--rollback": out.rollback = value(); break;
      case "--dry-run": out.dryRun = true; break;
      case "-h":
      case "--help":
        console.log("usage: updates.mjs --version X --artifact PATH [--platform p] [--stage minutes] [--rollback X] [--dry-run]");
        process.exit(0);
        break;
      default:
        throw new Error(`unknown flag ${flag}`);
    }
  }
  return out;
}

const run = (args, dry) => {
  const line = ["wrangler", ...args].join(" ");
  if (dry) {
    console.log(`would run: ${line}`);
    return "";
  }
  return execFileSync("wrangler", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
};

/** The pointer the Worker serves. Written last, and only when complete. */
function pointer({ version, platform, url, signature, notes, stage }) {
  return {
    version,
    pub_date: new Date().toISOString(),
    notes: notes ?? "",
    url,
    signature,
    ...(stage > 0 ? { not_before: new Date(Date.now() + stage * 60_000).toISOString() } : {}),
    platform,
  };
}

function keyFor({ channel, platform, version, file }) {
  return `desktop/${channel}/${platform}/${version}/${file}`;
}

function main() {
  const args = parseArgs(process.argv.slice(2));

  if (args.rollback) {
    const current = readPointer(args.platform);
    if (!current) throw new Error(`no pointer for ${args.platform}: nothing to roll back from`);
    const next = { ...current, version: args.rollback, pub_date: new Date().toISOString() };
    writePointer(args, next);
    console.log(`re-pointed ${args.platform} at ${args.rollback} (clients refuse downgrades)`);
    return;
  }

  if (!args.version) throw new Error("--version is required");
  if (!args.artifact) throw new Error("--artifact is required");
  if (!existsSync(args.artifact)) throw new Error(`no artifact at ${args.artifact}`);
  const signaturePath = `${args.artifact}.sig`;
  if (!existsSync(signaturePath)) {
    throw new Error(
      `no ${basename(signaturePath)} beside the artifact. Build with TAURI_SIGNING_PRIVATE_KEY ` +
        `and TAURI_SIGNING_PRIVATE_KEY_PASSWORD set, or the bundle carries no signature and ` +
        `the updater will reject it.`,
    );
  }

  const file = basename(args.artifact);
  const sigFile = basename(signaturePath);
  const key = keyFor({ ...args, file });
  const sigKey = keyFor({ ...args, file: sigFile });
  const url = `${ARTIFACT_HOST}/${key}`;

  // The tarball is immutable, so it can be cached forever; a version bump is a
  // new key. The .sig is beside it for anyone checking by hand.
  run(["r2", "object", "put", `${BUCKET}/${key}`, "--file", args.artifact, "--content-type", "application/octet-stream", "--cache-control", "public, max-age=31536000, immutable", "--remote"], args.dryRun);
  run(["r2", "object", "put", `${BUCKET}/${sigKey}`, "--file", signaturePath, "--content-type", "text/plain", "--cache-control", "public, max-age=31536000, immutable", "--remote"], args.dryRun);

  const signature = readFileSync(signaturePath, "utf8").trim();
  if (!signature) throw new Error(`${signaturePath} is empty`);

  writePointer(args, pointer({ ...args, url, signature }));
  console.log(`${args.platform} → ${args.version}${args.stage > 0 ? ` (staged ${args.stage}m)` : ""}`);
  console.log(url);
}

function writePointer(args, value) {
  const dir = mkdtempSync(join(tmpdir(), "marula-update-"));
  const file = join(dir, "pointer.json");
  writeFileSync(file, JSON.stringify(value));
  try {
    run(["kv", "key", "put", `latest:${args.platform}`, "--namespace-id", NAMESPACE, "--path", file, "--remote"], args.dryRun);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function readPointer(platform) {
  try {
    const raw = execFileSync("wrangler", ["kv", "key", "get", `latest:${platform}`, "--namespace-id", NAMESPACE, "--remote"], { encoding: "utf8" });
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

try {
  main();
} catch (error) {
  console.error(`update publish failed: ${error.message}`);
  process.exit(1);
}
