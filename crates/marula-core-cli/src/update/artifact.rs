//! Checksums and the signature over them (Wave 60 lane W60-5).
//!
//! Purpose: an artifact is verified before anything executes it. The rule is
//! the one the release lane's bootstrap follows: fetch `SHA256SUMS` and its
//! detached signature, check the signature over the exact bytes of the sums
//! file, then check every file it names. A file that does not match is refused
//! by name, and a sums file that lists a path outside its own directory is
//! refused rather than followed.
//!
//! Non-purpose: no key management. Trusted keys arrive from the caller, and the
//! ed25519 verification itself is `plugin-host`'s — the same implementation that
//! verifies plugin artifacts, so the project has one answer to "is this
//! signature good?" rather than two that can disagree.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use plugin_host::distribution::{verify_signature_over_against_keys, TrustedSigningKey};
use sha2::{Digest as _, Sha256};

use crate::update::UpdateError;

/// Lowercase hex SHA-256 of these bytes.
#[must_use]
pub fn digest_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Lowercase hex SHA-256 of a file.
pub fn checksum_file(path: &Path) -> Result<String, UpdateError> {
    let bytes = std::fs::read(path).map_err(|error| UpdateError::Io {
        path: path.to_path_buf(),
        error: error.to_string(),
    })?;
    Ok(digest_hex(&bytes))
}

/// One line of a `sha256sum` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SumEntry {
    pub digest: String,
    pub path: String,
}

/// Parses `SHA256SUMS`, refusing a line it cannot read rather than skipping it.
///
/// A skipped line is an unverified file, and an unverified file in a bundle is
/// the thing this module exists to prevent.
pub fn parse_sums(text: &str) -> Result<Vec<SumEntry>, UpdateError> {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (digest, rest) = line.split_once(' ').ok_or_else(|| {
            UpdateError::Sums(format!("line {} is not a checksum line: {line}", index + 1))
        })?;
        if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(UpdateError::Sums(format!(
                "line {} does not start with a sha256 hex digest: {line}",
                index + 1
            )));
        }
        // `sha256sum` writes "digest  path" and uses a leading '*' for binary.
        let path = rest.trim_start_matches([' ', '*']).trim();
        if path.is_empty() {
            return Err(UpdateError::Sums(format!(
                "line {} names no file: {line}",
                index + 1
            )));
        }
        entries.push(SumEntry {
            digest: digest.to_ascii_lowercase(),
            path: path.to_string(),
        });
    }
    if entries.is_empty() {
        return Err(UpdateError::Sums("the checksum file is empty".to_string()));
    }
    Ok(entries)
}

/// Checks every file the sums name, under `root`.
///
/// Returns the number of files verified. The first mismatch stops the run and
/// names the file, because a bundle with one bad file is not a bundle to
/// half-trust.
pub fn verify_sums(text: &str, root: &Path) -> Result<usize, UpdateError> {
    let entries = parse_sums(text)?;
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for entry in &entries {
        let relative = safe_relative(&entry.path)?;
        if seen.insert(relative.display().to_string(), ()).is_some() {
            return Err(UpdateError::Sums(format!(
                "{} is listed twice, so one of the two cannot be trusted",
                relative.display()
            )));
        }
        let full = root.join(&relative);
        let actual = checksum_file(&full)?;
        if actual != entry.digest {
            return Err(UpdateError::Sums(format!(
                "{} does not match its checksum (expected {}, found {})",
                relative.display(),
                entry.digest,
                actual
            )));
        }
    }
    Ok(entries.len())
}

/// A path from a sums file, or a refusal. Absolute paths, `..` and a Windows
/// drive prefix are refused: a checksum file that points outside the bundle is
/// asking the verifier to read something else.
fn safe_relative(path: &str) -> Result<PathBuf, UpdateError> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err(UpdateError::Sums(format!("{path} is an absolute path")));
    }
    let mut relative = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => {
                return Err(UpdateError::Sums(format!(
                    "{path} leaves the bundle directory"
                )))
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(UpdateError::Sums(format!("{path} names no file")));
    }
    Ok(relative)
}

/// Verifies a detached ed25519 signature over `message` against trusted keys.
///
/// The signature is hex, as every other signature in this project is, and an
/// empty key set refuses: "trusted by nobody" is not "verified".
pub fn verify_signature(
    keys: &[TrustedSigningKey],
    message: &[u8],
    signature_hex: &str,
) -> Result<(), UpdateError> {
    if keys.is_empty() {
        return Err(UpdateError::NoTrustedKeys);
    }
    verify_signature_over_against_keys(keys, message, signature_hex.trim())
        .map_err(|reason| UpdateError::Signature(format!("{reason:?}")))
}

/// Reads a public key file: 64 hex characters, optionally prefixed `ed25519:`.
pub fn read_public_key(path: &Path) -> Result<TrustedSigningKey, UpdateError> {
    let text = std::fs::read_to_string(path).map_err(|error| UpdateError::Io {
        path: path.to_path_buf(),
        error: error.to_string(),
    })?;
    let key = text.trim().trim_start_matches("ed25519:").trim();
    if key.len() != 64 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(UpdateError::Signature(format!(
            "{} is not an ed25519 public key (64 hex characters)",
            path.display()
        )));
    }
    Ok(TrustedSigningKey {
        key_id: key[..16].to_string(),
        public_key: key.to_ascii_lowercase(),
    })
}

/// Verifies a signed `SHA256SUMS`: the signature covers the sums file's bytes,
/// and the sums file covers every artifact.
pub fn verify_signed_sums(
    sums_bytes: &[u8],
    signature_hex: &str,
    keys: &[TrustedSigningKey],
) -> Result<usize, UpdateError> {
    verify_signature(keys, sums_bytes, signature_hex)?;
    let text = std::str::from_utf8(sums_bytes)
        .map_err(|error| UpdateError::Sums(format!("the checksum file is not UTF-8: {error}")))?;
    parse_sums(text).map(|entries| entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer as _;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marula-sums-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        dir
    }

    fn sums_for(dir: &Path, files: &[(&str, &[u8])]) -> String {
        let mut lines = Vec::new();
        for (name, bytes) in files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a parent");
            }
            std::fs::write(&path, bytes).expect("a file");
            lines.push(format!("{}  {}", digest_hex(bytes), name));
        }
        lines.join("\n")
    }

    #[test]
    fn every_file_is_checked_against_its_digest() {
        let dir = scratch();
        let sums = sums_for(
            &dir,
            &[
                ("marula", b"binary"),
                ("deploy/docker-compose.yml", b"compose"),
            ],
        );
        assert_eq!(verify_sums(&sums, &dir).expect("verified"), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_changed_byte_is_refused_and_the_file_is_named() {
        let dir = scratch();
        let sums = sums_for(&dir, &[("marula", b"binary")]);
        std::fs::write(dir.join("marula"), b"binarz").expect("tamper");
        let refused = verify_sums(&sums, &dir).expect_err("a mismatch is refused");
        assert!(refused.to_string().contains("marula"), "{refused}");
        assert!(refused.to_string().contains("does not match"), "{refused}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_outside_the_bundle_is_refused() {
        let dir = scratch();
        for bad in ["../outside", "/etc/passwd", "deploy/../../escape"] {
            let sums = format!("{}  {bad}", "0".repeat(64));
            let refused = verify_sums(&sums, &dir).expect_err("a traversal is refused");
            assert!(
                refused.to_string().contains("leave") || refused.to_string().contains("absolute"),
                "{refused}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_line_that_is_not_a_checksum_refuses_rather_than_being_skipped() {
        let dir = scratch();
        assert!(parse_sums("not a checksum line").is_err());
        assert!(parse_sums("ab  short-digest").is_err());
        assert!(parse_sums("").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_signature_covers_the_sums_file_and_a_tampered_one_refuses() {
        let keys = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
        let public = keys.verifying_key();
        let trusted = vec![TrustedSigningKey {
            key_id: "test".to_string(),
            public_key: hex::encode(public.as_bytes()),
        }];
        let sums = "0".repeat(64) + "  marula";
        let signature = hex::encode(keys.sign(sums.as_bytes()).to_bytes());

        assert_eq!(
            verify_signed_sums(sums.as_bytes(), &signature, &trusted).expect("verified"),
            1
        );

        let tampered = sums.replace("marula", "marula.sh");
        let refused = verify_signed_sums(tampered.as_bytes(), &signature, &trusted)
            .expect_err("a changed sums file is refused");
        assert!(matches!(refused, UpdateError::Signature(_)), "{refused}");
    }

    #[test]
    fn no_trusted_keys_is_not_a_pass() {
        let refused = verify_signature(&[], b"document", &"0".repeat(128));
        assert!(matches!(refused, Err(UpdateError::NoTrustedKeys)));
    }

    #[test]
    fn a_public_key_file_is_read_and_validated() {
        let dir = scratch();
        let path = dir.join("release.pub");
        std::fs::write(&path, format!("ed25519:{}\n", "a".repeat(64))).expect("a key");
        let key = read_public_key(&path).expect("a key");
        assert_eq!(key.public_key, "a".repeat(64));

        std::fs::write(&path, "not-a-key").expect("a bad key");
        assert!(read_public_key(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
