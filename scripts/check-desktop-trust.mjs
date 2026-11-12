#!/usr/bin/env node
/**
 * The desktop updater's trust anchor (Wave 60 follow-up, desktop updater lane;
 * extended by W61-4 to the conditional signing the release workflow runs).
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
 * The release workflow gets the same treatment (W61-4), because signing there
 * is conditional and a conditional trust path rots quietly:
 *
 *  4. A secret the workflow reads but the trust model does not name, or a
 *     canonical name it stopped reading. The set is fixed; drift is a refusal.
 *  5. An unsigned run that is not stated. The Summary must carry the unsigned
 *     disclosure for the updater payload, macOS and Windows, or a dispatch
 *     without secrets ships a posture nobody declared.
 *  6. An unsigned run that still asks for updater artifacts. With
 *     `createUpdaterArtifacts` left on and no private key, `tauri build`
 *     refuses to build at all — the workflow must turn the flag off on the
 *     unsigned path, and must consult the signing gate's outputs to do it.
 *  7. A certificate used as a thumbprint. `WINDOWS_CERTIFICATE` is the .pfx
 *     file; a thumbprint is the hash of the certificate inside it. Handing the
 *     bundler the blob as `certificateThumbprint` looks like wiring and signs
 *     nothing.
 *
 * Exit 1 on the first failed invariant. `--self-test` proves the refusals in
 * memory and against a temporary directory, so the checker cannot rot into
 * always-green.
 *
 * Usage: node scripts/check-desktop-trust.mjs [--self-test]
 */
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { tmpdir } from "node:os";

const PLACEHOLDER = /REPLACE_AT_RELEASE_TIME|REPLACE_ME|TODO_KEY/;
const WORKFLOW = ".github/workflows/release-desktop.yml";

/** The canonical secret names the desktop release workflow may read. */
export const EXPECTED_SECRETS = [
  "TAURI_SIGNING_PRIVATE_KEY",
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
  "APPLE_SIGNING_IDENTITY",
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_TEAM_ID",
  "WINDOWS_CERTIFICATE",
  "WINDOWS_CERTIFICATE_PASSWORD",
  "WINDOWS_TIMESTAMP_URL",
];

/** The disclosure lines the job Summary must be able to print. */
export const UNSIGNED_DISCLOSURES = [
  "updater payload: unsigned",
  "macOS: unsigned",
  "Windows: unsigned",
];

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

/**
 * Check one release workflow's text for the conditional-signing posture.
 * Returns a list of refusals, empty when it holds.
 */
export function inspectWorkflow(text) {
  const refusals = [];

  for (const name of EXPECTED_SECRETS) {
    if (!text.includes(`secrets.${name}`)) {
      refusals.push(
        `the workflow does not read a secret named ${name}: a trust input the ` +
          "checker names has gone missing from the workflow",
      );
    }
  }

  const read = text.match(/secrets\.([A-Z0-9_]+)/g) ?? [];
  for (const match of read) {
    const name = match.slice("secrets.".length);
    if (!EXPECTED_SECRETS.includes(name)) {
      refusals.push(
        `the workflow reads an unexpected secret: ${name}. Every secret it ` +
          "reads must be one the trust model names (ADR 0028, docs/desktop-release.md §2)",
      );
    }
  }

  if (!text.includes("createUpdaterArtifacts")) {
    refusals.push(
      "the workflow never turns createUpdaterArtifacts off: with the flag left on " +
        "and no private key, the bundler refuses to build, so a run without " +
        "TAURI_SIGNING_PRIVATE_KEY has no path to a (disclosed) unsigned bundle",
    );
  }

  if (!text.includes("steps.signing.outputs")) {
    refusals.push(
      "the workflow never consults a signing gate's outputs: the posture a step " +
        "takes must come from the gate that checked the secrets, not from a guess",
    );
  }

  for (const disclosure of UNSIGNED_DISCLOSURES) {
    if (!text.includes(disclosure)) {
      refusals.push(
        `the workflow has no "${disclosure}" disclosure: a run without the ` +
          "secrets must state its unsigned posture in the Summary, not ship silently",
      );
    }
  }

  // The .pfx is a certificate file; a thumbprint is the SHA-1 of the
  // certificate inside it. Either interpolation form (secrets.X or env X)
  // handed to certificateThumbprint is wiring that cannot sign.
  const thumbprintMisuse =
    /certificateThumbprint"\s*:\s*['"`]{0,4}\$\{(?:\{\s*secrets\.)?WINDOWS_CERTIFICATE/;
  if (thumbprintMisuse.test(text)) {
    refusals.push(
      "the workflow hands the certificate blob to certificateThumbprint: " +
        "WINDOWS_CERTIFICATE is the .pfx to import into the store, and the " +
        "thumbprint comes from the import step",
    );
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
  const configCases = [
    ["a placeholder is refused", { plugins: { updater: { pubkey: "REPLACE_AT_RELEASE_TIME: paste here", endpoints: ["https://x/y"] } } }, 1],
    ["a real key holds", { plugins: { updater: { pubkey: valid, endpoints: ["https://x/y"] } } }, 0],
    ["base64 that is not a minisign document is refused", { plugins: { updater: { pubkey: Buffer.from("hello").toString("base64"), endpoints: ["https://x/y"] } } }, 1],
    ["an http endpoint is refused", { plugins: { updater: { pubkey: valid, endpoints: ["http://x/y"] } } }, 1],
    ["no updater is a supported state", { plugins: {} }, 0],
  ];

  const workflowLines = [
    "env:",
    "  APPLE_CERTIFICATE: \${{ secrets.APPLE_CERTIFICATE }}",
    "  APPLE_CERTIFICATE_PASSWORD: \${{ secrets.APPLE_CERTIFICATE_PASSWORD }}",
    "  APPLE_SIGNING_IDENTITY: \${{ secrets.APPLE_SIGNING_IDENTITY }}",
    "  APPLE_ID: \${{ secrets.APPLE_ID }}",
    "  APPLE_PASSWORD: \${{ secrets.APPLE_PASSWORD }}",
    "  APPLE_TEAM_ID: \${{ secrets.APPLE_TEAM_ID }}",
    "  WINDOWS_CERTIFICATE: \${{ secrets.WINDOWS_CERTIFICATE }}",
    "  WINDOWS_CERTIFICATE_PASSWORD: \${{ secrets.WINDOWS_CERTIFICATE_PASSWORD }}",
    "  WINDOWS_TIMESTAMP_URL: \${{ secrets.WINDOWS_TIMESTAMP_URL }}",
    "  TAURI_SIGNING_PRIVATE_KEY: \${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
    "  TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}",
    "steps:",
    "  - if: matrix.os == 'macos' && steps.signing.outputs.macos == 'true'",
    "  - run: ARGS+=(--config '{\"bundle\":{\"createUpdaterArtifacts\":false}}')",
    "  - run: echo '- updater payload: unsigned — TAURI_SIGNING_PRIVATE_KEY was not set' >> \"$GITHUB_STEP_SUMMARY\"",
    "  - run: echo '- macOS: unsigned — the Apple signing secrets were not set' >> \"$GITHUB_STEP_SUMMARY\"",
    "  - run: echo '- Windows: unsigned — the Authenticode secrets were not set' >> \"$GITHUB_STEP_SUMMARY\"",
  ];
  const goodWorkflow = workflowLines.join("\n");
  const without = (line) => workflowLines.filter((l) => l !== line).join("\n");
  const workflowCases = [
    ["a conditional-signing workflow holds", goodWorkflow, 0],
    [
      "a dropped canonical secret name is refused",
      without("  APPLE_PASSWORD: \${{ secrets.APPLE_PASSWORD }}"),
      1,
    ],
    [
      "an unexpected secret is refused",
      goodWorkflow + "\n  DEPLOY_TOKEN: \${{ secrets.DEPLOY_TOKEN }}",
      1,
    ],
    [
      "an unsigned run that still asks for updater artifacts is refused",
      workflowLines.filter((line) => !line.includes("createUpdaterArtifacts")).join("\n"),
      1,
    ],
    [
      "a workflow that never consults the gate is refused",
      goodWorkflow.replace("steps.signing.outputs.macos", "matrix.os"),
      1,
    ],
    [
      "a missing unsigned disclosure is refused",
      without("  - run: echo '- updater payload: unsigned — TAURI_SIGNING_PRIVATE_KEY was not set' >> \"$GITHUB_STEP_SUMMARY\""),
      1,
    ],
    [
      "the certificate blob used as a thumbprint is refused",
      goodWorkflow + "\n  - run: ARGS+=(--config '{\"bundle\":{\"windows\":{\"certificateThumbprint\":\"'\"${WINDOWS_CERTIFICATE}\"'\"}}}')",
      1,
    ],
    [
      "the secrets-form certificate blob used as a thumbprint is refused",
      goodWorkflow + "\n  - run: ARGS+=(--config '{\"bundle\":{\"windows\":{\"certificateThumbprint\":\"${{ secrets.WINDOWS_CERTIFICATE }}\"}}}')",
      1,
    ],
  ];

  let failed = 0;
  const run = (kind, name, got, expected) => {
    if (got !== expected) {
      console.error(`  ✗ self-test (${kind}): ${name} (expected ${expected}, got ${got})`);
      failed += 1;
    } else {
      console.log(`  ok self-test (${kind}): ${name}`);
    }
  };
  for (const [name, config, expected] of configCases) {
    run("config", name, inspect(config).length > 0 ? 1 : 0, expected);
  }
  for (const [name, text, expected] of workflowCases) {
    run("workflow", name, inspectWorkflow(text).length > 0 ? 1 : 0, expected);
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
  try {
    const workflowRefusals = inspectWorkflow(readFileSync(WORKFLOW, "utf8"));
    if (workflowRefusals.length === 0) {
      console.log(`ok ${WORKFLOW}`);
    } else {
      failed += 1;
      for (const refusal of workflowRefusals) console.error(`✗ ${WORKFLOW}: ${refusal}`);
    }
  } catch (error) {
    failed += 1;
    console.error(`✗ ${WORKFLOW}: cannot be read (${error.message}); `);
    console.error("  the desktop release workflow is where the signing posture lives");
  }
  if (failed > 0) process.exit(1);
  console.log(`desktop trust: ${files.length + 1} file(s) hold`);
}
