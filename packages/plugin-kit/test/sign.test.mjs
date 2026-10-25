// The kit's half of the v2 registry-signature statement.
//
// `crates/plugin-host/src/distribution.rs` verifies a statement over the
// artifact digest AND the canonical manifest digest (record
// `plugin-distribution.registry-signature-binds-digest-only`). The Rust side
// proves the same fixture; this file proves the publisher-facing signer derives
// it, on the exact bytes, from a plain catalog JSON.
//
// Run with `node --test test/` (the package's `npm test`).

import assert from "node:assert/strict";
import { createPrivateKey, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  canonicalJson,
  manifestSha256,
  signCatalog,
  statementBytes,
  statementMessage,
} from "../bin/sign.mjs";

const fixturePath = new URL("./fixtures/registry-statement-v2.json", import.meta.url);
const fixture = JSON.parse(readFileSync(fixturePath, "utf8"));

/** The seed as a PKCS#8 ed25519 key, without shipping a PEM in the repo. */
const privateKeyFromSeed = (seedHex) =>
  createPrivateKey({
    key: Buffer.concat([
      Buffer.from("302e020100300506032b657004220420", "hex"),
      Buffer.from(seedHex, "hex"),
    ]),
    format: "der",
    type: "pkcs8",
  });

test("canonical JSON sorts members by UTF-8 bytes and keeps values as declared", () => {
  assert.equal(
    canonicalJson({ b: 1, a: { d: [2, true, null], c: "x" } }),
    '{"a":{"c":"x","d":[2,true,null]},"b":1}',
  );
  // Non-ASCII stays literal, exactly as serde_json writes it.
  assert.equal(canonicalJson({ s: "café" }), '{"s":"café"}');
  // Arrays keep declaration order (they are not sets).
  assert.equal(canonicalJson([3, 1, 2]), "[3,1,2]");
  // Member order in the source object cannot change the output.
  assert.equal(
    canonicalJson({ a: { c: "x", d: [2, true, null] }, b: 1 }),
    canonicalJson({ b: 1, a: { d: [2, true, null], c: "x" } }),
  );
});

test("canonical JSON refuses numbers with no single cross-language spelling", () => {
  assert.throws(() => canonicalJson({ minimum: 0.5 }));
  assert.throws(() => canonicalJson({ n: 9007199254740992 }));
  assert.throws(() => canonicalJson({ n: Number.NaN }));
  // And a manifest that is not an object is not a manifest.
  assert.throws(() => manifestSha256([1, 2, 3]));
});

test("the statement bytes and signature match the vector the Rust host asserts", () => {
  const manifestDigest = manifestSha256(fixture.manifest);
  assert.equal(manifestDigest, fixture.expected_manifest_sha256);

  const statement = statementBytes(fixture.artifact_digest, manifestDigest);
  assert.equal(statement, fixture.expected_statement);

  const message = statementMessage(fixture.artifact_digest, manifestDigest);
  assert.equal(message.length, 32);

  const key = privateKeyFromSeed(fixture.private_key_seed_hex);
  const { catalog, signature } = signCatalog(
    { slug: "ticket-summarizer", tag: "v1.4.0", artifact: { digest: fixture.artifact_digest }, manifest: fixture.manifest },
    key,
    "ops-2026",
  );

  assert.equal(signature, fixture.expected_signature_hex);
  assert.deepEqual(catalog.artifact.statement, {
    version: 2,
    manifest_sha256: fixture.expected_manifest_sha256,
  });
  assert.deepEqual(catalog.artifact.signature, {
    key_id: "ops-2026",
    signature: fixture.expected_signature_hex,
  });
  // The signature verifies over the 32-byte message, not the statement text.
  assert.equal(
    verify(null, message, createPublicKey(key), Buffer.from(signature, "hex")),
    true,
  );
  // A substituted manifest produces a different digest, so its statement is not
  // the one the publisher signed.
  const substituted = manifestSha256({ ...fixture.manifest, description: "text nobody signed" });
  assert.notEqual(substituted, fixture.expected_manifest_sha256);
});

test("a catalog with no artifact digest or no manifest is refused", () => {
  const key = privateKeyFromSeed(fixture.private_key_seed_hex);
  assert.throws(() => signCatalog({ manifest: fixture.manifest }, key));
  assert.throws(() => signCatalog({ artifact: { digest: fixture.artifact_digest } }, key));
  assert.throws(() =>
    signCatalog({ artifact: { digest: "not-a-digest" }, manifest: fixture.manifest }, key),
  );
});
