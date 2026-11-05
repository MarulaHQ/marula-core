//! Where this client's data lives: this machine, or an instance.
//!
//! ADR 0028 §1 says the desktop client loads the same front end the web
//! deployment serves, and §4 says a client may not keep a store of its own.
//! Both are silent on the question the shell cannot answer by itself: *which*
//! deployment. This module answers it from one small file, `connection.json`,
//! written by the first-run screen or by `--server`, and read before anything
//! else starts.
//!
//! Two modes:
//!
//! - [`Mode::Local`] is the embedded app from ADR 0005: Postgres, core-api and
//!   the bundled Next.js server in this process. The database is local and the
//!   client is the server.
//! - [`Mode::Remote`] is a deployment that is already running — a self-hosted
//!   stack or a hosted instance. The shell boots nothing of its own and loads
//!   that deployment's own URL.
//!
//! The file holds an address, never a credential. A user who can edit it can
//! already point a browser anywhere, so it is not part of the trust surface;
//! what it decides is which front end the webview loads.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The file, inside the app data dir.
pub const FILE_NAME: &str = "connection.json";

/// Scripted override for [`resolve`]. `local` selects the embedded app; any
/// other value is parsed as a server URL. Used by `scripts/uninstall-desktop.sh`
/// and by anyone who wants one launch to ignore the stored choice.
pub const ENV_OVERRIDE: &str = "MARULA_DESKTOP_SERVER";

/// The stored document's shape. Bumped only when a reader could misread an
/// older file, not when a field is added.
const VERSION: u32 = 1;

/// The decision this client made at startup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Mode {
    /// The embedded app from ADR 0005.
    Local,
    /// An instance that is already running.
    Remote { server_url: String },
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    version: u32,
    #[serde(flatten)]
    mode: Mode,
}

/// `connection.json` under the app data dir.
pub fn path(root: &Path) -> PathBuf {
    root.join(FILE_NAME)
}

/// The stored choice, or `None` when this is a first run.
///
/// A file that exists but cannot be read is an error rather than a silent
/// reset: the connect screen shows the reason and lets the person correct it,
/// which is better than a client that quietly forgets where its data lives.
pub fn load(root: &Path) -> Result<Option<Mode>, String> {
    let file = path(root);
    let raw = match std::fs::read_to_string(&file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{} could not be read: {e}", file.display())),
    };
    let stored: Stored = serde_json::from_str(&raw)
        .map_err(|e| format!("{} is not a connection file: {e}", file.display()))?;
    if stored.version != VERSION {
        return Err(format!(
            "{} was written by a newer version of Marula (file version {}, this build reads {VERSION})",
            file.display(),
            stored.version
        ));
    }
    Ok(Some(stored.mode))
}

/// Write the choice, atomically, so a crash mid-write cannot leave a file the
/// next launch refuses to read.
pub fn save(root: &Path, mode: &Mode) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let stored = Stored {
        version: VERSION,
        mode: mode.clone(),
    };
    let mut body = serde_json::to_vec_pretty(&stored)?;
    body.push(b'\n');
    let target = path(root);
    let scratch = target.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&scratch)?;
        file.write_all(&body)?;
        file.sync_all()?;
    }
    std::fs::rename(&scratch, &target)
}

/// The environment override, then the stored choice.
pub fn resolve(root: &Path) -> Result<Option<Mode>, String> {
    if let Some(raw) = std::env::var(ENV_OVERRIDE)
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        if raw.trim().eq_ignore_ascii_case("local") {
            return Ok(Some(Mode::Local));
        }
        return parse_server_url(&raw).map(|server_url| Some(Mode::Remote { server_url }));
    }
    load(root)
}

/// Normalise and validate a server address a person typed or a flag supplied.
///
/// The shell loads whatever this returns into a webview, so it has to be a web
/// address and nothing else: no `file:`, no `javascript:`, no credentials
/// smuggled in front of the host, and no query or fragment, which would change
/// what the address means without changing the field it is shown in.
pub fn parse_server_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(
            "Enter the address of your Marula instance, for example https://psa.example.com."
                .into(),
        );
    }
    let url = tauri::Url::parse(trimmed).map_err(|_| {
        "That is not a web address. It should start with http:// or https://.".to_string()
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("A Marula instance is reached over http or https, not this.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "An address cannot carry a username or a password. Sign in once the app has loaded."
                .into(),
        );
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("That address has no host name.".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("An address cannot carry a query or a fragment.".into());
    }
    let mut out = format!(
        "{}://{}",
        url.scheme(),
        url.host_str().expect("host checked above")
    );
    if let Some(port) = url.port() {
        out.push_str(&format!(":{port}"));
    }
    let path = url.path().trim_end_matches('/');
    if !path.is_empty() {
        out.push_str(path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("marula-connection-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_address_is_normalised_and_kept() {
        let cases = [
            ("https://psa.example.com", "https://psa.example.com"),
            ("https://psa.example.com/", "https://psa.example.com"),
            ("  https://psa.example.com/  ", "https://psa.example.com"),
            ("http://192.168.1.50:8080", "http://192.168.1.50:8080"),
            ("https://example.com/marula/", "https://example.com/marula"),
            ("HTTP://Example.COM", "http://example.com"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_server_url(input).as_deref(),
                Ok(expected),
                "input: {input}"
            );
        }
    }

    #[test]
    fn anything_that_is_not_a_web_origin_is_refused() {
        for input in [
            "",
            "   ",
            "psa.example.com",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hi",
            "ftp://example.com",
            "//example.com",
            "https://user:secret@example.com",
            "https://example.com/?next=evil",
            "https://example.com/#fragment",
        ] {
            assert!(
                parse_server_url(input).is_err(),
                "{input:?} should be refused"
            );
        }
    }

    #[test]
    fn a_missing_file_is_a_first_run_not_a_failure() {
        let root = scratch("missing");
        assert_eq!(load(&root), Ok(None));
    }

    #[test]
    fn both_modes_round_trip_through_the_file() {
        let root = scratch("roundtrip");
        for mode in [
            Mode::Local,
            Mode::Remote {
                server_url: "https://psa.example.com".into(),
            },
        ] {
            save(&root, &mode).expect("save");
            assert_eq!(load(&root), Ok(Some(mode)));
        }
        // The file is written for a person to read, so it is not opaque.
        let raw = std::fs::read_to_string(path(&root)).expect("read");
        assert!(raw.contains("\"version\": 1"), "{raw}");
        assert!(raw.contains("\"mode\": \"remote\""), "{raw}");
    }

    #[test]
    fn a_corrupt_file_is_reported_rather_than_overwritten_silently() {
        let root = scratch("corrupt");
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(path(&root), "{ not json").expect("write");
        let err = load(&root).expect_err("a corrupt file must not read as a first run");
        assert!(err.contains("connection.json"), "{err}");
    }

    #[test]
    fn a_newer_file_version_is_refused() {
        let root = scratch("newer");
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(path(&root), r#"{"version": 99, "mode": "local"}"#).expect("write");
        let err = load(&root).expect_err("an unknown version must not be guessed at");
        assert!(err.contains("newer version"), "{err}");
    }
}
