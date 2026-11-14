//! The inbound half of the Slack bridge: verifying a callback and reading the
//! parts of it a press needs (G7).
//!
//! Purpose: one place that knows Slack's signed-request scheme and the shape of
//! an interaction payload, so the Core route never hand-rolls either.
//!
//! ## The signature, exactly
//!
//! Slack signs `v0:{timestamp}:{raw_body}` with the app's signing secret under
//! HMAC-SHA256 and sends `X-Slack-Signature: v0=<hex>` plus
//! `X-Slack-Request-Timestamp` (epoch seconds). Two consequences decide the
//! implementation:
//!
//! * the MAC is over the **raw request bytes**. Re-serialising the JSON first
//!   would change key order, spacing and unicode escaping, so the digest would
//!   differ and the signature would be worthless as a tamper check. The caller
//!   must therefore read `Bytes` before parsing, which is why the verification
//!   function takes `&[u8]`;
//! * the timestamp is inside the signed string, so an old signature cannot be
//!   replayed with a new timestamp. It is also checked against the clock: a
//!   request more than five minutes old is refused even when the signature is
//!   genuine, because a captured body stays valid forever otherwise.
//!
//! The comparison is constant-time by construction: `hmac::Mac::verify_slice`
//! is the only comparison performed, and the length check it does internally is
//! not secret.
//!
//! ## Retries
//!
//! Slack re-delivers a callback it did not see answered within three seconds,
//! up to three times, with the same body. That is why the route's idempotency
//! key is the digest of these bytes: a retry is a replay, not a second press.
//! Nothing in this module makes that decision; it only guarantees the bytes are
//! available to it.
//!
//! Non-purpose: no action semantics (the registry lives in Core), no HTTP
//! response shapes beyond the small helpers, no OAuth.

use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Header carrying the signature. Axum lower-cases header names.
pub const SIGNATURE_HEADER: &str = "x-slack-signature";
/// Header carrying the signing timestamp (epoch seconds, as text).
pub const TIMESTAMP_HEADER: &str = "x-slack-request-timestamp";
/// How far the signed timestamp may be from now, either direction.
pub const REPLAY_WINDOW_SECS: i64 = 300;
/// The scheme prefix Slack requires. A signature without it is malformed.
const VERSION: &str = "v0";

/// Why a signature did not verify.
///
/// One variant per check, and **never** surfaced to the caller: the route
/// answers one typed 401 for all of them and records the variant in the ledger.
/// Telling an attacker which check failed is telling them what to fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackSignatureError {
    /// No `X-Slack-Request-Timestamp` header.
    MissingTimestamp,
    /// The timestamp header is not an integer.
    MalformedTimestamp,
    /// The signed timestamp is outside the replay window.
    StaleTimestamp,
    /// No `X-Slack-Signature` header.
    MissingSignature,
    /// The signature is not `v0=<hex>`, or the digest does not match.
    BadSignature,
}

impl SlackSignatureError {
    /// A stable code for the audit row.
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::MissingTimestamp => "slack_timestamp_missing",
            Self::MalformedTimestamp => "slack_timestamp_malformed",
            Self::StaleTimestamp => "slack_timestamp_stale",
            Self::MissingSignature => "slack_signature_missing",
            Self::BadSignature => "slack_signature_bad",
        }
    }
}

/// The `v0=<hex>` signature over one request, as Slack computes it.
///
/// Exposed because a test (or a fixture recorder) has to be able to produce a
/// signature without a live Slack app — that is the whole point of R10.
#[must_use]
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(format!("{VERSION}:{timestamp}:").as_bytes());
    mac.update(body);
    format!("{VERSION}={}", hex::encode(mac.finalize().into_bytes()))
}

/// Verifies one inbound callback.
///
/// `now` is passed in rather than read here so the replay window is testable
/// without waiting five minutes.
pub fn verify(
    secret: &[u8],
    timestamp_header: Option<&str>,
    signature_header: Option<&str>,
    body: &[u8],
    now: i64,
) -> Result<(), SlackSignatureError> {
    let timestamp_text = timestamp_header.ok_or(SlackSignatureError::MissingTimestamp)?;
    let timestamp: i64 = timestamp_text
        .trim()
        .parse()
        .map_err(|_| SlackSignatureError::MalformedTimestamp)?;
    // Absolute difference: a clock-skewed future timestamp is as suspect as an
    // old one, and `abs` on the subtraction cannot overflow for realistic
    // epoch seconds.
    if (now - timestamp).abs() > REPLAY_WINDOW_SECS {
        return Err(SlackSignatureError::StaleTimestamp);
    }

    let presented = signature_header.ok_or(SlackSignatureError::MissingSignature)?;
    let presented = presented.trim();
    let digest = presented
        .strip_prefix("v0=")
        .ok_or(SlackSignatureError::BadSignature)?;
    let bytes = hex::decode(digest).map_err(|_| SlackSignatureError::BadSignature)?;

    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(format!("{VERSION}:{timestamp}:").as_bytes());
    mac.update(body);
    // The only comparison performed, and it is constant-time.
    mac.verify_slice(&bytes)
        .map_err(|_| SlackSignatureError::BadSignature)
}

/// Decodes an `application/x-www-form-urlencoded` body into its pairs.
///
/// Hand-rolled because this crate must stay free of an extra dependency for
/// one decoder, and because Slack's form bodies are simple: `+` is a space, and
/// `%XX` is a byte. A pair without `=` is kept with an empty value, which is
/// how Slack sends a bare flag.
#[must_use]
pub fn parse_form(body: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(body);
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

/// The first value for a form field.
#[must_use]
pub fn form_field<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &raw[index + 1..index + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---- payload shapes --------------------------------------------------------

/// An interactive callback (`block_actions`, `view_submission`).
///
/// Only the fields a press needs are typed. Everything else is ignored rather
/// than refused: Slack adds fields without notice, and a stricter parser would
/// turn an additive vendor change into an outage.
#[derive(Debug, Clone, Deserialize)]
pub struct SlackInteraction {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub team: Option<SlackTeam>,
    #[serde(default)]
    pub user: Option<SlackUser>,
    #[serde(default)]
    pub channel: Option<SlackChannel>,
    #[serde(default)]
    pub message: Option<SlackMessage>,
    #[serde(default)]
    pub container: Option<SlackContainer>,
    #[serde(default)]
    pub actions: Vec<SlackAction>,
    #[serde(default)]
    pub trigger_id: Option<String>,
    /// Present on `view_submission`; kept so a modal can be acknowledged with
    /// the clear-the-view answer Slack expects.
    #[serde(default)]
    pub view: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackTeam {
    pub id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackUser {
    pub id: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackChannel {
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackMessage {
    pub ts: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackContainer {
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub message_ts: Option<String>,
}

/// One pressed element. `action_id` is `marula.<registry id>` for our buttons
/// and anything else for a button another app posted.
#[derive(Debug, Clone, Deserialize)]
pub struct SlackAction {
    pub action_id: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub action_ts: Option<String>,
}

impl SlackInteraction {
    /// The workspace the callback claims to come from. Used **only** to select
    /// candidate signing secrets; the signature decides whether it is believed.
    pub fn team_id(&self) -> Option<&str> {
        self.team
            .as_ref()
            .and_then(|t| t.id.as_deref())
            .filter(|id| !id.is_empty() && id.len() <= 200)
    }

    /// The pressing vendor identity (`U…`).
    pub fn user_id(&self) -> Option<&str> {
        self.user.as_ref().and_then(|u| u.id.as_deref())
    }

    /// The channel the message lives in.
    pub fn channel_id(&self) -> Option<&str> {
        self.container
            .as_ref()
            .and_then(|c| c.channel_id.as_deref())
            .or_else(|| self.channel.as_ref().and_then(|c| c.id.as_deref()))
    }

    /// The message the pressed button belongs to.
    pub fn message_ts(&self) -> Option<&str> {
        self.container
            .as_ref()
            .and_then(|c| c.message_ts.as_deref())
            .or_else(|| self.message.as_ref().and_then(|m| m.ts.as_deref()))
    }
}

/// A slash command (`/marula T-1042`), parsed from its form body.
#[derive(Debug, Clone)]
pub struct SlackCommand {
    pub team_id: String,
    pub user_id: String,
    pub channel_id: Option<String>,
    pub command: String,
    /// Everything after the command name, trimmed.
    pub text: String,
    pub trigger_id: Option<String>,
}

impl SlackCommand {
    /// Reads the fields Slack guarantees on a command request.
    ///
    /// `team_id` and `user_id` are required because without them there is no
    /// identity to resolve and no workspace to look the connection up in;
    /// their absence is a malformed request, not an authorisation answer.
    pub fn from_form(pairs: &[(String, String)]) -> Option<Self> {
        Some(Self {
            team_id: form_field(pairs, "team_id")?.to_string(),
            user_id: form_field(pairs, "user_id")?.to_string(),
            channel_id: form_field(pairs, "channel_id").map(str::to_string),
            command: form_field(pairs, "command").unwrap_or_default().to_string(),
            text: form_field(pairs, "text")
                .unwrap_or_default()
                .trim()
                .to_string(),
            trigger_id: form_field(pairs, "trigger_id").map(str::to_string),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scheme, both directions, including the exact string that is signed.
    #[test]
    fn signature_is_v0_hmac_over_timestamp_and_raw_body() {
        let signature = sign(b"secret", 1_700_000_000, b"payload=%7B%7D");
        assert!(signature.starts_with("v0="));
        assert_eq!(signature.len(), 3 + 64);
        assert_eq!(
            verify(
                b"secret",
                Some("1700000000"),
                Some(&signature),
                b"payload=%7B%7D",
                1_700_000_010
            ),
            Ok(())
        );
    }

    /// A body that has been re-serialised (spacing, key order) is a different
    /// body, and the signature says so. This is why the route reads bytes first.
    #[test]
    fn a_reserialised_body_does_not_verify() {
        let signature = sign(b"secret", 1_700_000_000, br#"{"a":1,"b":2}"#);
        assert_eq!(
            verify(
                b"secret",
                Some("1700000000"),
                Some(&signature),
                br#"{"b":2,"a":1}"#,
                1_700_000_000
            ),
            Err(SlackSignatureError::BadSignature)
        );
    }

    /// The replay window is absolute: old and future both refuse, and the
    /// boundary itself is allowed.
    #[test]
    fn the_replay_window_is_five_minutes_either_way() {
        let now = 1_700_000_000;
        for (ts, expected) in [
            (now, Ok(())),
            (now - 300, Ok(())),
            (now - 301, Err(SlackSignatureError::StaleTimestamp)),
            (now + 301, Err(SlackSignatureError::StaleTimestamp)),
        ] {
            let signature = sign(b"secret", ts, b"body");
            assert_eq!(
                verify(
                    b"secret",
                    Some(&ts.to_string()),
                    Some(&signature),
                    b"body",
                    now
                ),
                expected,
                "timestamp {ts}"
            );
        }
    }

    #[test]
    fn a_missing_or_malformed_header_is_its_own_reason() {
        assert_eq!(
            verify(b"secret", None, Some("v0=00"), b"body", 1),
            Err(SlackSignatureError::MissingTimestamp)
        );
        assert_eq!(
            verify(b"secret", Some("soon"), Some("v0=00"), b"body", 1),
            Err(SlackSignatureError::MalformedTimestamp)
        );
        assert_eq!(
            verify(b"secret", Some("1"), None, b"body", 1),
            Err(SlackSignatureError::MissingSignature)
        );
        assert_eq!(
            verify(b"secret", Some("1"), Some("v1=00"), b"body", 1),
            Err(SlackSignatureError::BadSignature)
        );
        assert_eq!(
            verify(b"secret", Some("1"), Some("v0=nothex"), b"body", 1),
            Err(SlackSignatureError::BadSignature)
        );
    }

    /// A wrong secret never verifies, and the reason does not distinguish it
    /// from a tampered body.
    #[test]
    fn a_wrong_secret_and_a_tampered_body_are_indistinguishable() {
        let signature = sign(b"secret", 1_700_000_000, b"body");
        let wrong_secret = verify(
            b"other",
            Some("1700000000"),
            Some(&signature),
            b"body",
            1_700_000_000,
        );
        let tampered = verify(
            b"secret",
            Some("1700000000"),
            Some(&signature),
            b"bodY",
            1_700_000_000,
        );
        assert_eq!(wrong_secret, Err(SlackSignatureError::BadSignature));
        assert_eq!(tampered, wrong_secret);
    }

    #[test]
    fn form_decoding_handles_plus_percent_and_a_bare_flag() {
        let pairs = parse_form(b"payload=%7B%22a%22%3A1%7D&text=hello+world&flag");
        assert_eq!(form_field(&pairs, "payload"), Some("{\"a\":1}"));
        assert_eq!(form_field(&pairs, "text"), Some("hello world"));
        assert_eq!(form_field(&pairs, "flag"), Some(""));
    }

    /// The message and channel a press belongs to come from `container` first,
    /// which is the field Slack documents for an interactive message.
    #[test]
    fn a_press_resolves_its_channel_and_message() {
        let payload: SlackInteraction = serde_json::from_str(
            r#"{"type":"block_actions","team":{"id":"T1"},"user":{"id":"U1"},
                "channel":{"id":"C-legacy"},
                "container":{"type":"message","channel_id":"C9","message_ts":"1699999999.000100"},
                "actions":[{"action_id":"marula.take","value":"take"}]}"#,
        )
        .expect("payload");
        assert_eq!(payload.team_id(), Some("T1"));
        assert_eq!(payload.user_id(), Some("U1"));
        assert_eq!(payload.channel_id(), Some("C9"));
        assert_eq!(payload.message_ts(), Some("1699999999.000100"));
        assert_eq!(payload.actions[0].action_id, "marula.take");
    }
}
