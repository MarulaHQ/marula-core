//! core-mail — the email edge for Meridian (§4.7 traits-at-the-edge).
//!
//! Purpose: define the `MailTransport` and `ScanProvider` trait boundaries
//! every external email dependency sits behind, ship the adapters
//! (Postmark API, SMTP/dev-mailpit, recording/noop test doubles, clamd),
//! and hold the provider-agnostic inbound logic: canonical webhook payload,
//! threading keys, subject-token extraction, and outbound rendering plus
//! retry-backoff math. Also owns the wire DTOs the worker↔core-api send
//! pipeline exchanges.
//!
//! Non-purpose: no database access (persistence lives in core-db, routing
//! orchestration in core-api), no MIME parsing of full raw messages (v1
//! consumes already-extracted fields from the provider webhook), and no
//! notification policy decisions (which mail to send is core-api's call).

pub mod hmac;
pub mod inbound;
pub mod internal;
pub mod outbound;
pub mod scan;
pub mod transport;

pub use inbound::InboundEmail;
pub use scan::{ClamdScan, NoopScan, ScanError, ScanProvider, ScanVerdict};
pub use transport::recording::{NoopTransport, RecordingTransport};
pub use transport::{MailTransport, OutboundEmail, SendReceipt, TransportError};

/// The transport kind selected by configuration; `"none"` keeps mail queued
/// without ever sending (the documented none mode — nothing is silently
/// dropped, the ledger simply stays pending and logs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportKind {
    None,
    Smtp {
        host: String,
        port: u16,
        username: Option<String>,
        password: Option<String>,
    },
    Postmark {
        server_token: String,
        base_url: String,
    },
}

impl TransportKind {
    /// Parses a `(EMAIL_TRANSPORT, …)` env-shaped tuple; unknown names fall
    /// back to [`TransportKind::None`] so a typo can never exfiltrate mail.
    pub fn parse(
        name: &str,
        smtp: Option<(String, u16, Option<String>, Option<String>)>,
        postmark: Option<(String, String)>,
    ) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "smtp" => smtp.map_or(TransportKind::None, |(host, port, username, password)| {
                TransportKind::Smtp {
                    host,
                    port,
                    username,
                    password,
                }
            }),
            "postmark" => postmark.map_or(TransportKind::None, |(server_token, base_url)| {
                TransportKind::Postmark {
                    server_token,
                    base_url,
                }
            }),
            _ => TransportKind::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TransportKind::None => "none",
            TransportKind::Smtp { .. } => "smtp",
            TransportKind::Postmark { .. } => "postmark",
        }
    }
}

/// Builds the configured transport adapter. `None` maps to a logging-only
/// transport that reports success WITHOUT sending — used only where the
/// caller has separately decided sends must not happen (tests, disabled
/// deployments); production binaries should keep such mail queued instead.
#[must_use]
pub fn build_transport(kind: &TransportKind) -> std::sync::Arc<dyn MailTransport> {
    match kind {
        TransportKind::None => std::sync::Arc::new(NoopTransport),
        TransportKind::Smtp {
            host,
            port,
            username,
            password,
        } => std::sync::Arc::new(transport::smtp::SmtpTransport {
            host: host.clone(),
            port: *port,
            username: username.clone(),
            password: password.clone(),
        }),
        TransportKind::Postmark {
            server_token,
            base_url,
        } => std::sync::Arc::new(transport::postmark::PostmarkTransport {
            server_token: server_token.clone(),
            base_url: base_url.clone(),
            client: reqwest::Client::new(),
        }),
    }
}
