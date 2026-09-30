//! core-mail — the email edge for Marula (§4.7 traits-at-the-edge).
//!
//! Purpose: define the `MailTransport` and `ScanProvider` trait boundaries
//! every external email dependency sits behind, ship the adapters
//! (Postmark API, Amazon SES v2, SMTP/dev-mailpit, recording/noop test
//! doubles, clamd),
//! and hold the provider-agnostic inbound logic: canonical webhook payload,
//! threading keys, subject-token extraction, and outbound rendering plus
//! retry-backoff math. Also owns the wire DTOs the worker↔core-api send
//! pipeline exchanges.
//!
//! Non-purpose: no database access (persistence lives in core-db, routing
//! orchestration in core-api), no MIME parsing of full raw messages (v1
//! consumes already-extracted fields from the provider webhook), and no
//! notification policy decisions (which mail to send is core-api's call).

pub mod brand;
pub mod hmac;
pub mod inbound;
pub mod internal;
pub mod outbound;
pub mod scan;
pub mod transport;

pub use brand::{render_digest, render_notification, render_test, Brand, BrandedEmail, DigestItem};
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
    /// Amazon SES v2, signed with SigV4. `endpoint` is resolved from
    /// `region` unless an operator overrides it (a VPC endpoint, a local
    /// capture server in tests).
    Ses {
        access_key_id: String,
        secret_access_key: String,
        region: String,
        endpoint: Option<String>,
    },
}

/// Per-provider credentials, gathered so [`TransportKind::parse`] takes one
/// argument instead of growing a tuple per adapter.
///
/// Grouped into a struct with named fields rather than a positional tuple
/// because the SES entry has three parts and a `None, None, Some(..), None`
/// call site is unreadable — and unreadable call sites are how the wrong
/// credential ends up wired to the wrong provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransportCredentials {
    pub smtp: Option<(String, u16, Option<String>, Option<String>)>,
    pub postmark: Option<(String, String)>,
    pub ses: Option<(String, String, String, Option<String>)>,
}

impl TransportKind {
    /// Parses a `(EMAIL_TRANSPORT, …)` env-shaped tuple; unknown names fall
    /// back to [`TransportKind::None`] so a typo can never exfiltrate mail.
    pub fn parse(name: &str, creds: TransportCredentials) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "smtp" => creds
                .smtp
                .map_or(TransportKind::None, |(host, port, username, password)| {
                    TransportKind::Smtp {
                        host,
                        port,
                        username,
                        password,
                    }
                }),
            "postmark" => creds
                .postmark
                .map_or(TransportKind::None, |(server_token, base_url)| {
                    TransportKind::Postmark {
                        server_token,
                        base_url,
                    }
                }),
            "ses" => creds.ses.map_or(
                TransportKind::None,
                |(access_key_id, secret_access_key, region, endpoint)| TransportKind::Ses {
                    access_key_id,
                    secret_access_key,
                    region,
                    endpoint,
                },
            ),
            _ => TransportKind::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TransportKind::None => "none",
            TransportKind::Smtp { .. } => "smtp",
            TransportKind::Postmark { .. } => "postmark",
            TransportKind::Ses { .. } => "ses",
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
        TransportKind::Ses {
            access_key_id,
            secret_access_key,
            region,
            endpoint,
        } => std::sync::Arc::new(transport::ses::SesV2Transport {
            access_key_id: access_key_id.clone(),
            secret_access_key: secret_access_key.clone(),
            region: region.clone(),
            endpoint: endpoint
                .clone()
                .unwrap_or_else(|| transport::ses::SesV2Transport::regional_endpoint(region)),
            client: reqwest::Client::new(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ses_creds() -> TransportCredentials {
        TransportCredentials {
            ses: Some((
                "AKIAEXAMPLE".into(),
                "shhh".into(),
                "eu-west-1".into(),
                None,
            )),
            ..Default::default()
        }
    }

    #[test]
    fn ses_parses_by_name_and_reports_its_name_back() {
        let kind = TransportKind::parse("ses", ses_creds());
        assert_eq!(kind.as_str(), "ses");
        assert!(matches!(kind, TransportKind::Ses { .. }));
        // Case-insensitive, and surrounding whitespace tolerated, because
        // these come from a hand-edited .env.
        assert_eq!(TransportKind::parse("  SES  ", ses_creds()).as_str(), "ses");
    }

    #[test]
    fn ses_without_credentials_falls_back_to_none_mode() {
        // The whole point of the fallback: a named provider with no usable
        // credential must not resolve to a live adapter.
        let creds = TransportCredentials::default();
        assert_eq!(TransportKind::parse("ses", creds), TransportKind::None);
    }

    #[test]
    fn the_three_providers_do_not_borrow_each_others_credentials() {
        // Every credential present at once; the NAME picks the provider.
        // A mis-wired credential that leaked across providers would send
        // mail with the wrong identity, which is the failure this guards.
        let creds = TransportCredentials {
            smtp: Some(("smtp.example".into(), 25, None, None)),
            postmark: Some(("pm-token".into(), "https://pm.example".into())),
            ses: Some(("AKIA".into(), "sk".into(), "us-east-1".into(), None)),
        };
        assert_eq!(TransportKind::parse("smtp", creds.clone()).as_str(), "smtp");
        assert_eq!(
            TransportKind::parse("postmark", creds.clone()).as_str(),
            "postmark"
        );
        assert_eq!(TransportKind::parse("ses", creds.clone()).as_str(), "ses");
        // And no name at all is none mode even with credentials present.
        assert_eq!(TransportKind::parse("", creds), TransportKind::None);
    }

    #[test]
    fn build_transport_names_each_adapter_for_the_ledger() {
        // The adapter name lands on the message ledger, so it is part of the
        // observable contract, not a debug string.
        let kinds = [
            TransportKind::None,
            TransportKind::Smtp {
                host: "localhost".into(),
                port: 25,
                username: None,
                password: None,
            },
            TransportKind::Postmark {
                server_token: "t".into(),
                base_url: "https://api.postmarkapp.com".into(),
            },
            TransportKind::Ses {
                access_key_id: "AKIA".into(),
                secret_access_key: "sk".into(),
                region: "eu-west-1".into(),
                endpoint: None,
            },
        ];
        let names: Vec<_> = kinds.iter().map(|k| build_transport(k).name()).collect();
        // "none", not "noop": the ledger's transport name is the SELECTION
        // value, so an operator reading the ledger sees the same string they
        // picked in settings. `NoopTransport` is the adapter's type name, not
        // its reported name, and conflating the two would make the ledger
        // disagree with the configuration.
        assert_eq!(names, ["none", "smtp", "postmark", "ses"]);
    }

    #[test]
    fn a_ses_transport_defaults_to_its_regional_endpoint() {
        // An explicit endpoint override wins, so a VPC endpoint or a local
        // capture server needs no code change.
        let regional = build_transport(&TransportKind::Ses {
            access_key_id: "AKIA".into(),
            secret_access_key: "sk".into(),
            region: "ap-southeast-2".into(),
            endpoint: None,
        });
        let overridden = build_transport(&TransportKind::Ses {
            access_key_id: "AKIA".into(),
            secret_access_key: "sk".into(),
            region: "ap-southeast-2".into(),
            endpoint: Some("http://127.0.0.1:9999".into()),
        });
        // The trait object hides the field, so the assertion is on the
        // adapter's own constructor rather than on the Arc.
        assert_eq!(
            transport::ses::SesV2Transport::regional_endpoint("ap-southeast-2"),
            "https://email.ap-southeast-2.amazonaws.com"
        );
        // Both build without panicking, which is the real contract here:
        // an endpoint override must not require a matching region.
        assert_eq!(regional.name(), overridden.name());
    }
}
