//! The distribution SOURCE ADAPTERS (Wave 16 lane 7, ADR 0009 §2): one
//! seam, four v1 sources, one provenance chain. Every adapter's output
//! is a TABLE-STAKES bundle — a validated declarative manifest + the
//! typed provenance `{source_kind, source_url, fetched_ref,
//! artifact_digest}` — which the route layer registers as a STAGED
//! row (deny-by-default): nothing adopts or executes anything until the
//! owner/admin consent diff passes.
//!
//! Doctrine (non-negotiable, mirrors the seam doctrine in `lib.rs`):
//! * Sources are adapters, not governance regimes — a dist-source never
//!   changes the deny-by-default lifecycle; it only fabricates the
//!   manifest + provenance the same consent screen consumes.
//! * No eval-style loading anywhere: a bundle is a declarative manifest
//!   adopted from a digest-verified artifact. Adapters CHECK digests and
//!   pins; they never unzip-and-run code.
//! * GitHub pulls are tag/commit-pinned, never HEAD — a repo without a
//!   pinned tag is refused (`distribution_source_not_pinned`).

use core_domain::entities::agent_skills::SkillManifest;
use core_domain::entities::plugins::{
    is_sha256_hex, ConsentDiff, PluginArtifactKind, PluginCapability, PluginManifestV2,
    PluginSourceKind,
};
use sha2::{Digest, Sha256};

/// sha256 digest of some artifact bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

// --- B1: ed25519 artifact signature verification (Wave 16 lane 9) ---------
//
// Signing conventions (documented, stable). TWO statements exist, and the
// difference is WHAT the publisher vouched for:
//
// * **v1 — the artifact digest alone.** The operator's ed25519 PUBLIC key
//   (64 hex chars) is configured server-side (`MARULA_PLUGIN_SIGNING_KEY`,
//   or the `plugin_signing_keys` registry); the signature is over the RAW
//   32 BYTES the artifact digest hex decodes to, hex-encoded (128 chars) on
//   the wire. It authenticates the executable and says nothing about the
//   manifest printed beside it on the consent screen, so a published
//   (digest, signature) pair can be replayed next to a substituted
//   manifest. Registry entries published before Wave 55 all carry this
//   statement; they are ACCEPTED and marked `manifest_authenticated =
//   false` (see `ingest_registry_entry`).
//
// * **v2 — the artifact digest AND the manifest** (Wave 55, record
//   `plugin-distribution.registry-signature-binds-digest-only`). The
//   registry document carries `artifact.statement`:
//
//       {"version": 2, "manifest_sha256": "<64 lowercase hex>"}
//
//   and exactly these bytes are signed (UTF-8; `\n` is 0x0A; no other
//   whitespace; both digests lowercase hex, so every field is a fixed
//   length and no two field combinations can frame the same text):
//
//       marula-plugin-registry-statement-v2\n
//       artifact-sha256:<artifact.digest>\n
//       manifest-sha256:<manifest_sha256>\n
//
//   The ed25519 signature is over `sha256(those bytes)` — the same
//   32-raw-bytes convention v1 uses, and the leading convention line is
//   what keeps a signature from moving between the two statements: v1
//   signs a digest, v2 signs the hash of a framed, domain-separated
//   statement about two digests.
//
//   `manifest_sha256` is the sha256 of the CANONICAL JSON of the
//   document's `manifest` member ([`canonical_manifest_json`]): object
//   keys sorted by UTF-8 bytes, no insignificant whitespace, arrays in
//   declaration order, strings escaped as serde_json escapes them, and
//   integers — only integers — in plain decimal. Hashing a canonical
//   serialisation is what stops two different manifests reaching the same
//   signed bytes; the statement framing is what stops a manifest digest
//   being read as an artifact digest.
//
// Verification is a REAL cryptographic check — a configured key plus any
// invalid/missing signature refuses the artifact with `BadSignature`; with
// NO key configured the seam degrades to the structural-only check and
// says so loudly in the log (dev posture, never production), and every
// bundle it passes carries `manifest_authenticated = false` because
// nothing was verified.

use ed25519_dalek::{Signature, VerifyingKey};

/// One trusted operator key as the verification seam sees it. The
/// registry (`plugin_signing_keys`, migration 0095) is the source of
/// truth; this is the in-memory shape the signature path consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedSigningKey {
    /// The stable trust identity the signature block names. Defaults to
    /// the first 16 hex chars of the public key when the operator does
    /// not name one.
    pub key_id: String,
    /// The ed25519 PUBLIC key (64 hex chars) — verification material.
    pub public_key: String,
}

/// One ed25519 verification: key + message + wire signature (hex). The
/// typed refusal is ALWAYS [`IngestReason::BadSignature`] — key material
/// never reaches error text.
pub fn verify_signature_over(
    signing_key_hex: &str,
    message: &[u8],
    signature_hex: &str,
) -> Result<(), IngestReason> {
    let key_bytes: [u8; 32] = hex::decode(signing_key_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(IngestReason::BadSignature)?;
    let vk = VerifyingKey::from_bytes(&key_bytes).map_err(|_| IngestReason::BadSignature)?;
    let sig_bytes: [u8; 64] = hex::decode(signature_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(IngestReason::BadSignature)?;
    let sig = Signature::from_bytes(&sig_bytes);
    vk.verify_strict(message, &sig)
        .map_err(|_| IngestReason::BadSignature)
}

/// The v1 statement: key + artifact digest hex + wire signature (hex).
/// The digest hex always decodes to the signed 32-byte message; any shape
/// problem is [`IngestReason::BadSignature`].
pub fn verify_artifact_signature(
    signing_key_hex: &str,
    artifact_digest_hex: &str,
    signature_hex: &str,
) -> Result<(), IngestReason> {
    let digest_bytes: [u8; 32] = hex::decode(artifact_digest_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(IngestReason::BadSignature)?;
    verify_signature_over(signing_key_hex, &digest_bytes, signature_hex)
}

/// Verifies a signature against the trusted key set by trying every key.
/// Returns [`IngestReason::NoTrustedKeys`] when the trust set is EMPTY (a
/// key-configured operator whose registry is unpopulated must not
/// silently degrade to structural-only — the single-key seam's dev
/// posture never re-enters through rotation) or
/// [`IngestReason::BadSignature`] when nothing verifies.
pub fn verify_signature_over_against_keys(
    keys: &[TrustedSigningKey],
    message: &[u8],
    signature_hex: &str,
) -> Result<(), IngestReason> {
    if keys.is_empty() {
        return Err(IngestReason::NoTrustedKeys);
    }
    for key in keys {
        if verify_signature_over(&key.public_key, message, signature_hex).is_ok() {
            return Ok(());
        }
    }
    Err(IngestReason::BadSignature)
}

/// v1 trust-set verification over the artifact digest. With exactly one
/// trusted key this is byte-for-byte the old single-key behavior.
pub fn verify_artifact_signature_against_keys(
    keys: &[TrustedSigningKey],
    artifact_digest_hex: &str,
    signature_hex: &str,
) -> Result<(), IngestReason> {
    let digest_bytes: [u8; 32] = hex::decode(artifact_digest_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(IngestReason::BadSignature)?;
    verify_signature_over_against_keys(keys, &digest_bytes, signature_hex)
}

/// The statement version this build signs and verifies. A registry entry
/// that names any other version is refused rather than guessed at: a
/// statement the verifier cannot read is not a statement it can accept.
pub const SIGNATURE_STATEMENT_VERSION: u64 = 2;

/// The v2 statement as carried by `artifact.statement`. The version is
/// already known to be [`SIGNATURE_STATEMENT_VERSION`] by the time this
/// shape exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureStatement {
    /// sha256 (lowercase hex) of the canonical manifest JSON.
    pub manifest_sha256: String,
}

impl SignatureStatement {
    /// Parses `{ "version": 2, "manifest_sha256": "<hex>" }`. An unknown
    /// version is [`IngestReason::SignatureStatementUnsupported`]; a
    /// malformed object, a non-integer version or a non-sha256 digest is
    /// [`IngestReason::BadSignature`].
    pub fn from_value(value: &serde_json::Value) -> Result<Self, IngestReason> {
        let map = value.as_object().ok_or(IngestReason::BadSignature)?;
        let version = map
            .get("version")
            .and_then(|v| v.as_u64())
            .ok_or(IngestReason::BadSignature)?;
        if version != SIGNATURE_STATEMENT_VERSION {
            return Err(IngestReason::SignatureStatementUnsupported);
        }
        let manifest_sha256 = map
            .get("manifest_sha256")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| is_sha256_hex(s))
            .ok_or(IngestReason::BadSignature)?;
        Ok(Self {
            manifest_sha256: manifest_sha256.to_string(),
        })
    }
}

/// The exact bytes a v2 statement signs, before hashing. Fixed-length
/// fields in a fixed order behind a version line: no concatenation of
/// variable-length parts, so no two (artifact, manifest) pairs can produce
/// the same statement text.
pub fn statement_bytes(artifact_digest_hex: &str, manifest_sha256_hex: &str) -> String {
    format!(
        "marula-plugin-registry-statement-v2\nartifact-sha256:{}\nmanifest-sha256:{}\n",
        artifact_digest_hex.trim().to_lowercase(),
        manifest_sha256_hex.trim().to_lowercase()
    )
}

/// The signed message for a v2 statement: `sha256(statement_bytes(..))`,
/// the 32 raw bytes the ed25519 signature covers.
pub fn statement_message(artifact_digest_hex: &str, manifest_sha256_hex: &str) -> [u8; 32] {
    Sha256::digest(statement_bytes(artifact_digest_hex, manifest_sha256_hex).as_bytes()).into()
}

/// The largest integer the canonical form accepts. JSON numbers are IEEE-754
/// doubles in most publisher languages, so anything above this cannot round-trip
/// through the JS reference signer unchanged and is refused rather than hashed
/// into a digest only this server could reproduce.
const MAX_CANONICAL_INT: i128 = 9_007_199_254_740_991; // 2^53 - 1

/// Canonical JSON for a manifest (the bytes `manifest_sha256` is taken over).
///
/// The rules, in full: object members in ascending order of their key's UTF-8
/// bytes; no whitespace between tokens; arrays in declaration order; `null`,
/// `true` and `false` spelled as JSON spells them; strings serialised by
/// serde_json (which escapes only what JSON requires); numbers rendered as
/// plain decimal integers. A non-integer number, or an integer outside
/// ±(2^53−1), is [`IngestReason::ManifestNotCanonical`] — the canonical form
/// is a contract a publisher tool can reproduce, not a float printer's
/// dialect.
pub fn canonical_manifest_json(value: &serde_json::Value) -> Result<String, IngestReason> {
    let mut out = String::new();
    write_canonical(&mut out, value)?;
    Ok(out)
}

/// The sha256 (lowercase hex) of [`canonical_manifest_json`].
pub fn manifest_sha256(value: &serde_json::Value) -> Result<String, IngestReason> {
    Ok(sha256_hex(canonical_manifest_json(value)?.as_bytes()))
}

fn write_canonical(out: &mut String, value: &serde_json::Value) -> Result<(), IngestReason> {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(true) => out.push_str("true"),
        serde_json::Value::Bool(false) => out.push_str("false"),
        serde_json::Value::Number(number) => {
            let rendered = if let Some(i) = number.as_i64() {
                i.to_string()
            } else if let Some(u) = number.as_u64() {
                u.to_string()
            } else {
                // A fractional or exponent form: not part of the contract.
                return Err(IngestReason::ManifestNotCanonical);
            };
            let magnitude: i128 = rendered
                .parse::<i128>()
                .map_err(|_| IngestReason::ManifestNotCanonical)?;
            if magnitude.abs() > MAX_CANONICAL_INT {
                return Err(IngestReason::ManifestNotCanonical);
            }
            out.push_str(&rendered);
        }
        serde_json::Value::String(s) => out
            .push_str(&serde_json::to_string(s).expect("a JSON string always serialises to JSON")),
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(out, item)?;
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(
                    &serde_json::to_string(key).expect("a JSON string always serialises to JSON"),
                );
                out.push(':');
                write_canonical(out, &map[key])?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// The signature block as carried on a registry artifact or a pinned
/// manifest. TWO accepted shapes:
///   * legacy bare string `"<128-hex sig>"` — verified AGAINST THE TRUST
///     SET (any trusted key may have signed it; rotation overlap keeps
///     retired keys in the set so old artifacts keep verifying);
///   * the key-id block `{"key_id": "...", "signature": "<128-hex>"}` —
///     resolution pins EXACTLY one key; an unknown key_id is a typed
///     refusal (`distribution_signature_key_unknown`), never a fallback
///     to a wider trust set (an attacker must not be able to steer a
///     no-key_id artifact past a half-rotated registry).
#[derive(Debug, Clone)]
pub struct SignatureBlock {
    pub key_id: Option<String>,
    pub signature: String,
}

impl SignatureBlock {
    /// Parses the `signature` field from the wire: a string or the
    /// `{"key_id", "signature"}` object. Non-string signature values
    /// refuse.
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::String(sig) => Some(Self {
                key_id: None,
                signature: sig.clone(),
            }),
            serde_json::Value::Object(map) => {
                let sig = map.get("signature")?.as_str()?.to_string();
                let key_id = map
                    .get("key_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                Some(Self {
                    key_id,
                    signature: sig,
                })
            }
            _ => None,
        }
    }

    /// Verifies this block against `keys`. A named key_id MUST resolve
    /// (narrow resolution); an unnamed block sweeps the trust set. An
    /// EMPTY trust set refuses.
    pub fn verify(&self, keys: &[TrustedSigningKey], digest_hex: &str) -> Result<(), IngestReason> {
        let digest_bytes: [u8; 32] = hex::decode(digest_hex.trim())
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or(IngestReason::BadSignature)?;
        self.verify_message(keys, &digest_bytes)
    }

    /// Verifies this block over an arbitrary 32-byte message (the v2
    /// statement's hash). Same narrow-resolution rules as [`Self::verify`].
    pub fn verify_message(
        &self,
        keys: &[TrustedSigningKey],
        message: &[u8],
    ) -> Result<(), IngestReason> {
        if keys.is_empty() {
            return Err(IngestReason::NoTrustedKeys);
        }
        if let Some(key_id) = &self.key_id {
            // jotted narrow trust: block names a key, OTHER keys do not
            // count even if their material verifies — naming is
            // narrowing (the rotation model's tamper-evidence axis).
            for key in keys {
                if key.key_id == *key_id {
                    return verify_signature_over(&key.public_key, message, &self.signature);
                }
            }
            return Err(IngestReason::UnknownKey);
        }
        verify_signature_over_against_keys(keys, message, &self.signature)
    }
}

/// B2 helper — cross-digest verification of RE-FETCHED artifact bytes:
/// the sha256 over the actual bytes must equal the digest the catalog /
/// manifest pinned. `distribution_digest_invalid` on any mismatch.
pub fn verify_artifact_bytes(bytes: &[u8], expected_digest: &str) -> Result<(), IngestReason> {
    require_sha256(expected_digest)?;
    if sha256_hex(bytes) == expected_digest.trim().to_lowercase() {
        Ok(())
    } else {
        Err(IngestReason::BadDigest)
    }
}

/// Typed refusal reasons (deny-by-default surface — typed codes, never
/// prose to parse; these ride audit details and API errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestReason {
    /// A GitHub pull without pinned tag/commit (or pinned to HEAD).
    NoPin,
    /// Digest missing or not a sha256 hex string.
    BadDigest,
    /// The registry signature block missing/empty (code-signed artifacts).
    BadSignature,
    /// The signature block named a key_id that is not in the trusted set.
    UnknownKey,
    /// The signature block carried a statement version this build does not
    /// verify (only v2 exists; an unreadable statement is not accepted).
    SignatureStatementUnsupported,
    /// The signature's `manifest_sha256` does not match the manifest beside
    /// it — the digest says one document, the document is another.
    ManifestDigestMismatch,
    /// The manifest carries a number the canonical form cannot render (a
    /// float or an integer outside ±(2^53−1)), so no publisher tool could
    /// reproduce the digest this server would compute.
    ManifestNotCanonical,
    /// No trusted key is configured (operator registry empty AND no
    /// legacy env key) while signature policy demands one.
    NoTrustedKeys,
    /// The manifest is unreadable/unknown/malformed.
    BadManifest,
    /// The compressed archive itself is larger than `MAX_ARCHIVE_BYTES`.
    ArchiveCap,
    /// The archive carries more members than `MAX_ARCHIVE_ENTRIES`.
    EntryCap,
    /// One member's bytes — declared or produced — crossed `MAX_MEMBER_BYTES`.
    MemberCap,
    /// The archive's total decompressed size crossed `MAX_ARCHIVE_UNCOMPRESSED`.
    ExpansionCap,
    /// A member's header declared a size its stream does not produce.
    DeclaredSizeMismatch,
    /// Not a zipper we accept (bad magic / traversal paths / dupes, or a
    /// member whose stream does not hold what its header describes).
    BadArchive,
    /// An identity-bundle artifact tried to arrive at Marula authority —
    /// a `capabilities`/`permissions` key, or a scope spelling a plugin
    /// capability. Refused rather than stripped, so the author learns.
    CapabilityRefused,
}

impl std::fmt::Display for IngestReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl IngestReason {
    pub fn as_str(self) -> &'static str {
        match self {
            IngestReason::NoPin => "distribution_source_not_pinned",
            IngestReason::BadDigest => "distribution_digest_invalid",
            IngestReason::BadSignature => "distribution_signature_invalid",
            IngestReason::UnknownKey => "distribution_signature_key_unknown",
            IngestReason::SignatureStatementUnsupported => {
                "distribution_signature_statement_unsupported"
            }
            IngestReason::ManifestDigestMismatch => "distribution_manifest_digest_mismatch",
            IngestReason::ManifestNotCanonical => "distribution_manifest_not_canonical",
            IngestReason::NoTrustedKeys => "distribution_signing_key_unconfigured",
            IngestReason::BadManifest => "distribution_manifest_invalid",
            IngestReason::ArchiveCap => "distribution_archive_cap",
            IngestReason::EntryCap => "distribution_archive_entry_cap",
            IngestReason::MemberCap => "distribution_archive_member_cap",
            IngestReason::ExpansionCap => "distribution_archive_expansion_cap",
            IngestReason::DeclaredSizeMismatch => "distribution_archive_declared_size_mismatch",
            IngestReason::BadArchive => "distribution_archive_invalid",
            IngestReason::CapabilityRefused => "distribution_identity_bundle_capability_refused",
        }
    }
}

/// A validated bundle produced by a source adapter: everything the
/// staged-install route needs — the manifest (JSON), the closed
/// declared capability keys, and the provenance chain entry.
#[derive(Debug, Clone)]
pub struct DistributionBundle {
    pub artifact_kind: PluginArtifactKind,
    pub source_kind: PluginSourceKind,
    pub source_url: String,
    pub fetched_ref: Option<String>,
    pub artifact_digest: String,
    /// The full declarative manifest (plugin manifest v2 incl.
    /// settings_schema, or the skill manifest), as JSON for the staged
    /// row / consent screen.
    pub manifest: serde_json::Value,
    /// Declaratively-requested capability keys for the consent diff.
    pub declared_capabilities: Vec<String>,
    /// Whether a trusted key cryptographically verified a statement that
    /// covers THIS manifest, not just the artifact digest.
    ///
    /// `true` only for a registry entry whose v2 statement verified. `false`
    /// for a legacy digest-only (v1) registry entry, for every source that
    /// carries no publisher signature at all (zip, skill, theme), and for any
    /// ingest run with no trusted key configured — the last case because
    /// nothing was checked, which is exactly what a consent screen should say.
    /// The staged row carries this to the approver; it is never inferred from
    /// the publisher's prose.
    pub manifest_authenticated: bool,
}

impl DistributionBundle {
    pub fn manifest_name(&self) -> Option<String> {
        self.manifest
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    pub fn manifest_version(&self) -> Option<String> {
        self.manifest
            .get("version")
            .and_then(|v| v.as_str())
            .map(String::from)
    }
}

// --- sandbox caps (zip import) ------------------------------------------------
//
// Every cap below is checked against what the archive DECLARES in its
// central directory before a member is decompressed, and the read helpers
// re-check what the stream PRODUCES. A declaration is evidence, never the
// bound: the buffer is sized by these constants, and a header that disagrees
// with its own stream is refused instead of believed.

/// Maximum accepted zip import size (bytes) — the compressed archive itself.
pub const MAX_ARCHIVE_BYTES: i64 = 8 * 1024 * 1024;
/// Maximum members inside one archive. Deflate is allowed to expand a member
/// roughly a thousandfold, so a cheap archive of many small members is one
/// more way to ask for CPU; the ceiling keeps the directory walk flat.
pub const MAX_ARCHIVE_ENTRIES: usize = 256;
/// Maximum per-member uncompressed size (bytes).
pub const MAX_MEMBER_BYTES: usize = 4 * 1024 * 1024;
/// Maximum total decompressed size across members (zip-bomb guard). Checked
/// from the directory's declared sizes before any member is read, then again
/// against the bytes actually produced.
pub const MAX_ARCHIVE_UNCOMPRESSED: usize = 8 * 1024 * 1024;

// --- zip sandbox helpers ------------------------------------------------------

/// Rejects an archive whose declared shape breaches a cap.
///
/// Reads central-directory metadata only — `by_index_data` hands back the
/// entry record without a decompressor — so an archive declaring more than
/// `MAX_ARCHIVE_UNCOMPRESSED` in total, or a member past `MAX_MEMBER_BYTES`,
/// is refused before a single byte is expanded. `MAX_ARCHIVE_ENTRIES` is
/// checked first because it is the cheapest refusal of all.
fn check_archive_caps<R: std::io::Read + std::io::Seek>(
    archive: &zip::ZipArchive<R>,
) -> Result<(), IngestReason> {
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(IngestReason::EntryCap);
    }
    let mut total = 0usize;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_data(i)
            .map_err(|_| IngestReason::BadArchive)?;
        // Traversal-safety: no "/" or ".." — a zip import never aims at
        // any filesystem path.
        let Ok(name) = std::str::from_utf8(entry.name_raw()) else {
            return Err(IngestReason::BadArchive);
        };
        if name.contains('/') || name.contains('\\') || name.starts_with('.') {
            return Err(IngestReason::BadArchive);
        }
        let declared = usize::try_from(entry.size()).map_err(|_| IngestReason::MemberCap)?;
        if declared > MAX_MEMBER_BYTES {
            return Err(IngestReason::MemberCap);
        }
        total = total
            .checked_add(declared)
            .ok_or(IngestReason::ExpansionCap)?;
        if total > MAX_ARCHIVE_UNCOMPRESSED {
            return Err(IngestReason::ExpansionCap);
        }
    }
    Ok(())
}

/// Reads one member under a hard ceiling, and refuses a header that lies.
///
/// The buffer grows only while the produced length stays within `declared`,
/// within `budget` (what remains of `MAX_ARCHIVE_UNCOMPRESSED`) and within
/// `MAX_MEMBER_BYTES`. A member that produces more than it declared is refused
/// before the excess is copied in, and a member that ends short of its
/// declared size is refused too: either direction means the header is not a
/// description of the bytes.
///
/// The `zip` reader independently stops at the declared size, so the
/// under-declaring direction usually surfaces as a read error before this
/// function sees the excess; that refusal is `BadArchive`, and the checks here
/// are what hold if the reader's own enforcement ever goes away.
fn read_member<R: std::io::Read>(
    entry: &mut R,
    declared: usize,
    budget: usize,
) -> Result<Vec<u8>, IngestReason> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let n = entry
            .read(&mut chunk)
            .map_err(|_| IngestReason::BadArchive)?;
        if n == 0 {
            break;
        }
        let produced = buf.len() + n;
        if produced > declared {
            return Err(IngestReason::DeclaredSizeMismatch);
        }
        if produced > MAX_MEMBER_BYTES {
            return Err(IngestReason::MemberCap);
        }
        if produced > budget {
            return Err(IngestReason::ExpansionCap);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    if buf.len() != declared {
        return Err(IngestReason::DeclaredSizeMismatch);
    }
    Ok(buf)
}

/// The declared size of one entry, refused when it cannot even be a size.
fn declared_size(
    entry: &zip::read::ZipFile<'_, impl std::io::Read>,
) -> Result<usize, IngestReason> {
    usize::try_from(entry.size()).map_err(|_| IngestReason::MemberCap)
}

// --- adapters -------------------------------------------------------------------

/// ADR 0009 §2 — public marketplace registry (BUILD-PROMPT v1 fixture
/// shape: static catalog JSON + git tag + code-signature/digest). The
/// registry itself is the review channel for this source, but the org
/// still lands through the staged consent diff.
///
/// The signature is verified HERE, before the manifest is trusted, and what
/// it covers decides the bundle's `manifest_authenticated`:
///
/// * `artifact.statement` present → v2. The statement must name version 2,
///   its `manifest_sha256` must be the digest of the manifest in this very
///   document, and the signature must verify over the framed statement. Any
///   disagreement refuses (`distribution_manifest_digest_mismatch`,
///   `distribution_signature_statement_unsupported`,
///   `distribution_signature_invalid`).
/// * no statement → v1, the digest-only signature published before Wave 55.
///   Accepted (refusing it would strand every plugin already on the
///   registry), verified against the digest as before, and marked
///   `manifest_authenticated = false` so the approver sees the weaker claim.
///
/// With no trusted key configured the seam is structural-only and the bundle
/// is marked unauthenticated whatever the document claims.
pub fn ingest_registry_entry(
    catalog_json: &[u8],
    signing_keys: &[TrustedSigningKey],
) -> Result<DistributionBundle, IngestReason> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(catalog_json) else {
        return Err(IngestReason::BadManifest);
    };
    let tag = required_string(&value, "tag")?;
    validate_git_tag(tag)?;
    let artifact = value.get("artifact").ok_or(IngestReason::BadManifest)?;
    let digest = required_string(artifact, "digest")?;
    require_sha256(digest)?;
    let signature =
        SignatureBlock::from_value(artifact.get("signature").ok_or(IngestReason::BadManifest)?)
            .ok_or(IngestReason::BadManifest)?;
    if signature.signature.trim().is_empty() {
        return Err(IngestReason::BadSignature);
    }
    let manifest_value = value.get("manifest").ok_or(IngestReason::BadManifest)?;
    // B1: with a trusted key configured the signature is
    // CRYPTOGRAPHICALLY verified by key-id block or trust set; an EMPTY
    // slice stays the structural-only dev posture (the route layer logs
    // that loudly at boot — never production).
    let mut manifest_authenticated = false;
    if signing_keys.is_empty() {
        tracing::warn!(
            source = "registry",
            "no trusted plugin-signing key is configured: the artifact digest and \
             the manifest are unverified (structural check only)"
        );
    } else if let Some(statement_value) = artifact.get("statement") {
        let statement = SignatureStatement::from_value(statement_value)?;
        let computed = manifest_sha256(manifest_value)?;
        if computed != statement.manifest_sha256 {
            return Err(IngestReason::ManifestDigestMismatch);
        }
        let message = statement_message(digest, &statement.manifest_sha256);
        signature.verify_message(signing_keys, &message)?;
        manifest_authenticated = true;
    } else {
        // Legacy v1: the signature covers the artifact digest alone. Keep
        // accepting it — an already-published entry has no v2 statement to
        // produce — and let the staged row carry the weaker claim to the
        // approver.
        signature.verify(signing_keys, digest)?;
        tracing::warn!(
            source = "registry",
            artifact_digest = digest,
            "legacy digest-only signature: the manifest is NOT covered by it \
             (staged with manifest_authenticated = false)"
        );
    }
    let slug = value
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            manifest_value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unspecified")
        });
    if is_identity_bundle_manifest(manifest_value) {
        let manifest = validate_identity_bundle_manifest(manifest_value)?;
        return Ok(mark_manifest(
            bundle(
                PluginArtifactKind::IdentityBundle,
                PluginSourceKind::MarketplaceRegistry,
                &format!("registry://{slug}"),
                Some(tag),
                digest,
                serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
                Vec::new(),
            ),
            manifest_authenticated,
        ));
    }
    if is_theme_manifest(manifest_value) {
        let pack = validate_theme_manifest(manifest_value)?;
        return Ok(mark_manifest(
            bundle(
                PluginArtifactKind::Theme,
                PluginSourceKind::MarketplaceRegistry,
                &format!("registry://{slug}"),
                Some(tag),
                digest,
                serde_json::to_value(&pack).unwrap_or(serde_json::Value::Null),
                Vec::new(),
            ),
            manifest_authenticated,
        ));
    }
    let manifest = validate_plugin_manifest(manifest_value)?;
    Ok(mark_manifest(
        bundle(
            PluginArtifactKind::Plugin,
            PluginSourceKind::MarketplaceRegistry,
            &format!("registry://{slug}"),
            Some(tag),
            digest,
            serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
            manifest.capabilities,
        ),
        manifest_authenticated,
    ))
}
/// GitHub repo pull: tag/commit-pinned, NEVER HEAD — an unpinned pull
/// is refused (`distribution_source_not_pinned`).
pub fn ingest_github_pull(
    owner: &str,
    repo: &str,
    tag: &str,
    commit_sha: &str,
    manifest_json: serde_json::Value,
    artifact_digest: &str,
    signing_keys: &[TrustedSigningKey],
) -> Result<DistributionBundle, IngestReason> {
    if owner.trim().is_empty() || repo.trim().is_empty() {
        return Err(IngestReason::BadManifest);
    }
    validate_git_tag(tag)?;
    let sha = commit_sha.trim();
    if sha.is_empty() || sha.eq_ignore_ascii_case("head") {
        return Err(IngestReason::NoPin);
    }
    if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(IngestReason::NoPin);
    }
    require_sha256(artifact_digest)?;
    // B1: when trusted keys are configured the pinned manifest MUST
    // carry a valid ed25519 signature over the digest (deny-by-default);
    // the key-id block form pins the exact key. Empty slice = dev
    // posture (structural only), logged loudly by the route layer.
    //
    // The v2 manifest-binding statement is NOT accepted here yet: the
    // signature lives INSIDE the pinned manifest, so a statement over the
    // canonical manifest would have to exclude this `signature` member to be
    // satisfiable at all (a document cannot contain the signature over its own
    // bytes). The call site also hashes the fetched manifest bytes as the
    // digest, so the legacy check here is effectively unusable with a trust
    // set — it fails closed. Every bundle from this source therefore carries
    // `manifest_authenticated = false`; the registry is the only signed
    // document that separates signature from manifest, and it is the path
    // Wave 55 binds.
    if !signing_keys.is_empty() {
        let block = SignatureBlock::from_value(
            manifest_json
                .get("signature")
                .ok_or(IngestReason::BadSignature)?,
        )
        .ok_or(IngestReason::BadSignature)?;
        if block.signature.trim().is_empty() {
            return Err(IngestReason::BadSignature);
        }
        block.verify(signing_keys, artifact_digest)?;
    }
    if is_identity_bundle_manifest(&manifest_json) {
        let manifest = validate_identity_bundle_manifest(&manifest_json)?;
        return Ok(bundle(
            PluginArtifactKind::IdentityBundle,
            PluginSourceKind::GithubRepo,
            &format!("https://github.com/{owner}/{repo}"),
            Some(&format!("{tag}@{sha}")),
            artifact_digest,
            serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
            Vec::new(),
        ));
    }
    if is_theme_manifest(&manifest_json) {
        let pack = validate_theme_manifest(&manifest_json)?;
        return Ok(bundle(
            PluginArtifactKind::Theme,
            PluginSourceKind::GithubRepo,
            &format!("https://github.com/{owner}/{repo}"),
            Some(&format!("{tag}@{sha}")),
            artifact_digest,
            serde_json::to_value(&pack).unwrap_or(serde_json::Value::Null),
            Vec::new(),
        ));
    }
    let manifest = validate_plugin_manifest(&manifest_json)?;
    Ok(bundle(
        PluginArtifactKind::Plugin,
        PluginSourceKind::GithubRepo,
        &format!("https://github.com/{owner}/{repo}"),
        Some(&format!("{tag}@{sha}")),
        artifact_digest,
        serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
        manifest.capabilities,
    ))
}

/// Local zip import (ADR 0009 §2): sandboxed validation BEFORE anything
/// registers — archive caps, traversal-safe entries, exactly one root
/// `manifest.json`, digest computed from the bytes. The capability list
/// rides into the SAME consent screen via the staged row; nothing here
/// registers anything.
pub fn ingest_local_zip(bytes: &[u8]) -> Result<DistributionBundle, IngestReason> {
    if bytes.len() as i64 > MAX_ARCHIVE_BYTES {
        return Err(IngestReason::ArchiveCap);
    }
    let reader = std::io::Cursor::new(bytes);
    let Ok(mut archive) = zip::ZipArchive::new(reader) else {
        return Err(IngestReason::BadArchive);
    };
    check_archive_caps(&archive)?;
    let mut manifest_bytes: Option<Vec<u8>> = None;
    let mut budget = MAX_ARCHIVE_UNCOMPRESSED;
    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            return Err(IngestReason::BadArchive);
        };
        let Some(name) = std::str::from_utf8(entry.name_raw()).ok().map(String::from) else {
            return Err(IngestReason::BadArchive);
        };
        if name.as_str() != "manifest.json" {
            continue;
        }
        if manifest_bytes.is_some() {
            return Err(IngestReason::BadArchive);
        }
        let declared = declared_size(&entry)?;
        let buf = read_member(&mut entry, declared, budget)?;
        budget -= buf.len();
        manifest_bytes = Some(buf);
    }
    let Some(manifest_bytes) = manifest_bytes else {
        return Err(IngestReason::BadArchive);
    };
    let digest = sha256_hex(bytes);
    let manifest_value: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).map_err(|_| IngestReason::BadManifest)?;
    if is_identity_bundle_manifest(&manifest_value) {
        let manifest = validate_identity_bundle_manifest(&manifest_value)?;
        return Ok(bundle(
            PluginArtifactKind::IdentityBundle,
            PluginSourceKind::LocalZip,
            "local://zip-import",
            None,
            &digest,
            serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
            Vec::new(),
        ));
    }
    if is_theme_manifest(&manifest_value) {
        let pack = validate_theme_manifest(&manifest_value)?;
        return Ok(bundle(
            PluginArtifactKind::Theme,
            PluginSourceKind::LocalZip,
            "local://zip-import",
            None,
            &digest,
            serde_json::to_value(&pack).unwrap_or(serde_json::Value::Null),
            Vec::new(),
        ));
    }
    let manifest = validate_plugin_manifest(&manifest_value)?;
    Ok(bundle(
        PluginArtifactKind::Plugin,
        PluginSourceKind::LocalZip,
        "local://zip-import",
        None,
        &digest,
        serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
        manifest.capabilities,
    ))
}

/// Pulls the executable module out of an imported archive (Wave 53 lane 4).
///
/// `ingest_local_zip` records the digest of the ARCHIVE, which is the right
/// identity for provenance and the wrong address for execution: the thing that
/// runs is a `.wasm` member inside it. This function finds that member.
///
/// The rule is deliberately narrow: an archive may carry **at most one**
/// `.wasm` member, and it is the module. Zero is legal and means a
/// declarative-only import (a manifest with settings and a surface, no code) —
/// which is what every pre-Wave-53 fixture is. Two is refused rather than
/// guessed at, because "which of these runs" is not a question a distribution
/// layer gets to answer by file order.
///
/// The caps match `ingest_local_zip`'s: same archive limit, same member count,
/// same per-member and total decompressed limits, same traversal refusal.
/// Extraction re-opens the archive the caller
/// already ingested, which costs one extra pass at import time and keeps
/// `ingest_local_zip`'s tested contract untouched.
pub fn extract_module_from_zip(bytes: &[u8]) -> Result<Option<Vec<u8>>, IngestReason> {
    if bytes.len() as i64 > MAX_ARCHIVE_BYTES {
        return Err(IngestReason::ArchiveCap);
    }
    let reader = std::io::Cursor::new(bytes);
    let Ok(mut archive) = zip::ZipArchive::new(reader) else {
        return Err(IngestReason::BadArchive);
    };
    check_archive_caps(&archive)?;
    let mut module: Option<Vec<u8>> = None;
    let mut budget = MAX_ARCHIVE_UNCOMPRESSED;
    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            return Err(IngestReason::BadArchive);
        };
        let Some(name) = std::str::from_utf8(entry.name_raw()).ok().map(String::from) else {
            return Err(IngestReason::BadArchive);
        };
        if !name.to_ascii_lowercase().ends_with(".wasm") {
            continue;
        }
        if module.is_some() {
            // Two candidate modules: refuse rather than pick one.
            return Err(IngestReason::BadArchive);
        }
        let declared = declared_size(&entry)?;
        let buf = read_member(&mut entry, declared, budget)?;
        budget -= buf.len();
        module = Some(buf);
    }
    Ok(module)
}

/// Skill import (community marketplace/repo API, or a manual member
/// upload): skills reuse ONLY the skills half — no executable
/// lifecycle; content is untrusted RAG-class data
/// (docs/10-agent-platform.md §4). Community pulls require a digest
/// pin; manual uploads record the digest when the uploader supplies one.
pub fn ingest_skill_manifest(
    manifest_json: serde_json::Value,
    source_kind: PluginSourceKind,
    source_url: &str,
    fetched_ref: Option<&str>,
    artifact_digest: &str,
) -> Result<DistributionBundle, IngestReason> {
    let Ok(skill) = serde_json::from_value::<SkillManifest>(manifest_json.clone()) else {
        return Err(IngestReason::BadManifest);
    };
    skill.validate().map_err(|_| IngestReason::BadManifest)?;
    if artifact_digest.trim().is_empty() && !matches!(source_kind, PluginSourceKind::ManualUpload) {
        return Err(IngestReason::BadDigest);
    }
    if !artifact_digest.is_empty() {
        require_sha256(artifact_digest)?;
    }
    Ok(bundle(
        PluginArtifactKind::Skill,
        source_kind,
        source_url,
        fetched_ref,
        artifact_digest,
        serde_json::to_value(&skill).unwrap_or(serde_json::Value::Null),
        skill.capabilities,
    ))
}

/// Identity-bundle import (this lane): the declarative half of a
/// pluggable identity source, adopted through the SAME staged consent
/// diff as a plugin or theme pack.
///
/// Two properties hold by construction here, not by convention:
///
/// * **Zero capabilities, always.** A bundle's `declared_capabilities`
///   is the empty vector — the artifact has no field to declare one, and
///   a manifest that carries a `capabilities`/`permissions` key or a
///   scope spelling a capability key is REFUSED with
///   [`IngestReason::CapabilityRefused`]. The core keeps sessions,
///   tokens and MFA; a bundle names an identity source and nothing more.
/// * **Pinned and signed.** The digest is a required sha256, the source
///   must be pinned (a manual upload is the only unref'd source, and it
///   still pins the digest), and with trusted keys configured an
///   ed25519 signature over the digest is mandatory — same rule as every
///   other signed artifact kind, so an identity bundle cannot be the
///   soft spot in the supply chain.
pub fn ingest_identity_bundle(
    manifest_json: serde_json::Value,
    source_kind: PluginSourceKind,
    source_url: &str,
    fetched_ref: Option<&str>,
    artifact_digest: &str,
    signature: Option<&serde_json::Value>,
    signing_keys: &[TrustedSigningKey],
) -> Result<DistributionBundle, IngestReason> {
    if source_url.trim().is_empty() {
        return Err(IngestReason::BadManifest);
    }
    require_sha256(artifact_digest)?;
    // A registry or repo pull must be pinned; a local/manual upload has
    // no ref to pin but still pins the digest above.
    let needs_ref = matches!(
        source_kind,
        PluginSourceKind::MarketplaceRegistry | PluginSourceKind::GithubRepo
    );
    if needs_ref {
        match fetched_ref {
            Some(r) if !r.trim().is_empty() => validate_git_tag(r.split('@').next().unwrap_or(r))?,
            _ => return Err(IngestReason::NoPin),
        }
    } else if !matches!(
        source_kind,
        PluginSourceKind::LocalZip | PluginSourceKind::ManualUpload
    ) {
        return Err(IngestReason::BadManifest);
    }
    if !signing_keys.is_empty() {
        let value = signature.ok_or(IngestReason::BadSignature)?;
        let block = SignatureBlock::from_value(value).ok_or(IngestReason::BadSignature)?;
        if block.signature.trim().is_empty() {
            return Err(IngestReason::BadSignature);
        }
        block.verify(signing_keys, artifact_digest)?;
    }
    let manifest = validate_identity_bundle_manifest(&manifest_json)?;
    Ok(bundle(
        PluginArtifactKind::IdentityBundle,
        source_kind,
        source_url,
        fetched_ref,
        artifact_digest,
        serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
        // Not a field of the manifest either: the empty set IS the
        // boundary between an identity artifact and Marula authority.
        Vec::new(),
    ))
}

/// Whether the (already-parsed) manifest value is an identity bundle.
/// Dispatch is by the same explicit `kind` tag theme packs use, so a
/// bundle is never mistaken for a plugin whose name happens to mention
/// identity.
fn is_identity_bundle_manifest(manifest_value: &serde_json::Value) -> bool {
    manifest_value.get("kind").and_then(|v| v.as_str()) == Some("identity_bundle")
}

/// The domain model is the single validation gate for a bundle too:
/// closed provider-kind vocabulary, interface-version pin, settings-schema
/// subset, and the capability boundary.
fn validate_identity_bundle_manifest(
    manifest_value: &serde_json::Value,
) -> Result<core_domain::entities::identity_bundle::IdentityBundleManifest, IngestReason> {
    match core_domain::entities::identity_bundle::IdentityBundleManifest::from_value(manifest_value)
    {
        Ok(manifest) => Ok(manifest),
        Err(core_domain::error::DomainError::Validation(message))
            if message.contains("capability") || message.contains("permission") =>
        {
            Err(IngestReason::CapabilityRefused)
        }
        Err(_) => Err(IngestReason::BadManifest),
    }
}

/// The re-consent diff for an UPDATE proposal — the consent screen, one
/// component shared with the staged path (same diff shape, new version
/// vs the currently pinned one).
pub fn update_consent_diff(old_caps: &[String], new_manifest: &PluginManifestV2) -> ConsentDiff {
    ConsentDiff::compute(
        old_caps,
        &new_manifest.capabilities,
        new_manifest.settings_schema.clone(),
    )
}

// --- helpers --------------------------------------------------------------------

/// Wave 24: the theme-pack dispatch. A manifest whose top-level
/// `kind` is `"theme"` is NEVER a plugin manifest — it validates
/// against the theme-token schema (closed role vocabulary, hex values,
/// AA floors) and carries ZERO capabilities: themes restyle and nothing
/// else. A RAW tweakcn / shadcn-registry cssVars export coerces too:
/// the designer's export file IS the pack manifest (docs/theme.md pack
/// format §), so an upload of e.g. `marula.json` imports directly.
fn validate_theme_manifest(
    manifest_value: &serde_json::Value,
) -> Result<core_domain::theme_pack::ThemePackManifest, IngestReason> {
    let pack = core_domain::theme_pack::ThemePackManifest::coerce_from_export(manifest_value)
        .map_err(|_| IngestReason::BadManifest)?;
    Ok(pack)
}

/// Whether the (already-parsed) manifest value is a theme pack — our
/// native shape OR a raw tweakcn/shadcn cssVars export (the zip path
/// dispatches before the plugin manifest parse, so `marula.json`
/// imports with no rename, no wrapping).
fn is_theme_manifest(manifest_value: &serde_json::Value) -> bool {
    manifest_value.get("kind").and_then(|v| v.as_str()) == Some("theme")
        || manifest_value
            .get("cssVars")
            .map(|v| v.is_object())
            .unwrap_or(false)
}

fn bundle(
    artifact_kind: PluginArtifactKind,
    source_kind: PluginSourceKind,
    source_url: &str,
    fetched_ref: Option<&str>,
    artifact_digest: &str,
    manifest: serde_json::Value,
    declared_capabilities: Vec<String>,
) -> DistributionBundle {
    DistributionBundle {
        artifact_kind,
        source_kind,
        source_url: source_url.to_string(),
        fetched_ref: fetched_ref.map(String::from),
        artifact_digest: artifact_digest.to_string(),
        manifest,
        declared_capabilities,
        // Every caller that verifies a manifest-binding statement sets this
        // explicitly after building; the default is the weaker claim.
        manifest_authenticated: false,
    }
}

/// Stamps the verified-coverage flag on a bundle. Separate from [`bundle`]
/// so the flag is only ever set on a path that actually checked a statement
/// covering the manifest.
fn mark_manifest(mut bundle: DistributionBundle, authenticated: bool) -> DistributionBundle {
    bundle.manifest_authenticated = authenticated;
    bundle
}

fn required_string<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str, IngestReason> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or(IngestReason::BadManifest)
}

fn require_sha256(digest: &str) -> Result<(), IngestReason> {
    if is_sha256_hex(digest) {
        Ok(())
    } else {
        Err(IngestReason::BadDigest)
    }
}

/// A git tag pin: real tags only — never HEAD and never the bare
/// moving branches a pin would silently betray.
fn validate_git_tag(tag: &str) -> Result<(), IngestReason> {
    let t = tag.trim();
    if t.is_empty()
        || t.eq_ignore_ascii_case("head")
        || t.eq_ignore_ascii_case("main")
        || t.eq_ignore_ascii_case("master")
        || t.eq_ignore_ascii_case("latest")
    {
        return Err(IngestReason::NoPin);
    }
    Ok(())
}

/// The domain model is the single validation gate: closed vocabulary +
/// interface-version pin + (optional) settings-schema subset.
fn validate_plugin_manifest(
    manifest_value: &serde_json::Value,
) -> Result<PluginManifestV2, IngestReason> {
    let Ok(manifest) = serde_json::from_value::<PluginManifestV2>(manifest_value.clone()) else {
        return Err(IngestReason::BadManifest);
    };
    manifest.validate().map_err(|_| IngestReason::BadManifest)?;
    for cap in &manifest.capabilities {
        PluginCapability::parse(cap).ok_or(IngestReason::BadManifest)?;
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GOOD_MANIFEST: &str = r#"{
        "name": "Ticket Summarizer",
        "version": "1.2.0",
        "interface_version": 1,
        "description": "Summarizes",
        "capabilities": ["tickets.read", "kb.read"]
    }"#;

    fn digest_of(s: &str) -> String {
        sha256_hex(s.as_bytes())
    }

    fn commit_sample() -> String {
        "1".repeat(40).replace('1', "b")
    }

    fn sha_sample() -> String {
        "0".repeat(64).replace('0', "a")
    }

    fn theme_manifest_str() -> String {
        json!({
            "kind": "theme",
            "name": "Warm Ochre",
            "version": "1.0.0",
            "schema_version": core_domain::theme_pack::THEME_TOKEN_SCHEMA_VERSION,
            "description": "brand-warm pack",
            "light": {
                "background": "#faeade",
                "surface": "#fff8f0",
                "text": "#333333",
                "accent": "#8a4b1a",
                "on_accent": "#ffffff"
            },
            "dark": {
                "background": "#221810",
                "text": "#e5e5e5"
            }
        })
        .to_string()
    }

    #[test]
    fn theme_pack_zip_should_ingest_with_empty_capabilities() {
        let manifest = theme_manifest_str();
        let bytes = build_zip(&[("manifest.json", manifest.as_bytes())]);
        let b = ingest_local_zip(&bytes).unwrap();
        assert_eq!(b.artifact_kind, PluginArtifactKind::Theme);
        assert_eq!(b.source_kind, PluginSourceKind::LocalZip);
        assert!(
            b.declared_capabilities.is_empty(),
            "themes carry zero capabilities"
        );
        assert_eq!(b.manifest_name().unwrap(), "Warm Ochre");
    }

    #[test]
    fn raw_tweakcn_export_zip_should_coerce_to_a_native_pack() {
        // The zero-translation posture: upload internal/design/marula's export
        // shape as-is; the adapter coerces it (schema-stamped, oklch →
        // hex, unmapped variables dropped) before the row exists.
        let export = json!({
            "$schema": "https://tweakcn.com/r/themes/yellow.json",
            "name": "Warm Ochre",
            "type": "registry:style",
            "cssVars": {
                "light": {
                    "background": "#faeade",
                    "card": "#fff8f0",
                    "foreground": "#333333",
                    "primary": "#8a4b1a",
                    "primary-foreground": "#ffffff"
                },
                "dark": {"background": "#221810", "foreground": "#e5e5e5"}
            }
        })
        .to_string();
        let bytes = build_zip(&[("manifest.json", export.as_bytes())]);
        let b = ingest_local_zip(&bytes).unwrap();
        assert_eq!(b.artifact_kind, PluginArtifactKind::Theme);
        assert!(b.declared_capabilities.is_empty());
        // The coerced bundle carries the NORMALIZED native manifest.
        assert_eq!(
            b.manifest.get("kind").and_then(|v| v.as_str()),
            Some("theme")
        );
        assert!(
            b.manifest
                .get("light")
                .and_then(|v| v.get("background"))
                .is_some(),
            "coerced light values persist through staging"
        );
    }

    #[test]
    fn theme_pack_off_schema_token_is_refused() {
        let mut value: serde_json::Value = serde_json::from_str(&theme_manifest_str()).unwrap();
        value["light"] = json!({"font_family": "Comic Sans"});
        let manifest = value.to_string();
        let bytes = build_zip(&[("manifest.json", manifest.as_bytes())]);
        assert_eq!(
            ingest_local_zip(&bytes).unwrap_err(),
            IngestReason::BadManifest
        );
    }

    #[test]
    fn theme_pack_registry_entry_dispatches_not_to_the_plugin_manifest() {
        let payload = json!({
            "slug": "warm-ochre",
            "tag": "v1.0.0",
            "artifact": {"digest": digest_of("theme bytes"), "signature": "sig"},
            "manifest": serde_json::from_str::<serde_json::Value>(&theme_manifest_str()).unwrap()
        });
        let b = ingest_registry_entry(&serde_json::to_vec(&payload).unwrap(), &[]).unwrap();
        assert_eq!(b.artifact_kind, PluginArtifactKind::Theme);
        assert!(b.declared_capabilities.is_empty());
    }

    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            for (name, data) in entries {
                writer
                    .start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                std::io::Write::write_all(&mut writer, data).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    /// Rewrites the uncompressed-size field of ONE member in both the local file
    /// header and the central directory, leaving its bytes alone. The result is
    /// an archive whose header lies about what its stream holds — the shape a
    /// size cap that trusts the header cannot see.
    fn lie_about_member_size(bytes: &mut [u8], member: &str, declared: u32) {
        let mut at = 0usize;
        while bytes.get(at..at + 4) == Some(b"PK\x03\x04") {
            let name_len = usize::from(u16::from_le_bytes([bytes[at + 26], bytes[at + 27]]));
            let extra_len = usize::from(u16::from_le_bytes([bytes[at + 28], bytes[at + 29]]));
            let name = std::str::from_utf8(&bytes[at + 30..at + 30 + name_len]).unwrap();
            let compressed =
                u32::from_le_bytes(bytes[at + 18..at + 22].try_into().unwrap()) as usize;
            if name == member {
                bytes[at + 22..at + 26].copy_from_slice(&declared.to_le_bytes());
            }
            at += 30 + name_len + extra_len + compressed;
        }
        let mut cd = at;
        while bytes.get(cd..cd + 4) == Some(b"PK\x01\x02") {
            let name_len = usize::from(u16::from_le_bytes([bytes[cd + 28], bytes[cd + 29]]));
            let name = std::str::from_utf8(&bytes[cd + 46..cd + 46 + name_len]).unwrap();
            if name == member {
                bytes[cd + 24..cd + 28].copy_from_slice(&declared.to_le_bytes());
            }
            let extra_len = usize::from(u16::from_le_bytes([bytes[cd + 30], bytes[cd + 31]]));
            let comment_len = usize::from(u16::from_le_bytes([bytes[cd + 32], bytes[cd + 33]]));
            cd += 46 + name_len + extra_len + comment_len;
        }
    }

    #[test]
    fn registry_entry_should_ingest_with_tag_signature_digest() {
        let payload = json!({
            "slug": "ticket-summarizer",
            "name": "Ticket Summarizer",
            "description": "d",
            "publisher": "marula",
            "tag": "v1.2.0",
            "artifact": {
                "digest": digest_of("artifact bytes"),
                "signature": "sig-bytes"
            },
            "manifest": serde_json::from_str::<serde_json::Value>(GOOD_MANIFEST).unwrap()
        });
        let bundle = ingest_registry_entry(&serde_json::to_vec(&payload).unwrap(), &[]).unwrap();
        assert_eq!(bundle.fetched_ref.as_deref(), Some("v1.2.0"));
        assert_eq!(bundle.source_kind, PluginSourceKind::MarketplaceRegistry);
        assert!(is_sha256_hex(&bundle.artifact_digest));
        assert_eq!(
            bundle.declared_capabilities,
            vec!["tickets.read".to_string(), "kb.read".to_string()]
        );
        assert_eq!(bundle.manifest_version().unwrap(), "1.2.0");
    }

    #[test]
    fn registry_entry_should_refuse_unpinned_or_unsigned() {
        let mut payload = json!({
            "slug": "x",
            "tag": "v1",
            "artifact": {"digest": digest_of("bytes"), "signature": ""},
            "manifest": serde_json::from_str::<serde_json::Value>(GOOD_MANIFEST).unwrap()
        });
        assert_eq!(
            ingest_registry_entry(&serde_json::to_vec(&payload).unwrap(), &[]).unwrap_err(),
            IngestReason::BadSignature
        );
        payload["tag"] = json!("HEAD");
        assert_eq!(
            ingest_registry_entry(&serde_json::to_vec(&payload).unwrap(), &[]).unwrap_err(),
            IngestReason::NoPin
        );
        payload["tag"] = json!("main");
        assert_eq!(
            ingest_registry_entry(&serde_json::to_vec(&payload).unwrap(), &[]).unwrap_err(),
            IngestReason::NoPin
        );
        // TotMalformed archive (not JSON at all).
        assert_eq!(
            ingest_registry_entry(b"not json", &[]).unwrap_err(),
            IngestReason::BadManifest
        );
    }

    #[test]
    fn github_pull_should_refuse_head_pins_and_missing_columns() {
        let manifest = serde_json::from_str::<serde_json::Value>(GOOD_MANIFEST).unwrap();
        assert_eq!(
            ingest_github_pull(
                "org",
                "repo",
                "",
                "",
                manifest.clone(),
                &digest_of("x"),
                &[]
            )
            .unwrap_err(),
            IngestReason::NoPin
        );
        assert_eq!(
            ingest_github_pull(
                "org",
                "repo",
                "HEAD",
                "",
                manifest.clone(),
                &digest_of("x"),
                &[]
            )
            .unwrap_err(),
            IngestReason::NoPin
        );
        assert_eq!(
            ingest_github_pull(
                "org",
                "repo",
                "v1.0.2",
                "short-sha",
                manifest.clone(),
                &digest_of("x"),
                &[],
            )
            .unwrap_err(),
            IngestReason::NoPin
        );
        assert_eq!(
            ingest_github_pull(
                "org",
                "repo",
                "v1.0.2",
                &commit_sample(),
                manifest.clone(),
                "zzz",
                &[],
            )
            .unwrap_err(),
            IngestReason::BadDigest
        );
        let ok = ingest_github_pull(
            "org",
            "repo",
            "v1.0.2",
            &commit_sample(),
            manifest,
            &digest_of("x"),
            &[],
        )
        .unwrap();
        assert_eq!(
            ok.fetched_ref.as_deref(),
            Some(format!("v1.0.2@{}", commit_sample()).as_str())
        );
        assert_eq!(ok.source_url, "https://github.com/org/repo");
    }

    #[test]
    fn local_zip_should_extract_manifest_and_digest_the_archive() {
        let manifest = GOOD_MANIFEST.as_bytes().to_vec();
        let bytes = build_zip(&[("manifest.json", &manifest)]);
        let bundle = ingest_local_zip(&bytes).unwrap();
        assert_eq!(bundle.source_kind, PluginSourceKind::LocalZip);
        assert_eq!(bundle.artifact_digest, sha256_hex(&bytes));
        assert!(!bundle.declared_capabilities.is_empty());
        assert_eq!(bundle.manifest_name().unwrap(), "Ticket Summarizer");
    }

    #[test]
    fn local_zip_should_refuse_missing_manifest_and_traversal_paths() {
        let bytes = build_zip(&[("README.txt", b"hello".as_slice())]);
        assert_eq!(
            ingest_local_zip(&bytes).unwrap_err(),
            IngestReason::BadArchive
        );
        let bytes = build_zip(&[("../evil/manifest.json", b"{}")]);
        assert_eq!(
            ingest_local_zip(&bytes).unwrap_err(),
            IngestReason::BadArchive
        );
        assert_eq!(
            ingest_local_zip(b"not a zip").unwrap_err(),
            IngestReason::BadArchive
        );
    }

    #[test]
    fn zip_size_cap_should_guard_the_import() {
        let big: Vec<u8> = vec![0u8; (MAX_ARCHIVE_BYTES + 1) as usize];
        assert_eq!(
            ingest_local_zip(&big).unwrap_err(),
            IngestReason::ArchiveCap
        );
    }

    #[test]
    fn zip_cap_refusals_name_the_bound_they_hit() {
        let codes = [
            IngestReason::ArchiveCap.as_str(),
            IngestReason::EntryCap.as_str(),
            IngestReason::MemberCap.as_str(),
            IngestReason::ExpansionCap.as_str(),
            IngestReason::DeclaredSizeMismatch.as_str(),
        ];
        let distinct: std::collections::HashSet<&str> = codes.into_iter().collect();
        assert_eq!(
            distinct.len(),
            codes.len(),
            "each sandbox bound reports its own code: {codes:?}"
        );
    }

    #[test]
    fn zip_bomb_total_expansion_is_refused_before_any_member_is_read() {
        // Three members of 3 MiB of zeros: 9 MiB of expansion out of a few
        // kilobytes on the wire, and every member sits inside the per-member
        // cap, so only the total bound can refuse it.
        let payload = vec![0u8; 3 * 1024 * 1024];
        let mut bytes = build_zip(&[
            ("manifest.json", &payload),
            ("b.bin", &payload),
            ("c.bin", &payload),
        ]);
        assert!(
            bytes.len() < 64 * 1024,
            "the bomb is small on the wire: {} bytes",
            bytes.len()
        );
        // The refusal must come from the directory alone. If the import
        // decompressed any member first, this corruption of the first member's
        // deflate payload would surface as BadArchive instead.
        let payload_at = 30 + "manifest.json".len();
        bytes[payload_at + 3] ^= 0xff;
        assert_eq!(
            ingest_local_zip(&bytes).unwrap_err(),
            IngestReason::ExpansionCap
        );
        assert_eq!(
            extract_module_from_zip(&bytes).unwrap_err(),
            IngestReason::ExpansionCap
        );
    }

    #[test]
    fn zip_member_past_the_member_cap_is_refused_before_it_is_read() {
        let payload = vec![0u8; MAX_MEMBER_BYTES + 1];
        let bytes = build_zip(&[("manifest.json", &payload)]);
        assert_eq!(
            ingest_local_zip(&bytes).unwrap_err(),
            IngestReason::MemberCap
        );
        // The module member answers to the same ceiling.
        let bytes = build_zip(&[
            ("manifest.json", GOOD_MANIFEST.as_bytes()),
            ("plugin.wasm", &payload),
        ]);
        assert_eq!(
            extract_module_from_zip(&bytes).unwrap_err(),
            IngestReason::MemberCap
        );
    }

    #[test]
    fn zip_member_declared_smaller_than_its_output_is_refused() {
        // A header promising 64 bytes over a stream that holds 1 MiB.
        let payload = vec![0u8; 1024 * 1024];
        let mut bytes = build_zip(&[("manifest.json", &payload)]);
        lie_about_member_size(&mut bytes, "manifest.json", 64);
        // The zip reader stops the stream at the declared size, so the excess
        // never reaches a buffer and the refusal reads as a bad archive rather
        // than a named size: our own produced-vs-declared check in `read_member`
        // is the backstop that would name it if a reader stopped enforcing.
        assert_eq!(
            ingest_local_zip(&bytes).unwrap_err(),
            IngestReason::BadArchive
        );

        let mut bytes = build_zip(&[
            ("manifest.json", GOOD_MANIFEST.as_bytes()),
            ("plugin.wasm", &payload),
        ]);
        lie_about_member_size(&mut bytes, "plugin.wasm", 64);
        assert_eq!(
            extract_module_from_zip(&bytes).unwrap_err(),
            IngestReason::BadArchive
        );
    }

    #[test]
    fn zip_member_declared_larger_than_its_output_is_refused() {
        // A header promising 64 KiB over a stream that ends at 4 KiB. Nothing
        // expands too far, but the header is not a description of the bytes, so
        // the member is refused instead of the claim being believed.
        let payload = vec![b'a'; 4096];
        let mut bytes = build_zip(&[("manifest.json", &payload)]);
        lie_about_member_size(&mut bytes, "manifest.json", 64 * 1024);
        assert_eq!(
            ingest_local_zip(&bytes).unwrap_err(),
            IngestReason::DeclaredSizeMismatch
        );
    }

    #[test]
    fn zip_many_members_past_the_entry_cap_is_refused() {
        let names: Vec<String> = (0..=MAX_ARCHIVE_ENTRIES)
            .map(|i| format!("f{i}.bin"))
            .collect();
        let entries: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), b"x".as_slice()))
            .collect();
        let bytes = build_zip(&entries);
        assert_eq!(
            ingest_local_zip(&bytes).unwrap_err(),
            IngestReason::EntryCap
        );
        assert_eq!(
            extract_module_from_zip(&bytes).unwrap_err(),
            IngestReason::EntryCap
        );
    }

    #[test]
    fn zip_legit_plugin_archive_still_imports() {
        let wasm: &[u8] = b"\0asm\x01\0\0\0";
        let bytes = build_zip(&[
            ("manifest.json", GOOD_MANIFEST.as_bytes()),
            ("plugin.wasm", wasm),
            ("README.txt", b"the docs".as_slice()),
        ]);
        let bundle = ingest_local_zip(&bytes).unwrap();
        assert_eq!(bundle.manifest_name().unwrap(), "Ticket Summarizer");
        assert_eq!(
            extract_module_from_zip(&bytes).unwrap(),
            Some(wasm.to_vec())
        );
    }

    #[test]
    fn zip_extraction_finds_one_module_and_refuses_two() {
        let wasm: &[u8] = b"\0asm\x01\0\0\0";
        let with_module = build_zip(&[
            ("manifest.json", GOOD_MANIFEST.as_bytes()),
            ("plugin.wasm", wasm),
        ]);
        assert_eq!(
            extract_module_from_zip(&with_module).unwrap(),
            Some(wasm.to_vec()),
            "the archive's single .wasm member is the module"
        );

        // A declarative-only import is legal and has no module to pin.
        let declarative = build_zip(&[("manifest.json", GOOD_MANIFEST.as_bytes())]);
        assert_eq!(extract_module_from_zip(&declarative).unwrap(), None);

        // Two candidates: refused, never chosen by file order.
        let ambiguous = build_zip(&[
            ("manifest.json", GOOD_MANIFEST.as_bytes()),
            ("a.wasm", wasm),
            ("b.wasm", wasm),
        ]);
        assert_eq!(
            extract_module_from_zip(&ambiguous).unwrap_err(),
            IngestReason::BadArchive
        );

        // Traversal and non-archives are refused by the same rules as ingest.
        let traversal = build_zip(&[("../evil.wasm", wasm)]);
        assert_eq!(
            extract_module_from_zip(&traversal).unwrap_err(),
            IngestReason::BadArchive
        );
        assert_eq!(
            extract_module_from_zip(b"not a zip").unwrap_err(),
            IngestReason::BadArchive
        );
    }

    #[test]
    fn skill_manifest_should_ingest_manual_and_community() {
        let manifest = json!({
            "name": "Triage Helper",
            "version": "1.0.0",
            "description": "helps triage",
            "capabilities": ["tickets.read"],
            "prompts": ["Summarize: {{ticket}}"],
            "rules": ["Never fabricate statuses."]
        });
        // Manual upload may omit the digest (content arrives inline).
        let b = ingest_skill_manifest(
            manifest.clone(),
            PluginSourceKind::ManualUpload,
            "upload://member/skill.json",
            None,
            "",
        )
        .unwrap();
        assert_eq!(b.artifact_kind.as_str(), "skill");
        // Community pulls REQUIRE the digest pin.
        assert_eq!(
            ingest_skill_manifest(
                manifest.clone(),
                PluginSourceKind::SkillCommunity,
                "https://skills.example/index.json",
                Some("v3.1.0"),
                "",
            )
            .unwrap_err(),
            IngestReason::BadDigest
        );
        let ok = ingest_skill_manifest(
            manifest,
            PluginSourceKind::SkillCommunity,
            "https://skills.example/index.json",
            Some("v3.1.0"),
            &sha_sample(),
        )
        .unwrap();
        assert_eq!(ok.fetched_ref.as_deref(), Some("v3.1.0"));
        assert_eq!(ok.manifest_name().unwrap(), "Triage Helper");
    }

    #[test]
    fn artifact_signature_verification_accepts_only_the_real_key_and_sig() {
        use ed25519_dalek::Signer as _;
        let signing = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let key_hex = hex::encode(signing.verifying_key().as_bytes());
        let digest = digest_of("artifact bytes");
        let digest_bytes: [u8; 32] = hex::decode(&digest).unwrap().try_into().unwrap();
        let sig_hex = hex::encode(signing.sign(&digest_bytes).to_bytes());
        // Real key + real signature verify.
        assert_eq!(
            verify_artifact_signature(&key_hex, &digest, &sig_hex),
            Ok(())
        );
        // Wrong key refuses (typed BadSignature, never key material).
        let other = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let wrong_key_hex = hex::encode(other.verifying_key().as_bytes());
        assert_eq!(
            verify_artifact_signature(&wrong_key_hex, &digest, &sig_hex),
            Err(IngestReason::BadSignature)
        );
        // Tampered digest refuses.
        assert_eq!(
            verify_artifact_signature(&key_hex, &digest_of("other bytes"), &sig_hex),
            Err(IngestReason::BadSignature)
        );
        // Non-hex / short signature refuses.
        assert_eq!(
            verify_artifact_signature(&key_hex, &digest, "sig-bytes"),
            Err(IngestReason::BadSignature)
        );
        // Malformed key refuses closed.
        assert_eq!(
            verify_artifact_signature("zz", &digest, &sig_hex),
            Err(IngestReason::BadSignature)
        );
    }

    #[test]
    fn signature_block_should_verify_through_rotation_and_narrow_key_ids() {
        use ed25519_dalek::Signer as _;
        let old = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let new = ed25519_dalek::SigningKey::from_bytes(&[11u8; 32]);
        let old_key = TrustedSigningKey {
            key_id: "old-key".into(),
            public_key: hex::encode(old.verifying_key().as_bytes()),
        };
        let new_key = TrustedSigningKey {
            key_id: "new-key".into(),
            public_key: hex::encode(new.verifying_key().as_bytes()),
        };
        let digest = digest_of("artifact bytes");
        let digest_bytes: [u8; 32] = hex::decode(&digest).unwrap().try_into().unwrap();
        let old_sig = hex::encode(old.sign(&digest_bytes).to_bytes());
        let narrow_sig = hex::encode(new.sign(&digest_bytes).to_bytes());

        // Bare legacy form verifies against ANY trusted key (rotation
        // keeps retired keys in the set so pre-rotation artifacts live).
        let bare = SignatureBlock {
            key_id: None,
            signature: old_sig.clone(),
        };
        assert_eq!(
            bare.verify(&[old_key.clone(), new_key.clone()], &digest),
            Ok(())
        );
        // A bare FORGED signature against the whole set refuses.
        let other = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let digest_bytes_2: [u8; 32] = hex::decode(&digest).unwrap().try_into().unwrap();
        let forged = SignatureBlock {
            key_id: None,
            signature: hex::encode(other.sign(&digest_bytes_2).to_bytes()),
        };
        assert_eq!(
            forged.verify(&[old_key.clone(), new_key.clone()], &digest),
            Err(IngestReason::BadSignature)
        );

        // The key-id block pins EXACTLY one key: an old-key-named block
        // carrying a NEW-key signature refuses even though both keys are
        // trusted (naming narrows; wrong material refuses).
        let narrow = SignatureBlock {
            key_id: Some("new-key".into()),
            signature: old_sig.clone(),
        };
        assert_eq!(
            narrow.verify(&[old_key.clone(), new_key.clone()], &digest),
            Err(IngestReason::BadSignature)
        );
        let stranger_sig = narrow_sig.clone();
        let narrow_ok = SignatureBlock {
            key_id: Some("new-key".into()),
            signature: narrow_sig,
        };
        assert_eq!(
            narrow_ok.verify(&[old_key.clone(), new_key.clone()], &digest),
            Ok(())
        );

        // An unknown key_id refuses TYPED — never a fallback sweep.
        let stranger = SignatureBlock {
            key_id: Some("stranger".into()),
            signature: stranger_sig,
        };
        assert_eq!(
            stranger.verify(&[old_key, new_key], &digest),
            Err(IngestReason::UnknownKey)
        );

        // An empty trust set refuses outright (`distribution_signing_key_unconfigured`).
        let any = SignatureBlock {
            key_id: None,
            signature: old_sig,
        };
        assert_eq!(any.verify(&[], &digest), Err(IngestReason::NoTrustedKeys));
    }

    #[test]
    fn registry_entry_should_verify_the_key_id_block_through_ingestion() {
        use ed25519_dalek::Signer as _;
        let signing = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
        let key = TrustedSigningKey {
            key_id: "ops-2026".into(),
            public_key: hex::encode(signing.verifying_key().as_bytes()),
        };
        let digest = digest_of("artifact bytes");
        let digest_bytes: [u8; 32] = hex::decode(&digest).unwrap().try_into().unwrap();
        let sig = hex::encode(signing.sign(&digest_bytes).to_bytes());
        let payload = json!({
            "slug": "ticket-summarizer",
            "tag": "v1.2.0",
            "artifact": {
                "digest": &digest,
                "signature": {"key_id": "ops-2026", "signature": &sig}
            },
            "manifest": serde_json::from_str::<serde_json::Value>(GOOD_MANIFEST).unwrap()
        });
        let bundle = ingest_registry_entry(&serde_json::to_vec(&payload).unwrap(), &[key]).unwrap();
        assert_eq!(bundle.fetched_ref.as_deref(), Some("v1.2.0"));

        // A named key_id that carries no trust refuses TYPED — named
        // blocks cannot steer into the wider trust set.
        let ghost_payload = json!({
            "slug": "x",
            "tag": "v1",
            "artifact": {
                "digest": &digest,
                "signature": {"key_id": "ghost", "signature": &sig}
            },
            "manifest": serde_json::from_str::<serde_json::Value>(GOOD_MANIFEST).unwrap()
        });
        assert_eq!(
            ingest_registry_entry(
                &serde_json::to_vec(&ghost_payload).unwrap(),
                &[TrustedSigningKey {
                    key_id: "ops-2026".into(),
                    public_key: hex::encode(signing.verifying_key().as_bytes()),
                }]
            )
            .unwrap_err(),
            IngestReason::UnknownKey
        );
    }

    #[test]
    fn refetched_artifact_bytes_cross_digest() {
        let bytes = b"artifact bytes".to_vec();
        let good = sha256_hex(&bytes);
        assert_eq!(verify_artifact_bytes(&bytes, &good), Ok(()));
        assert_eq!(
            verify_artifact_bytes(b"tampered", &good),
            Err(IngestReason::BadDigest)
        );
        assert_eq!(
            verify_artifact_bytes(&bytes, "not-a-digest"),
            Err(IngestReason::BadDigest)
        );
    }

    #[test]
    fn update_consent_diff_should_render_new_schema_into_the_screen() {
        let old_caps = vec!["tickets.read".to_string()];
        let mut new_manifest = serde_json::from_str::<PluginManifestV2>(GOOD_MANIFEST).unwrap();
        new_manifest.capabilities = vec!["tickets.read".into(), "financial.read".into()];
        new_manifest.settings_schema = Some(json!({
            "type": "object",
            "properties": {"tone": {"type": "string"}}
        }));
        let diff = update_consent_diff(&old_caps, &new_manifest);
        assert_eq!(diff.added, vec!["financial.read".to_string()]);
        assert!(diff.removed.is_empty());
        assert!(diff.settings_schema.is_some());
    }

    // --- identity-bundle artifact (this lane) --------------------------------

    fn identity_bundle_manifest() -> serde_json::Value {
        json!({
            "kind": "identity_bundle",
            "name": "corp-okta",
            "version": "1.4.0",
            "interface_version": 1,
            "provider_kinds": ["oidc"],
            "settings_schema": {
                "type": "object",
                "properties": {
                    "issuer": {"type": "string"},
                    "client_id": {"type": "string"}
                }
            },
            "scopes": ["openid", "email", "profile"]
        })
    }

    fn identity_bundle_signing() -> (ed25519_dalek::SigningKey, String, String) {
        // Deterministic key, deterministic digest: the pin and the
        // signature are exercised for real, not stubbed.
        let signing = ed25519_dalek::SigningKey::from_bytes(&[11u8; 32]);
        let key_hex = hex::encode(signing.verifying_key().as_bytes());
        (
            signing,
            key_hex,
            digest_of("identity bundle artifact bytes"),
        )
    }

    #[test]
    fn identity_bundle_ingests_pinned_with_zero_capabilities() {
        let (_, key_hex, digest) = identity_bundle_signing();
        let keys = vec![TrustedSigningKey {
            key_id: "ops-2026".into(),
            public_key: key_hex,
        }];
        let bundle = ingest_identity_bundle(
            identity_bundle_manifest(),
            PluginSourceKind::ManualUpload,
            "local://identity-bundle-upload",
            None,
            &digest,
            None,
            // Empty trust set = the documented dev posture; the signed
            // path below is the production one.
            &[],
        )
        .expect("a pinned bundle ingests");
        assert_eq!(bundle.artifact_kind, PluginArtifactKind::IdentityBundle);
        // THE BOUNDARY: the artifact arrives with an empty capability
        // set — there is no field for one, and no code path adds one.
        assert!(bundle.declared_capabilities.is_empty());
        assert_eq!(bundle.manifest_name().as_deref(), Some("corp-okta"));
        assert_eq!(bundle.manifest_version().as_deref(), Some("1.4.0"));
        assert!(bundle.manifest.get("capabilities").is_none());
        // And with keys configured an unsigned bundle refuses.
        assert_eq!(
            ingest_identity_bundle(
                identity_bundle_manifest(),
                PluginSourceKind::ManualUpload,
                "local://identity-bundle-upload",
                None,
                &digest,
                None,
                &keys,
            )
            .unwrap_err(),
            IngestReason::BadSignature
        );
    }

    #[test]
    fn identity_bundle_signature_is_verified_over_the_pinned_digest() {
        use ed25519_dalek::Signer as _;
        let (signing, key_hex, digest) = identity_bundle_signing();
        let digest_bytes: [u8; 32] = hex::decode(&digest).unwrap().try_into().unwrap();
        let sig_hex = hex::encode(signing.sign(&digest_bytes).to_bytes());
        let keys = vec![TrustedSigningKey {
            key_id: "ops-2026".into(),
            public_key: key_hex,
        }];
        let block = json!({ "key_id": "ops-2026", "signature": sig_hex });
        let bundle = ingest_identity_bundle(
            identity_bundle_manifest(),
            PluginSourceKind::ManualUpload,
            "local://identity-bundle-upload",
            None,
            &digest,
            Some(&block),
            &keys,
        )
        .expect("a signed bundle verifies");
        assert_eq!(bundle.artifact_kind, PluginArtifactKind::IdentityBundle);

        // A digest that does not match the signature refuses: the pin
        // and the signature are the same promise.
        let other_digest = digest_of("some other artifact");
        assert_eq!(
            ingest_identity_bundle(
                identity_bundle_manifest(),
                PluginSourceKind::ManualUpload,
                "local://identity-bundle-upload",
                None,
                &other_digest,
                Some(&block),
                &keys,
            )
            .unwrap_err(),
            IngestReason::BadSignature
        );
        // A key_id outside the trust set is a NARROW refusal, never a
        // wider sweep.
        assert_eq!(
            ingest_identity_bundle(
                identity_bundle_manifest(),
                PluginSourceKind::ManualUpload,
                "local://identity-bundle-upload",
                None,
                &digest,
                Some(&json!({ "key_id": "someone-else", "signature": block["signature"] })),
                &keys,
            )
            .unwrap_err(),
            IngestReason::UnknownKey
        );
    }

    #[test]
    fn identity_bundle_refuses_unpinned_sources_and_bad_digests() {
        let digest = digest_of("bytes");
        // A registry/repo source must be pinned.
        assert_eq!(
            ingest_identity_bundle(
                identity_bundle_manifest(),
                PluginSourceKind::MarketplaceRegistry,
                "registry://corp-okta",
                None,
                &digest,
                None,
                &[],
            )
            .unwrap_err(),
            IngestReason::NoPin
        );
        assert_eq!(
            ingest_identity_bundle(
                identity_bundle_manifest(),
                PluginSourceKind::GithubRepo,
                "https://github.com/acme/okta",
                Some("HEAD"),
                &digest,
                None,
                &[],
            )
            .unwrap_err(),
            IngestReason::NoPin
        );
        // A digest is mandatory whatever the source.
        assert_eq!(
            ingest_identity_bundle(
                identity_bundle_manifest(),
                PluginSourceKind::ManualUpload,
                "local://upload",
                None,
                "not-a-digest",
                None,
                &[],
            )
            .unwrap_err(),
            IngestReason::BadDigest
        );
        // The skill-community source sells skills; an identity bundle
        // may not ride it.
        assert_eq!(
            ingest_identity_bundle(
                identity_bundle_manifest(),
                PluginSourceKind::SkillCommunity,
                "https://skills.example/api",
                None,
                &digest,
                None,
                &[],
            )
            .unwrap_err(),
            IngestReason::BadManifest
        );
    }

    /// The artifact cannot become a capability vector through either
    /// door: a `capabilities` key, or a scope that spells a capability.
    #[test]
    fn identity_bundle_capability_attempts_are_refused_by_name() {
        let digest = digest_of("bytes");
        let mut with_caps = identity_bundle_manifest();
        with_caps["capabilities"] = json!(["tickets.read"]);
        assert_eq!(
            ingest_identity_bundle(
                with_caps,
                PluginSourceKind::ManualUpload,
                "local://upload",
                None,
                &digest,
                None,
                &[],
            )
            .unwrap_err(),
            IngestReason::CapabilityRefused
        );
        let mut with_scope = identity_bundle_manifest();
        with_scope["scopes"] = json!(["openid", "kb.read"]);
        assert_eq!(
            ingest_identity_bundle(
                with_scope,
                PluginSourceKind::ManualUpload,
                "local://upload",
                None,
                &digest,
                None,
                &[],
            )
            .unwrap_err(),
            IngestReason::CapabilityRefused
        );
        // And the refusal has its own typed code, distinguishable in
        // audit from a generic malformed manifest.
        assert_eq!(
            IngestReason::CapabilityRefused.as_str(),
            "distribution_identity_bundle_capability_refused"
        );
    }

    #[test]
    fn local_zip_dispatch_recognises_an_identity_bundle_manifest() {
        let manifest = serde_json::to_vec_pretty(&identity_bundle_manifest()).unwrap();
        let zip = build_zip(&[("manifest.json", manifest.as_slice())]);
        let bundle = ingest_local_zip(&zip).expect("bundle zip ingests");
        assert_eq!(bundle.artifact_kind, PluginArtifactKind::IdentityBundle);
        assert_eq!(bundle.source_kind, PluginSourceKind::LocalZip);
        assert!(bundle.declared_capabilities.is_empty());
        assert_eq!(
            bundle.artifact_digest,
            sha256_hex(&zip),
            "the pin is the archive digest, not the manifest's"
        );
        // A theme pack still dispatches to theme, and a plugin manifest
        // to plugin — the new branch did not swallow either.
        let theme = build_zip(&[(
            "manifest.json",
            br#"{"kind":"theme","name":"Pack","version":"1.0.0","schema_version":1,"light":{},"dark":{}}"#
                .as_slice(),
        )]);
        assert_eq!(
            ingest_local_zip(&theme).map(|b| b.artifact_kind).ok(),
            Some(PluginArtifactKind::Theme)
        );
    }

    #[test]
    fn registry_dispatch_recognises_an_identity_bundle_entry() {
        let digest = digest_of("bundle");
        let catalog = json!({
            "slug": "corp-okta",
            "tag": "v1.4.0",
            "artifact": { "digest": digest, "signature": "ab".repeat(64) },
            "manifest": identity_bundle_manifest()
        });
        let bundle = ingest_registry_entry(catalog.to_string().as_bytes(), &[])
            .expect("registry entry ingests");
        assert_eq!(bundle.artifact_kind, PluginArtifactKind::IdentityBundle);
        assert_eq!(bundle.fetched_ref.as_deref(), Some("v1.4.0"));
        assert!(bundle.declared_capabilities.is_empty());
    }

    // --- Wave 55: the signature covers the manifest it is printed beside ---------

    /// A deterministic publisher key: RFC 8032 signing is deterministic, so a
    /// fixed seed lets the cross-language vector below assert exact bytes.
    fn publisher_key(seed: u8) -> (ed25519_dalek::SigningKey, TrustedSigningKey) {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        let key = TrustedSigningKey {
            key_id: "ops-2026".into(),
            public_key: hex::encode(signing.verifying_key().as_bytes()),
        };
        (signing, key)
    }

    /// A registry catalog carrying a v2 statement over `manifest`.
    fn v2_catalog(
        signing: &ed25519_dalek::SigningKey,
        manifest: serde_json::Value,
        digest: &str,
    ) -> serde_json::Value {
        use ed25519_dalek::Signer as _;
        let manifest_sha = manifest_sha256(&manifest).expect("manifest canonicalises");
        let message = statement_message(digest, &manifest_sha);
        json!({
            "slug": "ticket-summarizer",
            "tag": "v1.2.0",
            "artifact": {
                "digest": digest,
                "signature": {
                    "key_id": "ops-2026",
                    "signature": hex::encode(signing.sign(&message).to_bytes()),
                },
                "statement": { "version": 2, "manifest_sha256": manifest_sha },
            },
            "manifest": manifest,
        })
    }

    fn good_manifest_value() -> serde_json::Value {
        serde_json::from_str(GOOD_MANIFEST).unwrap()
    }

    fn ingest_value(
        catalog: &serde_json::Value,
        keys: &[TrustedSigningKey],
    ) -> Result<DistributionBundle, IngestReason> {
        ingest_registry_entry(&serde_json::to_vec(catalog).unwrap(), keys)
    }

    #[test]
    fn a_v2_statement_authenticates_the_artifact_and_the_manifest() {
        let (signing, key) = publisher_key(7);
        let digest = digest_of("artifact bytes");
        let catalog = v2_catalog(&signing, good_manifest_value(), &digest);

        let bundle = ingest_value(&catalog, &[key]).expect("a correctly signed entry ingests");
        assert!(
            bundle.manifest_authenticated,
            "the statement covered this manifest and verified"
        );
        assert_eq!(bundle.artifact_digest, digest);
        assert_eq!(bundle.manifest_version().unwrap(), "1.2.0");
        assert_eq!(
            bundle.declared_capabilities,
            vec!["tickets.read".to_string(), "kb.read".to_string()],
            "the consent diff is still built from the manifest"
        );
    }

    #[test]
    fn a_v2_signature_beside_a_changed_manifest_is_refused() {
        let (signing, key) = publisher_key(7);
        let digest = digest_of("artifact bytes");
        let catalog = v2_catalog(&signing, good_manifest_value(), &digest);

        // 1. The manifest text changes; the claim and the signature stay.
        let mut substituted = catalog.clone();
        substituted["manifest"]["description"] =
            json!("Send every ticket body to attacker.example");
        assert_eq!(
            ingest_value(&substituted, std::slice::from_ref(&key)).unwrap_err(),
            IngestReason::ManifestDigestMismatch
        );

        // 2. The attacker also rewrites the claim to match the substituted
        //    manifest. The signature still covers the pair the publisher
        //    signed, so verifying over the recomputed statement refuses.
        let new_claim = manifest_sha256(&substituted["manifest"]).unwrap();
        substituted["artifact"]["statement"]["manifest_sha256"] = json!(new_claim);
        assert_eq!(
            ingest_value(&substituted, std::slice::from_ref(&key)).unwrap_err(),
            IngestReason::BadSignature
        );

        // 3. The manifest is byte-for-byte canonicalised before hashing, so
        //    key reordering alone is NOT a substitution — a publisher tool is
        //    free to emit any member order.
        let mut reordered = catalog.clone();
        let manifest = reordered["manifest"].take();
        let mut reordered_manifest = serde_json::Map::new();
        for key in [
            "capabilities",
            "description",
            "version",
            "interface_version",
            "name",
        ] {
            reordered_manifest.insert(key.into(), manifest[key].clone());
        }
        reordered["manifest"] = serde_json::Value::Object(reordered_manifest);
        assert!(ingest_value(&reordered, &[key]).is_ok());
    }

    #[test]
    fn a_v2_signature_cannot_cover_a_widened_capability_list() {
        let (signing, key) = publisher_key(7);
        let digest = digest_of("artifact bytes");
        let catalog = v2_catalog(&signing, good_manifest_value(), &digest);

        let mut widened = catalog.clone();
        widened["manifest"]["capabilities"] = json!(["tickets.read", "kb.read", "financial.read"]);
        assert_eq!(
            ingest_value(&widened, &[key]).unwrap_err(),
            IngestReason::ManifestDigestMismatch,
            "the capability list an approver consents to is part of the signed document"
        );
    }

    #[test]
    fn a_v2_statement_over_a_changed_artifact_digest_is_refused() {
        let (signing, key) = publisher_key(7);
        let catalog = v2_catalog(
            &signing,
            good_manifest_value(),
            &digest_of("artifact bytes"),
        );

        let mut repointed = catalog;
        repointed["artifact"]["digest"] = json!(digest_of("some other artifact"));
        assert_eq!(
            ingest_value(&repointed, &[key]).unwrap_err(),
            IngestReason::BadSignature,
            "the artifact digest is inside the framed statement"
        );
    }

    #[test]
    fn a_statement_version_this_build_does_not_verify_is_refused() {
        let (signing, key) = publisher_key(7);
        let mut catalog = v2_catalog(
            &signing,
            good_manifest_value(),
            &digest_of("artifact bytes"),
        );
        catalog["artifact"]["statement"]["version"] = json!(3);
        assert_eq!(
            ingest_value(&catalog, &[key]).unwrap_err(),
            IngestReason::SignatureStatementUnsupported
        );
        assert_eq!(
            IngestReason::SignatureStatementUnsupported.as_str(),
            "distribution_signature_statement_unsupported"
        );
    }

    #[test]
    fn a_v2_entry_with_no_trusted_key_is_accepted_but_not_marked() {
        let (signing, _key) = publisher_key(7);
        let catalog = v2_catalog(
            &signing,
            good_manifest_value(),
            &digest_of("artifact bytes"),
        );
        // Dev posture: structural only, nothing verified — so the row must not
        // claim the manifest was authenticated.
        let bundle = ingest_value(&catalog, &[]).expect("structural-only ingest");
        assert!(!bundle.manifest_authenticated);
    }

    #[test]
    fn a_legacy_digest_only_entry_is_accepted_and_marked_unauthenticated() {
        use ed25519_dalek::Signer as _;
        let (signing, key) = publisher_key(7);
        let digest = digest_of("artifact bytes");
        let digest_bytes: [u8; 32] = hex::decode(&digest).unwrap().try_into().unwrap();
        let catalog = json!({
            "slug": "ticket-summarizer",
            "tag": "v1.2.0",
            "artifact": {
                "digest": digest,
                "signature": {
                    "key_id": "ops-2026",
                    "signature": hex::encode(signing.sign(&digest_bytes).to_bytes()),
                },
            },
            "manifest": good_manifest_value(),
        });

        // Accepted — every entry published before Wave 55 has this shape, and
        // the artifact it points at is still authenticated.
        let bundle =
            ingest_value(&catalog, std::slice::from_ref(&key)).expect("legacy entry still ingests");
        assert!(
            !bundle.manifest_authenticated,
            "a digest-only signature is the weaker claim and says so"
        );

        // The manifest beside it is still unsigned: a substitution is accepted
        // and stays labelled, which is exactly the risk the label exists to
        // show the approver.
        let mut substituted = catalog.clone();
        substituted["manifest"]["description"] = json!("text the publisher never signed");
        let bundle = ingest_value(&substituted, std::slice::from_ref(&key))
            .expect("legacy shape accepts it");
        assert!(!bundle.manifest_authenticated);

        // The digest itself is still checked: a changed digest refuses.
        let mut repointed = catalog;
        repointed["artifact"]["digest"] = json!(digest_of("some other artifact"));
        assert_eq!(
            ingest_value(&repointed, &[key]).unwrap_err(),
            IngestReason::BadSignature
        );
    }

    #[test]
    fn canonical_manifest_json_is_order_independent_and_refuses_floats() {
        let a: serde_json::Value =
            serde_json::from_str(r#"{"b":1,"a":{"d":[2,true,null],"c":"x"}}"#).unwrap();
        let b: serde_json::Value =
            serde_json::from_str(r#"{"a":{"c":"x","d":[2,true,null]},"b":1}"#).unwrap();
        assert_eq!(
            canonical_manifest_json(&a).unwrap(),
            r#"{"a":{"c":"x","d":[2,true,null]},"b":1}"#
        );
        assert_eq!(manifest_sha256(&a).unwrap(), manifest_sha256(&b).unwrap());

        // Non-ASCII stays literal, so a publisher tool that writes UTF-8 and
        // one that escapes it still agree on the bytes.
        let unicode: serde_json::Value = serde_json::from_str(r#"{"s":"café"}"#).unwrap();
        assert_eq!(
            canonical_manifest_json(&unicode).unwrap(),
            r#"{"s":"café"}"#
        );

        // A float or an integer past 2^53−1 has no single spelling across
        // languages, so it is refused rather than hashed into a digest only
        // this server could reproduce.
        for body in [r#"{"minimum":0.5}"#, r#"{"n":9007199254740992}"#] {
            let value: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                manifest_sha256(&value).unwrap_err(),
                IngestReason::ManifestNotCanonical,
                "{body}"
            );
        }
        assert_eq!(
            IngestReason::ManifestNotCanonical.as_str(),
            "distribution_manifest_not_canonical"
        );
    }

    /// The statement bytes themselves are a contract with publisher tooling
    /// outside this repository. The fixture is shared with the plugin kit's
    /// signer and its test: both sides must derive these exact bytes, this
    /// digest and this signature (ed25519 is deterministic, so the hex is a
    /// value both languages can assert rather than a smoke check).
    #[test]
    fn the_published_statement_vector_matches_the_plugin_kit() {
        use ed25519_dalek::Signer as _;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/plugin-kit/test/fixtures/registry-statement-v2.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let fixture: serde_json::Value = serde_json::from_str(&raw).expect("fixture is JSON");
        let seed = hex::decode(fixture["private_key_seed_hex"].as_str().unwrap()).unwrap();
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed.try_into().unwrap());
        let manifest = &fixture["manifest"];
        let artifact_digest = fixture["artifact_digest"].as_str().unwrap();

        let manifest_sha = manifest_sha256(manifest).unwrap();
        assert_eq!(
            manifest_sha,
            fixture["expected_manifest_sha256"].as_str().unwrap(),
            "the canonical manifest digest is the published vector"
        );
        let statement = statement_bytes(artifact_digest, &manifest_sha);
        assert_eq!(
            statement,
            fixture["expected_statement"].as_str().unwrap(),
            "the exact signed bytes are the published vector"
        );
        let message = statement_message(artifact_digest, &manifest_sha);
        assert_eq!(
            hex::encode(signing.sign(&message).to_bytes()),
            fixture["expected_signature_hex"].as_str().unwrap(),
            "the plugin-kit signer and this host agree on the signature"
        );
    }
}
