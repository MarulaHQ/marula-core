//! App bridges (Wave 22 ecosystem stretch): outbound forwarders for
//! Slack + Microsoft Teams and the Rewst workflow trigger bridge.
//!
//! Every bridge is the SAME shape: configuration validated at the write
//! boundary (URL + secret), one typed send method, never-logging-or-
//! returning secrets, typed refusals for provider errors — and the
//! conformance suites below pin each adapter against loopback mock
//! servers (deterministic, no real network exit).
//!
//! Non-purpose: no OAuth handshakes here — the forwarders consume the
//! ALREADY-provisioned app webhook/endpoint URL from the connector
//! installation (the consent store) — app-level OAuth setup is the
//! admin's provisioning step that ends before this adapter ever runs.
pub mod rewst;
pub mod slack;
pub mod teams;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Typed outcomes of one bridge call.
#[derive(Debug, Error, PartialEq)]
pub enum BridgeError {
    /// The configured endpoint refused the message (provider text kept).
    #[error("the {0} endpoint refused the message ({1})")]
    Refused(&'static str, String),
    /// Wrong or missing configuration — never a network attempt.
    #[error("the {0} configuration is incomplete: {1}")]
    BadConfig(&'static str, String),
    /// Network failure reaching the endpoint.
    #[error("the {0} endpoint is unreachable: {1}")]
    Unreachable(&'static str, String),
}

/// A single outbound notification payload — the SAME shape for Slack
/// (chat.post-style incoming webhooks) and Teams (workflow cards), so
/// notification routes can pick adapters by kind without forking shapes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotificationPayload {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub link_url: Option<String>,
}

/// Validates one( installation's config un-DB : URL parts and secrets at
/// the write boundary. Every connector takes its own validation.
pub trait ConfigValidate {
    fn validate(self) -> Result<Self, BridgeError>
    where
        Self: Sized;
}
