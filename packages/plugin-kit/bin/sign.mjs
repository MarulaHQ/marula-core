#!/usr/bin/env node
// @marula-core/plugin-kit — sign a marketplace-registry catalog entry.
//
// Usage:
//   node bin/sign.mjs <catalog.json> --key <private-key.pem> [--key-id <id>] [--out <path>]
//
// The v2 signature statement, as `crates/plugin-host/src/distribution.rs`
// verifies it (record `plugin-distribution.registry-signature-binds-digest-only`):
//
//   marula-plugin-registry-statement-v2\n
//   artifact-sha256:<artifact.digest>\n
//   manifest-sha256:<sha256 of the canonical manifest JSON>\n
//
// The ed25519 signature covers `sha256(those bytes)`. `manifest_sha256` is the
// sha256 of the `manifest` member serialised canonically — object members in
// ascending UTF-8 byte order, no whitespace, arrays in order, integers in plain
// decimal. This file is the reference implementation publishers can run; the
// shared test vector in test/fixtures/registry-statement-v2.json is what keeps
// it byte-identical with the Rust verifier.
//
// The output catalog gains:
//   artifact.statement = { "version": 2, "manifest_sha256": "<hex>" }
//   artifact.signature = { "key_id": "<id>", "signature": "<128 hex>" }
// (or a bare signature hex when no key id is given).
//
// Non-integers, integers outside ±(2^53−1) and non-object manifests are
// refused: the canonical form has to be reproducible by any publisher tool, so
// a value with no single spelling is an error rather than a digest only this
// machine would compute.

import { createHash, createPrivateKey, sign } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const STATEMENT_PREFIX = "marula-plugin-registry-statement-v2";
const MAX_CANONICAL_INT = 9007199254740991; // 2^53 - 1

/** A refusal the caller can handle; the CLI turns it into exit 1 below. */
const refuse = (msg) => {
  throw new Error(msg);
};

const fail = (msg) => {
  console.error(`plugin-kit sign: ${msg instanceof Error ? msg.message : msg}`);
  process.exit(1);
};

/** Compare two strings by their UTF-8 bytes (the host's key order). */
const byUtf8Bytes = (a, b) => Buffer.compare(Buffer.from(a, "utf8"), Buffer.from(b, "utf8"));

/**
 * Canonical JSON for a manifest. Must stay byte-identical with
 * `canonical_manifest_json` in crates/plugin-host/src/distribution.rs.
 */
export function canonicalJson(value) {
  if (value === null) return "null";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") {
    if (!Number.isInteger(value) || !Number.isSafeInteger(value) || Object.is(value, -0))
      return refuse(
        `number ${value} is not a plain integer in ±${MAX_CANONICAL_INT}; the canonical form has no single spelling for it`,
      );
    return String(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (typeof value === "object") {
    const members = Object.keys(value)
      .sort(byUtf8Bytes)
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`);
    return `{${members.join(",")}}`;
  }
  return refuse(`cannot canonicalise a ${typeof value}`);
}

export function manifestSha256(manifest) {
  if (manifest === null || typeof manifest !== "object" || Array.isArray(manifest))
    return refuse("the catalog's `manifest` must be a JSON object");
  return createHash("sha256").update(canonicalJson(manifest), "utf8").digest("hex");
}

export function statementBytes(artifactDigest, manifestDigest) {
  const artifact = String(artifactDigest).trim().toLowerCase();
  const manifest = String(manifestDigest).trim().toLowerCase();
  for (const [what, digest] of [
    ["artifact.digest", artifact],
    ["manifest sha256", manifest],
  ]) {
    if (!/^[0-9a-f]{64}$/.test(digest))
      return refuse(`${what} is not a lowercase sha256 hex digest`);
  }
  return `${STATEMENT_PREFIX}\nartifact-sha256:${artifact}\nmanifest-sha256:${manifest}\n`;
}

/** The 32 bytes the ed25519 signature covers for a v2 statement. */
export function statementMessage(artifactDigest, manifestDigest) {
  return createHash("sha256").update(statementBytes(artifactDigest, manifestDigest), "utf8").digest();
}

/**
 * Returns a copy of `catalog` with `artifact.statement` and
 * `artifact.signature` set. Throws (via fail) on a shape the host would refuse.
 */
export function signCatalog(catalog, privateKey, keyId) {
  if (catalog === null || typeof catalog !== "object" || Array.isArray(catalog))
    return refuse("the catalog must be a JSON object");
  const artifact = catalog.artifact;
  if (artifact === null || typeof artifact !== "object" || Array.isArray(artifact))
    return refuse("the catalog must carry an `artifact` object");
  if (typeof artifact.digest !== "string")
    return refuse("the catalog must carry `artifact.digest` (the artifact's sha256 hex)");

  const manifestDigest = manifestSha256(catalog.manifest);
  const message = statementMessage(artifact.digest, manifestDigest);
  const signature = sign(null, message, privateKey).toString("hex");

  const signed = {
    ...artifact,
    statement: { version: 2, manifest_sha256: manifestDigest },
    signature:
      keyId === undefined || keyId === null || keyId === ""
        ? signature
        : { key_id: keyId, signature },
  };
  return { catalog: { ...catalog, artifact: signed }, manifestDigest, signature };
}

function main(argv) {
  const [input, ...rest] = argv;
  if (!input) {
    fail(
      "usage: node bin/sign.mjs <catalog.json> --key <private-key.pem> [--key-id <id>] [--out <path>]",
    );
  }
  let keyPath;
  let keyId;
  let out;
  for (let i = 0; i < rest.length; i += 1) {
    const flag = rest[i];
    if (flag === "--key") keyPath = rest[(i += 1)];
    else if (flag === "--key-id") keyId = rest[(i += 1)];
    else if (flag === "--out") out = rest[(i += 1)];
    else fail(`unknown argument ${flag}`);
  }
  if (keyPath === undefined) fail("--key <private-key.pem> is required");

  const catalog = JSON.parse(readFileSync(input, "utf8"));
  const privateKey = createPrivateKey({ key: readFileSync(keyPath, "utf8"), format: "pem" });
  if (privateKey.asymmetricKeyType !== "ed25519")
    fail(`the key is ${privateKey.asymmetricKeyType}, not ed25519`);

  const { catalog: signed, manifestDigest } = signCatalog(catalog, privateKey, keyId);
  const body = `${JSON.stringify(signed, null, 2)}\n`;
  if (out === undefined) process.stdout.write(body);
  else writeFileSync(out, body);
  console.error(`plugin-kit sign: manifest sha256 ${manifestDigest}`);
  if (out !== undefined) console.error(`plugin-kit sign: wrote ${out}`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    fail(error);
  }
}
