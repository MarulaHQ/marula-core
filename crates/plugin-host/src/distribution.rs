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

/// Typed refusal reasons (deny-by-default surface — typed codes, never
/// prose to parse; these ride audit details and API errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestReason {
    /// A GitHub pull without pinned tag/commit (or pinned to HEAD).
    NoPin,
    /// Digest missing or not a sha256 hex string.
    BadDigest,
    /// Registry signature block missing/empty (code-signed artifacts).
    BadSignature,
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
pub fn ingest_registry_entry(catalog_json: &[u8]) -> Result<DistributionBundle, IngestReason> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(catalog_json) else {
        return Err(IngestReason::BadManifest);
    };
    let tag = required_string(&value, "tag")?;
    validate_git_tag(tag)?;
    let artifact = value.get("artifact").ok_or(IngestReason::BadManifest)?;
    let digest = required_string(artifact, "digest")?;
    require_sha256(digest)?;
    let signature = artifact
        .get("signature")
        .and_then(|v| v.as_str())
        .ok_or(IngestReason::BadManifest)?;
    if signature.trim().is_empty() {
        return Err(IngestReason::BadSignature);
    }
    let manifest =
        validate_plugin_manifest(value.get("manifest").ok_or(IngestReason::BadManifest)?)?;
    let slug = value
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or(manifest.name.as_str());
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
    let manifest = validate_plugin_manifest(
        &serde_json::from_slice(&manifest_bytes).map_err(|_| IngestReason::BadManifest)?,
    )?;
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
            "publisher": "meridian",
            "tag": "v1.2.0",
            "artifact": {
                "digest": digest_of("artifact bytes"),
                "signature": "sig-bytes"
            },
            "manifest": serde_json::from_str::<serde_json::Value>(GOOD_MANIFEST).unwrap()
        });
        let bundle = ingest_registry_entry(&serde_json::to_vec(&payload).unwrap()).unwrap();
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
            ingest_registry_entry(&serde_json::to_vec(&payload).unwrap()).unwrap_err(),
            IngestReason::BadSignature
        );
        payload["tag"] = json!("HEAD");
        assert_eq!(
            ingest_registry_entry(&serde_json::to_vec(&payload).unwrap()).unwrap_err(),
            IngestReason::NoPin
        );
        payload["tag"] = json!("main");
        assert_eq!(
            ingest_registry_entry(&serde_json::to_vec(&payload).unwrap()).unwrap_err(),
            IngestReason::NoPin
        );
        // TotMalformed archive (not JSON at all).
        assert_eq!(
            ingest_registry_entry(b"not json").unwrap_err(),
            IngestReason::BadManifest
        );
    }

    #[test]
    fn github_pull_should_refuse_head_pins_and_missing_columns() {
        let manifest = serde_json::from_str::<serde_json::Value>(GOOD_MANIFEST).unwrap();
        assert_eq!(
            ingest_github_pull("org", "repo", "", "", manifest.clone(), &digest_of("x"))
                .unwrap_err(),
            IngestReason::NoPin
        );
        assert_eq!(
            ingest_github_pull("org", "repo", "HEAD", "", manifest.clone(), &digest_of("x"))
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
                &digest_of("x")
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
                "zzz"
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
