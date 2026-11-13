//! Loopback credentials for the desktop shell.
//!
//! Two things this app used to publish on loopback were compile-time
//! defaults: the JWT secret the in-process core-api signs tokens with (and
//! derives its at-rest seal key from), and the embedded Postgres passwords.
//! Both sit in the public source, and the API and the database listen on
//! `127.0.0.1`, so any other local process could mint an owner token against
//! the API or connect to Postgres as a superuser.
//!
//! The published dev value that REMAINS is the JWT secret alone (ADR 0005:
//! the demo seed's sealed columns, compose parity (`deploy/docker-compose.yml`)
//! and a developer's existing data directory all carry it). The role passwords
//! a debug build runs its embedded cluster with are derived from that secret,
//! so no credential beyond one dev value ships in this source — and a data
//! directory an OLDER build created opens through the one-time rotation that
//! resolves [`legacy_superuser_password`] from the environment at runtime.
//!
//! A release build (the `desktop` profile `scripts/build-desktop.sh` and the
//! bundler use) reads a per-install secret from the app data dir and mints one
//! on first launch. The Postgres passwords are derived from it, so a release
//! build trusts no credential that ships in this repository. The secret
//! persists across launches: derived from it are the JWT signing key and the
//! at-rest key that opens sealed rows in the existing data directory, so a
//! fresh value per launch would make yesterday's sealed data unreadable.

use anyhow::Context;
use rand::TryRng;
use std::path::Path;

/// Byte length of the per-install secret before hex encoding.
const SECRET_BYTES: usize = 32;
/// Length of the hex-encoded secret.
const SECRET_HEX_LEN: usize = SECRET_BYTES * 2;

/// The compose/dev JWT secret. Debug builds, and only debug builds, pin it:
/// the demo seed's at-rest seal key derives from it, which is what keeps a
/// developer's existing data dir readable and the compose stack in parity.
/// The role passwords a debug build uses derive from it too — see
/// [`PgCredentials::dev_pg`] — so no other credential ships in this source.
pub const DEV_JWT_SECRET: &str = "dev-only-insecure-secret-change-me";

/// The environment variable that carries the ONE-TIME rotation credential
/// for an embedded cluster a pre-hardening build created (see the module
/// doc). It is read, and never stored, only when such a cluster is opened.
pub const LEGACY_PG_PASSWORD_VAR: &str = "MARULA_DESKTOP_LEGACY_PG_PASSWORD";

/// Under the app data dir: the per-install secret a release build mints.
pub const SECRET_FILE: &str = "desktop-secret";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgCredentials {
    /// Bootstrap superuser (`postgres`).
    pub superuser: String,
    /// Schema owner and migration role (`marula`, SUPERUSER for compose
    /// parity).
    pub owner: String,
    /// The RLS-constrained role the API connects as (`marula_app`).
    pub app: String,
}

impl PgCredentials {
    /// The credentials a debug build runs its embedded cluster with:
    /// derived from the published dev JWT secret, so they are stable across
    /// launches — a developer's existing cluster keeps opening — while
    /// carrying no credential besides that one dev value. The labels are
    /// the ones [`select`] derives for a release build; one derivation
    /// serves both postures.
    pub fn dev_pg() -> Self {
        Self {
            superuser: derive(DEV_JWT_SECRET, "pg-superuser"),
            owner: derive(DEV_JWT_SECRET, "pg-owner"),
            app: derive(DEV_JWT_SECRET, "pg-app"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSecrets {
    /// HS256 signing secret for access/refresh tokens.
    pub jwt_secret: String,
    pub pg: PgCredentials,
    /// True when this launch signs with the published dev JWT secret (the
    /// one dev value ADR 0005 keeps published). Drives the loud boot
    /// warning, and is what a reviewer should read as "this build exposes
    /// the repository's dev secret".
    pub publishes_dev_defaults: bool,
}

impl LaunchSecrets {
    /// The at-rest seal key the demo seed and the API both derive from the
    /// effective JWT secret (`core_api::config` does the same derivation).
    pub fn seal_key(&self) -> [u8; 32] {
        core_db::demo::derive_dev_seal_key(&self.jwt_secret)
    }

    fn dev_defaults() -> Self {
        Self {
            jwt_secret: DEV_JWT_SECRET.to_string(),
            pg: PgCredentials::dev_pg(),
            publishes_dev_defaults: true,
        }
    }
}

/// The secrets this binary should use: the dev posture in a debug build,
/// a per-install secret in a release build.
pub fn for_this_build(root: &Path) -> anyhow::Result<LaunchSecrets> {
    select(root, !cfg!(debug_assertions))
}

/// The choice behind [`for_this_build`], with the build posture as an
/// argument so both branches are testable from one `cargo test` run.
pub fn select(root: &Path, release_build: bool) -> anyhow::Result<LaunchSecrets> {
    if !release_build {
        return Ok(LaunchSecrets::dev_defaults());
    }
    let secret = install_secret(root)?;
    Ok(LaunchSecrets {
        jwt_secret: secret.clone(),
        pg: PgCredentials {
            superuser: derive(&secret, "pg-superuser"),
            owner: derive(&secret, "pg-owner"),
            app: derive(&secret, "pg-app"),
        },
        publishes_dev_defaults: false,
    })
}

/// Read the install secret, minting it on first launch.
fn install_secret(root: &Path) -> anyhow::Result<String> {
    let path = root.join(SECRET_FILE);
    match std::fs::read_to_string(&path) {
        Ok(existing) => {
            let secret = existing.trim().to_string();
            anyhow::ensure!(
                is_secret(&secret),
                "{} does not hold a {SECRET_HEX_LEN}-hex install secret — delete it and relaunch \
                 to mint a new one (the at-rest key derived from it will no longer open sealed \
                 rows written under the old secret)",
                path.display()
            );
            Ok(secret)
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("reading {}", path.display()))
        }
        Err(_) => {
            let mut bytes = [0u8; SECRET_BYTES];
            let mut rng = rand::rngs::SysRng;
            rng.try_fill_bytes(&mut bytes)
                .context("reading the OS CSPRNG for the install secret")?;
            let secret = hex::encode(bytes);
            match write_owner_only_new(&path, &secret) {
                // Two launches raced before the single-instance plugin was up:
                // the winner's file is the install secret.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let existing = std::fs::read_to_string(&path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    let secret = existing.trim().to_string();
                    anyhow::ensure!(
                        is_secret(&secret),
                        "{} was created by another launch but does not hold a \
                         {SECRET_HEX_LEN}-hex secret",
                        path.display()
                    );
                    Ok(secret)
                }
                Err(e) => Err(e).with_context(|| format!("writing {}", path.display())),
                Ok(()) => {
                    if root.join("pgdata").exists() {
                        tracing::warn!(
                            dir = %root.join("pgdata").display(),
                            "an existing data directory had no install secret, so this launch \
                             minted one: sealed rows written under the previous key (the demo \
                             password vault, integration secrets) will not open under it"
                        );
                    }
                    tracing::info!(
                        file = %path.display(),
                        "minted this install's loopback secret (0600); the API and the embedded \
                         Postgres use it instead of the repository's published defaults"
                    );
                    Ok(secret)
                }
            }
        }
    }
}

/// Derive one credential from the install secret. `core_db`'s dev derivation
/// is a domain-separated SHA-256 over the seed, which is exactly a KDF here:
/// the label keeps the three passwords independent.
fn derive(secret: &str, label: &str) -> String {
    let digest = core_db::demo::derive_dev_seal_key(&format!("marula-desktop-{label}:{secret}"));
    hex::encode(&digest[..16])
}

fn is_secret(candidate: &str) -> bool {
    candidate.len() == SECRET_HEX_LEN && candidate.chars().all(|c| c.is_ascii_hexdigit())
}

/// The superuser credential an embedded cluster older than the per-install
/// secret was initialized with. It is deliberately nowhere in this source —
/// a release binary must not carry a credential from the repository — so a
/// legacy cluster's one-time rotation resolves it from the environment at
/// the moment it is needed. `None` means "no legacy cluster to open": the
/// caller surfaces operator copy instead of trying a second credential.
#[must_use]
pub fn legacy_superuser_password() -> Option<String> {
    resolved_from_env(std::env::var(LEGACY_PG_PASSWORD_VAR))
}

/// The pure half of [`legacy_superuser_password`]: a candidate env value,
/// trimmed and blank-refused — testable without touching process env.
fn resolved_from_env(candidate: Result<String, std::env::VarError>) -> Option<String> {
    candidate
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(unix)]
fn write_owner_only_new(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_owner_only_new(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

/// Create or replace `path` with `contents`, owner-only. Used for the
/// short-lived bootstrap password file `initdb` reads.
#[cfg(unix)]
pub fn write_owner_only(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

#[cfg(not(unix))]
pub fn write_owner_only(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_data_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[test]
    fn a_debug_build_signs_with_the_published_secret_and_derives_its_pg_credentials() {
        let root = app_data_dir();
        let secrets = select(root.path(), false).expect("select");
        assert_eq!(secrets.jwt_secret, DEV_JWT_SECRET);
        assert_eq!(secrets.pg, PgCredentials::dev_pg());
        assert!(secrets.publishes_dev_defaults);
        assert!(
            !root.path().join(SECRET_FILE).exists(),
            "a debug build must not mint an install secret"
        );
    }

    #[test]
    fn a_release_build_mints_a_per_install_secret_and_never_the_published_one() {
        let root = app_data_dir();
        let secrets = select(root.path(), true).expect("select");

        assert_ne!(secrets.jwt_secret, DEV_JWT_SECRET);
        assert_eq!(secrets.jwt_secret.len(), SECRET_HEX_LEN);
        assert!(is_secret(&secrets.jwt_secret));
        assert!(!secrets.publishes_dev_defaults);

        let file = std::fs::read_to_string(root.path().join(SECRET_FILE)).expect("secret file");
        assert_eq!(file.trim(), secrets.jwt_secret);

        // A second launch reads the same secret, so sealed rows stay openable.
        let again = select(root.path(), true).expect("select again");
        assert_eq!(again, secrets);
    }

    #[test]
    fn the_build_this_crate_compiled_selects_the_posture_it_claims() {
        let root = app_data_dir();
        let secrets = for_this_build(root.path()).expect("select");
        assert_eq!(
            secrets.jwt_secret == DEV_JWT_SECRET,
            cfg!(debug_assertions),
            "the published dev secret is used by a debug build and by nothing else"
        );
        if cfg!(debug_assertions) {
            assert!(!root.path().join(SECRET_FILE).exists());
        } else {
            assert!(!secrets.publishes_dev_defaults);
        }
    }

    #[test]
    fn release_database_passwords_are_distinct_from_the_dev_derivation_and_each_other() {
        let root = app_data_dir();
        let release_pg = select(root.path(), true).expect("release select").pg;
        let debug_pg = PgCredentials::dev_pg();
        assert_ne!(
            release_pg, debug_pg,
            "a release build must not share the debug derivation: its seed is this \
             install's secret, not the published dev value"
        );
        for (label, released, dev) in [
            ("superuser", &release_pg.superuser, &debug_pg.superuser),
            ("owner", &release_pg.owner, &debug_pg.owner),
            ("app", &release_pg.app, &debug_pg.app),
        ] {
            assert_ne!(
                released, dev,
                "the {label} password must differ between postures"
            );
            assert!(
                released.len() >= 16,
                "the release {label} password is too short to be a credential"
            );
        }
        assert_ne!(release_pg.superuser, release_pg.owner);
        assert_ne!(release_pg.owner, release_pg.app);
        assert_ne!(release_pg.superuser, release_pg.app);
    }

    #[test]
    fn the_legacy_cluster_credential_is_resolved_from_the_environment_only() {
        // The pure half of the resolver: unset and blank read as "no legacy
        // cluster", whitespace is not part of the credential.
        assert!(resolved_from_env(Err(std::env::VarError::NotPresent)).is_none());
        assert!(resolved_from_env(Ok(String::new())).is_none());
        assert!(resolved_from_env(Ok("   ".to_owned())).is_none());
        assert_eq!(
            resolved_from_env(Ok("  value  ".to_owned())).as_deref(),
            Some("value")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_install_secret_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = app_data_dir();
        select(root.path(), true).expect("select");
        let mode = std::fs::metadata(root.path().join(SECRET_FILE))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "no group or other bit may be set on the install secret (mode {mode:o})"
        );
    }

    #[test]
    fn a_secret_file_that_is_not_a_secret_fails_closed() {
        let root = app_data_dir();
        std::fs::write(root.path().join(SECRET_FILE), "not-a-secret\n").expect("write");
        let error = select(root.path(), true).expect_err("must refuse");
        let message = format!("{error:#}");
        assert!(message.contains(SECRET_FILE), "{message}");
        assert!(!message.contains("not-a-secret"), "{message}");
    }

    #[cfg(unix)]
    #[test]
    fn the_owner_only_writer_replaces_and_narrows_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = app_data_dir();
        let path = root.path().join("pgpass-init");
        std::fs::write(&path, "first").expect("seed");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("widen");
        write_owner_only(&path, "second").expect("write");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "second");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
