//! Environment-driven configuration. Defaults suit `docker compose up`.

use core_mail::TransportKind;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub bind_addr: String,
    pub jwt_secret: String,
    /// Access token TTL in seconds (default 15 min).
    pub access_ttl_secs: i64,
    /// Refresh token TTL in seconds (default 30 days).
    pub refresh_ttl_secs: i64,
    pub cors_origin: Option<String>,
    /// HMAC shared secret for the inbound email webhook. Unset ⇒ the
    /// webhook endpoint is DISABLED (the documented none mode — never an
    /// open relay into ticketing).
    pub email_inbound_hmac_secret: Option<String>,
    /// Shared secret the worker must present on internal send-pipeline
    /// endpoints. Unset ⇒ those endpoints are disabled.
    pub email_internal_token: Option<String>,
    /// Outbound transport selection (none | smtp | postmark).
    pub email_transport: TransportKind,
    /// Malware scanner selection (none | clamd TCP).
    pub email_scan_clamd_addr: Option<String>,
    /// From-identity for Meridian-generated outbound mail.
    pub email_from_address: String,
    pub email_from_name: Option<String>,
}

/// Which malware scanner the inbound path uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanChoice {
    None,
    Clamd(String),
}

impl Config {
    #[must_use]
    pub fn scan_choice(&self) -> ScanChoice {
        match &self.email_scan_clamd_addr {
            Some(addr) => ScanChoice::Clamd(addr.clone()),
            None => ScanChoice::None,
        }
    }

    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://meridian_app:meridian_app@localhost:15432/meridian".into()
            }),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:16379/0".into()),
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            jwt_secret: std::env::var("JWT_SECRET").unwrap_or_else(|_| {
                // Dev default; compose sets a real one. Never do this in prod.
                "dev-only-insecure-secret-change-me".into()
            }),
            access_ttl_secs: std::env::var("ACCESS_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
            refresh_ttl_secs: std::env::var("REFRESH_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60 * 60 * 24 * 30),
            cors_origin: std::env::var("CORS_ORIGIN").ok(),
            email_inbound_hmac_secret: std::env::var("EMAIL_INBOUND_HMAC_SECRET").ok(),
            email_internal_token: std::env::var("EMAIL_INTERNAL_TOKEN").ok(),
            email_transport: TransportKind::parse(
                &std::env::var("EMAIL_TRANSPORT").unwrap_or_default(),
                smtp_from_env(),
                postmark_from_env(),
            ),
            email_scan_clamd_addr: match std::env::var("EMAIL_SCAN").as_deref() {
                Ok("clamd") => {
                    Some(std::env::var("CLAMD_ADDR").unwrap_or_else(|_| "127.0.0.1:3310".into()))
                }
                _ => None,
            },
            email_from_address: std::env::var("EMAIL_FROM_ADDRESS")
                .unwrap_or_else(|_| "support@meridian.local".into()),
            email_from_name: std::env::var("EMAIL_FROM_NAME").ok(),
        }
    }
}

fn smtp_from_env() -> Option<(String, u16, Option<String>, Option<String>)> {
    let host = std::env::var("SMTP_HOST").ok()?;
    let port = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(25);
    Some((
        host,
        port,
        std::env::var("SMTP_USERNAME").ok(),
        std::env::var("SMTP_PASSWORD").ok(),
    ))
}

fn postmark_from_env() -> Option<(String, String)> {
    let token = std::env::var("POSTMARK_SERVER_TOKEN").ok()?;
    Some((
        token,
        std::env::var("POSTMARK_BASE_URL").unwrap_or_else(|_| "https://api.postmarkapp.com".into()),
    ))
}
