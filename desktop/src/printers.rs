//! Configured printers for the raw-ZPL transport.
//!
//! The web app renders a label and asks this shell to move the bytes. The
//! destination may not come from the renderer: a script in that origin (a bug
//! in the app, a compromised dependency, a page the webview was navigated to)
//! could otherwise use the shell as a TCP write primitive against every host
//! the workstation can reach — a port scanner with a payload, and a way to
//! spray bytes at internal devices.
//!
//! So the shell owns the list, in `<app data>/printers.json`, and a send
//! resolves the caller's `host:port` against it. The caller's string is a
//! lookup key, never the address that gets connected to: the host and port
//! written to the socket are the configured ones. An unconfigured destination
//! is refused with a message that names the file and the printers it holds,
//! so the operator's next step is in the error rather than in a log.
//!
//! The file the shell writes on first launch is empty on purpose. A printer is
//! desk hardware with an address the person at the desk knows; an installer
//! guessing one would be wrong silently, which is worse than an empty list.

use std::path::{Path, PathBuf};

/// Under the app data dir: the printers this shell may send to.
pub const CONFIG_FILE: &str = "printers.json";

/// Zebra's bulk-ZPL port, and the default when a host is named without one.
pub const DEFAULT_PORT: u16 = 9100;

/// One printer the shell may send to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Printer {
    /// Display label for the refusal message ("office-zebra").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Hostname or address, as configured.
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFile {
    #[serde(default)]
    printers: Vec<Printer>,
}

/// Where a send will actually go: a configured printer, spelled by its config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub host: String,
    pub port: u16,
}

/// The configured printers, read from `<app data>/printers.json`.
///
/// The file is re-read on every send rather than snapshotted at launch: an
/// address added while the app runs is usable without a restart, and the disk
/// stays the single authority. A send is not a hot path, and the file is small.
pub struct Registry {
    path: PathBuf,
}

impl Registry {
    pub fn load(root: &Path) -> Self {
        Self {
            path: root.join(CONFIG_FILE),
        }
    }

    /// The configured printers as they are on disk now.
    ///
    /// A missing file and a broken file are both refusals rather than fatal:
    /// label printing falls back to the browser dialog and the app still runs.
    /// The refusal carries the reason and the path.
    fn read(&self) -> Result<Vec<Printer>, String> {
        match std::fs::read_to_string(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!(
                "no printers are configured — add one to {} (shape: \
                 {{\"printers\":[{{\"name\":\"office-zebra\",\"host\":\"10.0.0.9\",\"port\":9100}}]}})",
                self.path.display()
            )),
            Err(e) => Err(format!("{} could not be read: {e}", self.path.display())),
            Ok(raw) => parse(&raw, &self.path),
        }
    }

    /// Resolve a requested destination against configured state. The returned
    /// host and port are the configured ones, never the caller's spelling.
    pub fn resolve(&self, host: &str, port: u16) -> Result<Destination, String> {
        let printers = self.read()?;
        let requested = normalise_host(host);
        if requested.is_empty() {
            return Err("printer host is empty".to_string());
        }
        match printers
            .iter()
            .find(|printer| normalise_host(&printer.host) == requested && printer.port == port)
        {
            Some(printer) => Ok(Destination {
                host: printer.host.trim().to_string(),
                port: printer.port,
            }),
            None => Err(self.not_configured(&printers, &requested, port)),
        }
    }

    fn not_configured(&self, printers: &[Printer], host: &str, port: u16) -> String {
        let configured = if printers.is_empty() {
            "none".to_string()
        } else {
            printers
                .iter()
                .map(|printer| match &printer.name {
                    Some(name) => format!("{name} ({}:{})", printer.host.trim(), printer.port),
                    None => format!("{}:{}", printer.host.trim(), printer.port),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "printer {host}:{port} is not configured — configured printers: {configured}; edit {}",
            self.path.display()
        )
    }
}

/// Parse the config, validating every entry rather than half-trusting one.
fn parse(raw: &str, path: &Path) -> Result<Vec<Printer>, String> {
    let file: ConfigFile = serde_json::from_str(raw)
        .map_err(|e| format!("{} is not a valid printer config: {e}", path.display()))?;
    for (index, printer) in file.printers.iter().enumerate() {
        if printer.host.trim().is_empty() {
            return Err(format!(
                "{}: printer #{index} has an empty host",
                path.display()
            ));
        }
        if printer.host.chars().any(char::is_whitespace) {
            return Err(format!(
                "{}: printer #{index} host {:?} contains whitespace",
                path.display(),
                printer.host
            ));
        }
        if printer.port == 0 {
            return Err(format!(
                "{}: printer #{index} has port 0 — a printer listens on a real port (9100 \
                 for raw ZPL)",
                path.display()
            ));
        }
    }
    Ok(file.printers)
}

/// Case, surrounding whitespace, IPv6 brackets and a trailing root dot are
/// spelling, not identity: compare hosts on the spelling that matters.
fn normalise_host(host: &str) -> String {
    let trimmed = host.trim();
    let unbracketed = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(trimmed);
    unbracketed
        .strip_suffix('.')
        .unwrap_or(unbracketed)
        .to_ascii_lowercase()
}

/// Template the shell writes on first launch: a valid file with no printers.
const TEMPLATE: &str = "{\n  \"printers\": []\n}\n";

/// Write an empty config template when none exists, so the file is
/// discoverable at the path the refusal message names. Never overwrites.
pub fn ensure_config(root: &Path) -> std::io::Result<PathBuf> {
    let path = root.join(CONFIG_FILE);
    if path.exists() {
        return Ok(path);
    }
    std::fs::write(&path, TEMPLATE)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    /// A registry over a config the test writes into a scratch dir. The dir
    /// travels with the registry because every resolve re-reads the file.
    fn registry(printers: &str) -> (tempfile::TempDir, Registry) {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::write(root.path().join(CONFIG_FILE), printers).expect("write config");
        let registry = Registry::load(root.path());
        (root, registry)
    }

    #[test]
    fn a_configured_printer_is_resolved_to_its_own_spelling() {
        let (_dir, registry) = registry(r#"{"printers":[{"name":"office","host":"10.0.0.9"}]}"#);
        assert_eq!(
            registry
                .resolve(" 10.0.0.9 ", DEFAULT_PORT)
                .expect("resolve"),
            Destination {
                host: "10.0.0.9".to_string(),
                port: DEFAULT_PORT,
            }
        );
        // A caller that spells the host differently still reaches the
        // configured printer, and the resolved host is the configured one.
        assert_eq!(
            registry
                .resolve("10.0.0.9.", DEFAULT_PORT)
                .expect("resolve")
                .host,
            "10.0.0.9"
        );
    }

    #[test]
    fn a_host_that_is_not_configured_is_refused() {
        let (_dir, registry) = registry(r#"{"printers":[{"host":"10.0.0.9","port":9100}]}"#);
        let error = registry
            .resolve("192.168.1.50", 9100)
            .expect_err("must refuse");
        assert!(error.contains("not configured"), "{error}");
        assert!(error.contains("10.0.0.9:9100"), "{error}");
        assert!(error.contains(CONFIG_FILE), "{error}");
    }

    #[test]
    fn a_configured_host_on_another_port_is_refused() {
        let (_dir, registry) = registry(r#"{"printers":[{"host":"10.0.0.9","port":9100}]}"#);
        let error = registry.resolve("10.0.0.9", 9101).expect_err("must refuse");
        assert!(error.contains("10.0.0.9:9101"), "{error}");
    }

    #[test]
    fn an_absent_config_file_refuses_and_names_the_file() {
        let root = tempfile::tempdir().expect("temp dir");
        let registry = Registry::load(root.path());
        let error = registry
            .resolve("10.0.0.9", DEFAULT_PORT)
            .expect_err("must refuse");
        assert!(error.contains("no printers are configured"), "{error}");
        assert!(error.contains(CONFIG_FILE), "{error}");
    }

    #[test]
    fn a_malformed_config_file_refuses_rather_than_guessing() {
        let (_dir, registry) = registry("{ this is not json");
        let error = registry
            .resolve("10.0.0.9", DEFAULT_PORT)
            .expect_err("must refuse");
        assert!(error.contains("not a valid printer config"), "{error}");
    }

    #[test]
    fn an_entry_with_an_empty_host_is_refused_at_load() {
        let (_dir, registry) = registry(r#"{"printers":[{"host":"","port":9100}]}"#);
        let error = registry
            .resolve("10.0.0.9", DEFAULT_PORT)
            .expect_err("must refuse");
        assert!(error.contains("empty host"), "{error}");
    }

    #[test]
    fn an_entry_with_port_zero_is_refused_at_load() {
        let (_dir, registry) = registry(r#"{"printers":[{"host":"10.0.0.9","port":0}]}"#);
        let error = registry
            .resolve("10.0.0.9", DEFAULT_PORT)
            .expect_err("must refuse");
        assert!(error.contains("port 0"), "{error}");
    }

    #[test]
    fn an_empty_host_is_refused_before_the_lookup() {
        let (_dir, registry) = registry(r#"{"printers":[{"host":"10.0.0.9","port":9100}]}"#);
        let error = registry
            .resolve("   ", DEFAULT_PORT)
            .expect_err("must refuse");
        assert_eq!(error, "printer host is empty");
    }

    /// The config is the live authority, not a launch-time snapshot: an entry
    /// added while the app runs is usable on the next send, which is what the
    /// operator does after reading a refusal.
    #[test]
    fn a_printer_added_while_running_is_usable_without_a_restart() {
        let (dir, registry) = registry(r#"{"printers":[]}"#);
        assert!(registry.resolve("10.0.0.9", DEFAULT_PORT).is_err());
        std::fs::write(
            dir.path().join(CONFIG_FILE),
            r#"{"printers":[{"host":"10.0.0.9","port":9100}]}"#,
        )
        .expect("edit config");
        assert_eq!(
            registry.resolve("10.0.0.9", DEFAULT_PORT).expect("resolve"),
            Destination {
                host: "10.0.0.9".to_string(),
                port: DEFAULT_PORT,
            }
        );
    }

    #[tokio::test]
    async fn a_configured_printer_receives_the_exact_bytes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("addr");
        let printer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut socket, &mut buf).expect("read");
            buf
        });

        let (_dir, registry) = registry(&format!(
            r#"{{"printers":[{{"name":"local","host":"127.0.0.1","port":{}}}]}}"#,
            addr.port()
        ));
        let destination = registry.resolve("127.0.0.1", addr.port()).expect("resolve");
        crate::send_zpl_to(&destination.host, destination.port, b"^XA^XZ\n")
            .await
            .expect("send");
        assert_eq!(printer.join().expect("printer thread"), b"^XA^XZ\n");
    }

    #[test]
    fn ensure_config_writes_an_empty_template_once() {
        let root = tempfile::tempdir().expect("temp dir");
        let path = ensure_config(root.path()).expect("template");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), TEMPLATE);
        std::fs::write(&path, r#"{"printers":[{"host":"10.0.0.9"}]}"#).expect("edit");
        ensure_config(root.path()).expect("second call");
        assert!(
            std::fs::read_to_string(&path)
                .expect("read")
                .contains("10.0.0.9"),
            "an existing config must not be overwritten"
        );
    }

    #[test]
    fn normalise_host_treats_spelling_as_spelling() {
        assert_eq!(normalise_host(" Zebra.Office. "), "zebra.office");
        assert_eq!(normalise_host("[fe80::1]"), "fe80::1");
        assert_eq!(normalise_host("10.0.0.9."), "10.0.0.9");
    }
}
