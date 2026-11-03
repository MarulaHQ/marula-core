//! What a release bundle says about its own images.
//!
//! A release carries `release.json` beside the wizard, and that file records one
//! digest per service. The digest is the identity an install consumes, so a
//! bundle that already names them should not need the operator to copy four
//! `--image-digest` flags out of a signed document by hand: that is a step where
//! a typo turns into a wrong image, and it is the step this module removes.
//!
//! The manifest is not the update manifest. `update` reads a channel document
//! listing releases and advisories; this reads the bundle's own description of
//! what it will run.

use std::collections::BTreeMap;

/// The registry prefix and one digest per service, lifted from a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Images {
    /// `ghcr.io/marulagh`, shared by every reference in the file.
    pub base: String,
    /// `core-api` → `sha256:<64 hex>`.
    pub digests: BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
struct Document {
    #[serde(default)]
    images: BTreeMap<String, String>,
}

/// The bundle's images, as an install will consume them.
///
/// Three places, in one order for both hosts: the path named on the command
/// line, then `release.json` beside the install directory, then one in the
/// working directory. A released bundle ships the file next to what it installs,
/// so the common case needs no flag in the terminal and no field in the wizard —
/// and both get the same answer, because both call this.
///
/// `Ok(None)` means no manifest was found, which is not an error: a source
/// install has nothing to read, and a registry install with a typed prefix does
/// not need one.
pub fn resolve(
    dir: &std::path::Path,
    explicit: Option<&std::path::Path>,
) -> Result<Option<Images>, String> {
    let candidate = explicit.map(std::path::Path::to_path_buf).or_else(|| {
        let beside = dir.join("release.json");
        beside.is_file().then_some(beside)
    });
    let candidate = candidate.or_else(|| {
        let here = std::path::PathBuf::from("release.json");
        here.is_file().then_some(here)
    });
    let Some(path) = candidate else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    parse(&text).map(Some)
}

/// Parses `release.json`'s `images` map.
///
/// Each value is a full reference, `<base>/<image>@sha256:<hex>`. A tag in that
/// position is refused rather than resolved: a tag can be moved, and the point
/// of the field is that it cannot be.
pub fn parse(text: &str) -> Result<Images, String> {
    let document: Document =
        serde_json::from_str(text).map_err(|error| format!("not a release manifest: {error}"))?;
    if document.images.is_empty() {
        return Err(
            "the release manifest records no images, so it cannot say what to pull; \
             pass --image-digest for each service instead"
                .to_string(),
        );
    }

    let mut base: Option<String> = None;
    let mut digests = BTreeMap::new();
    for (service, reference) in document.images {
        let (prefix, digest) = reference.split_once('@').ok_or_else(|| {
            format!(
                "the image for {service} is {reference:?}, which names no digest: a registry \
                 install pins digests, and a tag can be moved"
            )
        })?;
        if !is_digest(digest) {
            return Err(format!(
                "the image for {service} pins {digest:?}, which is not sha256:<64 hex>"
            ));
        }
        // `--image-base` composes `<prefix>-<service>`, so the prefix is the
        // registry plus the image name with the service suffix removed:
        // `ghcr.io/marulagh/marula-web` for `web` is `ghcr.io/marulagh/marula`.
        // Taking everything before the last `/` would give `ghcr.io/marulagh`
        // and produce `ghcr.io/marulagh-web`, a repository that does not exist.
        let (registry, image) = prefix.rsplit_once('/').ok_or_else(|| {
            format!("the image for {service} is {reference:?}, which names no registry")
        })?;
        let suffix = format!("-{service}");
        let stem = image.strip_suffix(&suffix).ok_or_else(|| {
            format!(
                "the image for {service} is {image:?}, which is not named `<prefix>{suffix}`;                  the installer composes `<--image-base>-<service>`"
            )
        })?;
        let candidate = format!("{registry}/{stem}");
        match &base {
            Some(seen) if seen != &candidate => {
                return Err(format!(
                    "the release manifest mixes registries: {seen} and {candidate}"
                ))
            }
            Some(_) => {}
            None => base = Some(candidate.to_string()),
        }
        digests.insert(service, digest.to_string());
    }

    Ok(Images {
        base: base.unwrap_or_default(),
        digests,
    })
}

fn is_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .map(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn document(images: &str) -> String {
        format!("{{\"schema\":1,\"assets\":{{}},\"images\":{images}}}")
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("marula-release-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// The wizard runs inside a bundle, so the file beside the install directory
    /// is the whole contract: no field in the app, no flag on the terminal.
    #[test]
    fn resolve_reads_the_manifest_beside_the_install_directory() {
        let dir = scratch("beside");
        std::fs::write(
            dir.join("release.json"),
            document(&format!(
                "{{\"core-api\":\"ghcr.io/marulagh/marula-core-api@sha256:{HEX}\"}}"
            )),
        )
        .expect("a manifest");
        let found = resolve(&dir, None)
            .expect("a readable bundle")
            .expect("a bundle beside the directory");
        assert_eq!(found.base, "ghcr.io/marulagh/marula");
        assert_eq!(found.digests["core-api"], format!("sha256:{HEX}"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A path named explicitly wins, which is what `--image-manifest` means.
    #[test]
    fn resolve_prefers_the_manifest_it_was_given() {
        let dir = scratch("explicit");
        std::fs::write(
            dir.join("release.json"),
            document(&format!(
                "{{\"core-api\":\"ghcr.io/marulagh/marula-core-api@sha256:{HEX}\"}}"
            )),
        )
        .expect("a manifest");
        let other = dir.join("signed.json");
        std::fs::write(
            &other,
            document(&format!(
                "{{\"core-api\":\"registry.example.com/team/marula-core-api@sha256:{HEX}\"}}"
            )),
        )
        .expect("a second manifest");
        let found = resolve(&dir, Some(&other))
            .expect("a readable bundle")
            .expect("the named bundle");
        assert_eq!(found.base, "registry.example.com/team/marula");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bundle_that_names_its_digests_yields_a_base_and_one_per_service() {
        let text = document(&format!(
            "{{\"core-api\":\"ghcr.io/marulagh/marula-core-api@sha256:{HEX}\",\
              \"web\":\"ghcr.io/marulagh/marula-web@sha256:{HEX}\"}}"
        ));
        let images = parse(&text).expect("a well formed bundle");
        assert_eq!(images.base, "ghcr.io/marulagh/marula");
        assert_eq!(images.digests.len(), 2);
        assert_eq!(images.digests["core-api"], format!("sha256:{HEX}"));
    }

    #[test]
    fn a_tag_is_refused_because_it_can_be_moved() {
        let text = document("{\"core-api\":\"ghcr.io/marulagh/marula-core-api:0.4.0\"}");
        let error = parse(&text).expect_err("a tag is not an identity");
        assert!(error.contains("names no digest"), "{error}");
    }

    #[test]
    fn a_digest_that_is_not_a_sha256_is_refused() {
        let text = document("{\"core-api\":\"ghcr.io/marulagh/marula-core-api@sha256:deadbeef\"}");
        let error = parse(&text).expect_err("a short digest is not a digest");
        assert!(error.contains("sha256:<64 hex>"), "{error}");
    }

    #[test]
    fn a_bundle_with_no_images_says_what_to_do_instead() {
        let error = parse("{\"schema\":1,\"assets\":{}}").expect_err("nothing to pull");
        assert!(error.contains("--image-digest"), "{error}");
    }

    #[test]
    fn two_registries_in_one_bundle_are_refused() {
        let text = document(&format!(
            "{{\"core-api\":\"ghcr.io/marulagh/marula-core-api@sha256:{HEX}\",\
              \"web\":\"registry.example.com/other/marula-web@sha256:{HEX}\"}}"
        ));
        let error = parse(&text).expect_err("one bundle, one registry");
        assert!(error.contains("mixes registries"), "{error}");
    }
}
