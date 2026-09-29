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
// Signing convention (documented, stable): the operator's ed25519 PUBLIC
// key (64 hex chars) is configured server-side
// (`MARULA_PLUGIN_SIGNING_KEY`); the artifact signature is an ed25519
// signature over the RAW 32 BYTES the artifact digest hex decodes to,
// hex-encoded (128 chars) on the wire. Verification is a REAL
// cryptographic check — a configured key plus any invalid/missing
// signature refuses the artifact with `BadSignature`; with NO key
// configured the seam degrades to the structural-only check and says so
// loudly in the log (dev posture, never production).

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

/// One signature verification: key + digest hex + wire signature (hex).
/// The typed refusal is ALWAYS [`IngestReason::BadSignature`] — key
/// material never reaches error text.
pub fn verify_artifact_signature(
    signing_key_hex: &str,
    artifact_digest_hex: &str,
    signature_hex: &str,
) -> Result<(), IngestReason> {
    let key_bytes: [u8; 32] = hex::decode(signing_key_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(IngestReason::BadSignature)?;
    let vk = VerifyingKey::from_bytes(&key_bytes).map_err(|_| IngestReason::BadSignature)?;
    let digest_bytes: [u8; 32] = hex::decode(artifact_digest_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(IngestReason::BadSignature)?;
    let sig_bytes: [u8; 64] = hex::decode(signature_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(IngestReason::BadSignature)?;
    let sig = Signature::from_bytes(&sig_bytes);
    vk.verify_strict(&digest_bytes, &sig)
        .map_err(|_| IngestReason::BadSignature)
}

/// Verifies a signature against the trusted key set by trying every key.
/// Returns [`IngestReason::BadSignature`] when the trust set is EMPTY (a
/// key-configured operator whose registry is unpopulated must not
/// silently degrade to structural-only — the single-key seam's dev
/// posture never re-enters through rotation) or when nothing verifies.
/// With exactly one trusted key this is byte-for-byte the old
/// single-key behavior.
pub fn verify_artifact_signature_against_keys(
    keys: &[TrustedSigningKey],
    artifact_digest_hex: &str,
    signature_hex: &str,
) -> Result<(), IngestReason> {
    if keys.is_empty() {
        return Err(IngestReason::NoTrustedKeys);
    }
    for key in keys {
        if verify_artifact_signature(&key.public_key, artifact_digest_hex, signature_hex).is_ok() {
            return Ok(());
        }
    }
    Err(IngestReason::BadSignature)
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
        if keys.is_empty() {
            return Err(IngestReason::NoTrustedKeys);
        }
        if let Some(key_id) = &self.key_id {
            // jotted narrow trust: block names a key, OTHER keys do not
            // count even if their material verifies — naming is
            // narrowing (the rotation model's tamper-evidence axis).
            for key in keys {
                if key.key_id == *key_id {
                    return verify_artifact_signature(&key.public_key, digest_hex, &self.signature);
                }
            }
            return Err(IngestReason::UnknownKey);
        }
        verify_artifact_signature_against_keys(keys, digest_hex, &self.signature)
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
    /// No trusted key is configured (operator registry empty AND no
    /// legacy env key) while signature policy demands one.
    NoTrustedKeys,
    /// The manifest is unreadable/unknown/malformed.
    BadManifest,
    /// The zip import breaches a sandbox cap.
    ArchiveCap,
    /// Not a zipper we accept (bad magic / traversal paths / dupes).
    BadArchive,
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
            IngestReason::NoTrustedKeys => "distribution_signing_key_unconfigured",
            IngestReason::BadManifest => "distribution_manifest_invalid",
            IngestReason::ArchiveCap => "distribution_archive_cap",
            IngestReason::BadArchive => "distribution_archive_invalid",
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

/// Maximum accepted zip import size (bytes).
pub const MAX_ARCHIVE_BYTES: i64 = 8 * 1024 * 1024;
/// Maximum members inside one archive.
pub const MAX_ARCHIVE_ENTRIES: usize = 256;
/// Maximum per-member uncompressed size (bytes).
pub const MAX_MEMBER_BYTES: usize = 4 * 1024 * 1024;
/// Maximum total uncompressed size across members (zip-bomb guard).
pub const MAX_ARCHIVE_UNCOMPRESSED: usize = 8 * 1024 * 1024;

// --- adapters -------------------------------------------------------------------

/// ADR 0009 §2 — public marketplace registry (BUILD-PROMPT v1 fixture
/// shape: static catalog JSON + git tag + code-signature/digest). The
/// registry itself is the review channel for this source, but the org
/// still lands through the staged consent diff.
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
    // B1: with a trusted key configured the signature is
    // CRYPTOGRAPHICALLY verified against the digest by key-id block or
    // trust set; an EMPTY slice stays the structural-only dev posture
    // (the route layer logs that loudly at boot — never production).
    if !signing_keys.is_empty() {
        signature.verify(signing_keys, digest)?;
    }
    let manifest_value = value.get("manifest").ok_or(IngestReason::BadManifest)?;
    let slug = value
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            manifest_value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unspecified")
        });
    if is_theme_manifest(manifest_value) {
        let pack = validate_theme_manifest(manifest_value)?;
        return Ok(bundle(
            PluginArtifactKind::Theme,
            PluginSourceKind::MarketplaceRegistry,
            &format!("registry://{slug}"),
            Some(tag),
            digest,
            serde_json::to_value(&pack).unwrap_or(serde_json::Value::Null),
            Vec::new(),
        ));
    }
    let manifest = validate_plugin_manifest(manifest_value)?;
    Ok(bundle(
        PluginArtifactKind::Plugin,
        PluginSourceKind::MarketplaceRegistry,
        &format!("registry://{slug}"),
        Some(tag),
        digest,
        serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null),
        manifest.capabilities,
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
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(IngestReason::ArchiveCap);
    }
    let mut manifest_bytes: Option<Vec<u8>> = None;
    let mut total_uncompressed: usize = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|_| IngestReason::BadArchive)?;
        let Some(name) = std::str::from_utf8(entry.name_raw()).ok().map(String::from) else {
            return Err(IngestReason::BadArchive);
        };
        // Traversal-safety: no "/" or ".." — a zip import never aims at
        // any filesystem path.
        if name.contains('/') || name.contains('\\') || name.starts_with('.') {
            return Err(IngestReason::BadArchive);
        }
        if name.as_str() != "manifest.json" {
            continue;
        }
        if (entry.size() as usize) > MAX_MEMBER_BYTES {
            return Err(IngestReason::ArchiveCap);
        }
        total_uncompressed += entry.size() as usize;
        if total_uncompressed > MAX_ARCHIVE_UNCOMPRESSED {
            return Err(IngestReason::ArchiveCap);
        }
        if manifest_bytes.is_some() {
            return Err(IngestReason::BadArchive);
        }
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|_| IngestReason::BadArchive)?;
        manifest_bytes = Some(buf);
    }
    let Some(manifest_bytes) = manifest_bytes else {
        return Err(IngestReason::BadArchive);
    };
    let digest = sha256_hex(bytes);
    let manifest_value: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).map_err(|_| IngestReason::BadManifest)?;
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
    }
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
        // The zero-translation posture: upload design/marula's export
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
}
