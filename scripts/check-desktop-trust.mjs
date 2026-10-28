#!/usr/bin/env node
/**
 * The desktop updater's trust anchor (Wave 60 follow-up, desktop updater lane).
 *
 * The app updates itself by verifying a minisign signature against the pubkey
 * committed in `desktop/tauri.conf.json`. Three things about that key are worth
 * refusing mechanically rather than reviewing by eye:
 *
 *  1. A PLACEHOLDER. The field once carried `REPLACE_AT_RELEASE_TIME: …`, and a
 *     placeholder that reaches a build is worse than an empty field: it looks
 *     like a key to every reader and to nothing that verifies signatures.
 *  2. A misshaped key. Tauri compares against base64 of a minisign public key
 *     document — `untrusted comment:` line, then the `RW…` key line. A key that
 *     is valid base64 but not that document fails at update time, on the user's
 *     machine, after the release is out.
 *  3. An endpoint that is not https. The updater downloads and executes the
 *     artifact it verifies; a plaintext channel hands an attacker the choice of
 *     which signed bytes to serve, which is a downgrade rather than a break.
 *
 * Exit 1 on the first failed invariant. `--self-test` proves the three refusals
 * in a temporary directory so the checker cannot rot into always-green.
 *
 * Usage: node scripts/check-desktop-trust.mjs [--self-test]
 */
import { readFileSync, mkdtempSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { tmpdir } from "node:os";

const PLACEHOLDER = /REPLACE_AT_RELEASE_TIME|REPLACE_ME|TODO_KEY/;

/** Check one parsed config. Returns a list of refusals, empty when it holds. */
export function inspect(config) {
  const refusals = [];
  const updater = config?.plugins?.updater;
  if (!updater) return refusals; // No updater configured is a supported state.

  const pubkey = updater.pubkey;
  if (typeof pubkey !== "string" || pubkey.trim() === "") {
    refusals.push("plugins.updater.pubkey is missing: an updater with no key verifies nothing");
  } else if (PLACEHOLDER.test(pubkey)) {
    refusals.push("plugins.updater.pubkey is a placeholder, not a key");
  } else {
    let decoded = "";
    try {
      decoded = Buffer.from(pubkey, "base64").toString("utf8");
    } catch {
      decoded = "";
    }
    const looksMinisign =
      decoded.startsWith("untrusted comment:") && /^RW[A-Za-z0-9+/=]{20,}$/m.test(decoded);
    if (!looksMinisign) {
      refusals.push(
        "plugins.updater.pubkey is not a minisign public key document " +
          "(expected base64 of `untrusted comment: …` followed by an `RW…` line)",
      );
    }
  }

  for (const endpoint of updater.endpoints ?? []) {
    if (!/^https:\/\//.test(String(endpoint))) {
      refusals.push(`plugins.updater.endpoints carries a non-https endpoint: ${endpoint}`);
    }
  }
  return refusals;
}

function configs() {
  const tracked = execFileSync("git", ["ls-files", "*tauri.conf.json"], {
    encoding: "utf8",
  })
    .split("\n")
    .filter(Boolean);
  return tracked;
}

function selfTest() {
  const dir = mkdtempSync(join(tmpdir(), "desktop-trust-"));
  const valid =
    "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDhENEYzNjA4NjQ5RThFRTEKUldUaGpwNWtDRFpQalU0VjAxZ1FTeE9MOGVPOXYyRkphbmdTVWpLWEovNkNxRFlXNEc4ZXlibXIK";
  const cases = [
    ["a placeholder is refused", { plugins: { updater: { pubkey: "REPLACE_AT_RELEASE_TIME: paste here", endpoints: ["https://x/y"] } } }, 1],
    ["a real key holds", { plugins: { updater: { pubkey: valid, endpoints: ["https://x/y"] } } }, 0],
    ["base64 that is not a minisign document is refused", { plugins: { updater: { pubkey: Buffer.from("hello").toString("base64"), endpoints: ["https://x/y"] } } }, 1],
    ["an http endpoint is refused", { plugins: { updater: { pubkey: valid, endpoints: ["http://x/y"] } } }, 1],
    ["no updater is a supported state", { plugins: {} }, 0],
  ];
  let failed = 0;
  for (const [name, config, expected] of cases) {
    const got = inspect(config).length > 0 ? 1 : 0;
    if (got !== expected) {
      console.error(`  ✗ self-test: ${name} (expected ${expected}, got ${got})`);
      failed += 1;
    } else {
      console.log(`  ok self-test: ${name}`);
    }
  }
  writeFileSync(join(dir, "scratch"), "");
  rmSync(dir, { recursive: true, force: true });
  if (failed > 0) process.exit(1);
  console.log("self-test: ok");
}

if (process.argv.includes("--self-test")) {
  selfTest();
} else {
  let failed = 0;
  const files = configs();
  if (files.length === 0) {
    console.error("no tauri.conf.json is tracked; the check found nothing to verify");
    process.exit(1);
  }
  for (const file of files) {
    const refusals = inspect(JSON.parse(readFileSync(file, "utf8")));
    if (refusals.length === 0) {
      console.log(`ok ${file}`);
    } else {
      failed += 1;
      for (const refusal of refusals) console.error(`✗ ${file}: ${refusal}`);
    }
  }
  if (failed > 0) process.exit(1);
  console.log(`desktop trust: ${files.length} config(s) hold`);
}
