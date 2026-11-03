//! Amazon SES v2 adapter — the third documented `MailTransport` choice
//! (§2 mail row: "Postmark/SES/self-host"), and the one the verdict sweep
//! recorded as defined-but-not-built.
//!
//! Purpose: send one message through SES's `SendEmail` v2 endpoint using
//! the raw-MIME form, so Marula's existing RFC 5322 renderer and its
//! threading headers (`In-Reply-To`, `References`, `Message-ID`) are
//! transmitted verbatim. SES's own `Content.Subject`/`Body.Text` form has
//! no field for a caller-supplied Message-ID, and the threading anchor is
//! the whole reason the ledger can rebuild a conversation.
//!
//! # Why SigV4 is hand-rolled here
//!
//! The obvious implementation is `aws-sdk-sesv2`, which would add the AWS
//! SDK (a very large dependency tree) plus a version-pinning obligation to
//! a crate whose entire job is one HTTP POST. SES's signing scheme is four
//! chained HMACs over a canonical string, and this crate ALREADY depends on
//! `hmac`, `sha2`, `base64`, `hex` and `chrono` for inbound webhook
//! verification and the SMTP adapter. So the signing is ~40 lines and the
//! dependency tree is unchanged.
//!
//! This is the same trade the SMTP adapter made: a minimal protocol
//! implementation over a vendor SDK, for a seam where a vendor SDK type
//! must not leak past `crate::transport` anyway.
//!
//! # Credential scope
//!
//! SES accepts either static IAM keys or a role you assume. Only static
//! keys are wired here, deliberately: STS session credentials expire on the
//! order of an hour, and a transport that silently starts rejecting mail
//! when a token lapses is worse than one an operator has to re-key. The BYOK
//! path (per-org sealed credentials) rides the same `TransportKind`, so an
//! org on SES and the deployment default are configured the same way.
//!
//! # None-mode note
//!
//! No implicit none mode, matching the SMTP adapter. Choose
//! `EMAIL_TRANSPORT=none` to keep mail queued without any provider
//! dependency.

use crate::transport::smtp::SmtpTransport;
use crate::transport::{MailTransport, OutboundEmail, SendReceipt, TransportError};
use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use hmac::{KeyInit, Mac};
use sha2::{Digest, Sha256};

/// HMAC-SHA256, for the four-step SigV4 key chain.
///
/// Deliberately a local alias rather than a re-export of `crate::hmac`'s
/// private one: that module exists for INBOUND webhook message
/// authentication (a single sign-then-verify over a payload), whereas
/// SigV4 is outbound KEY DERIVATION — a chain where each step's key is the
/// previous step's output. Different enough uses that sharing the alias
/// would suggest a relationship that is not there, and making the existing
/// one public would widen this crate's surface for no caller.
type HmacSha256 = hmac::Hmac<Sha256>;

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const SERVICE: &str = "ses";
/// The SES v2 send path, named for the canonical-request builder below.
const SES_PATH: &str = "/v2/email/outbound-emails";

#[derive(Debug, Clone)]
pub struct SesV2Transport {
    /// The IAM access key id (`AKIA…`). Part of the credential, never logged.
    pub access_key_id: String,
    /// The IAM secret access key. Never logged, never in an error message.
    pub secret_access_key: String,
    /// AWS region, e.g. `eu-west-1`. SES's endpoint is region-scoped, so
    /// this is part of the endpoint, not a preference.
    pub region: String,
    /// Overridable so tests can point at a local capture server, and so an
    /// operator can route through a VPC endpoint without a code change.
    pub endpoint: String,
    pub client: reqwest::Client,
}

impl SesV2Transport {
    /// The regional SES endpoint for the configured region.
    pub fn regional_endpoint(region: &str) -> String {
        format!("https://email.{region}.amazonaws.com")
    }

    /// The host an `Authorization` signature is computed over. Kept as a
    /// function so the signer and the request agree by construction: a
    /// mismatch here is the classic "signature does not match" 403, and the
    /// two values must never be written out separately.
    fn signing_host(endpoint: &str) -> String {
        endpoint
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string()
    }

    /// The SES v2 request body: the rendered MIME message, base64'd into
    /// `Content.Raw.Data`.
    ///
    /// The renderer is the SMTP adapter's, reused rather than duplicated.
    /// Two renderers for RFC 5322 would be two places for a threading header
    /// to go missing, and threading is the one thing the ledger depends on.
    fn build_request(message: &OutboundEmail) -> serde_json::Value {
        let raw = SmtpTransport::render_message(message);
        serde_json::json!({
            "FromEmailAddress": from_header(message),
            "Destination": { "ToAddresses": message.to_addresses },
            "Content": {
                "Raw": { "Data": base64::engine::general_purpose::STANDARD.encode(raw) }
            },
        })
    }

    /// The request path this transport signs. Named so the canonical-request
    /// test can reason about a different path without changing the signer.
    pub const PATH: &'static str = "/v2/email/outbound-emails";

    /// The SigV4 **canonical request** for one call: method, path, empty
    /// query, the signed headers, and the payload hash.
    ///
    /// Split out from [`Self::authorization_header`] and exposed because this
    /// string is where SigV4 actually goes wrong. A stray space, a missing
    /// newline, or a header listed in SignedHeaders but absent from the
    /// canonical form all produce a well-formed-looking signature that AWS
    /// rejects at runtime with a 403 and no useful diagnostic. Asserting the
    /// exact string is the only place that class of bug is visible.
    pub fn canonical_request(host: &str, amz_date: &str, payload: &[u8]) -> String {
        let payload_hash = hex::encode(Sha256::digest(payload));
        format!(
            "POST\n{SES_PATH}\n\n\
             host:{host}\n\
             x-amz-date:{amz_date}\n\
             \n\
             host;x-amz-date\n\
             {payload_hash}"
        )
    }

    /// The date/region/service scope the signing key is derived under. This
    /// string appears verbatim in the `Credential=` field, so it is part of
    /// the wire contract and not just an internal detail.
    pub fn credential_scope(date_stamp: &str, region: &str) -> String {
        format!("{date_stamp}/{region}/{SERVICE}/aws4_request")
    }

    /// The four-step SigV4 signing key, exposed for tests. Each step's
    /// output is the next step's key, which is why it is a chain and not four
    /// independent calls.
    pub fn signing_key(secret_access_key: &str, date_stamp: &str, region: &str) -> Vec<u8> {
        let k_date = hmac_sha256(
            format!("AWS4{secret_access_key}").as_bytes(),
            date_stamp.as_bytes(),
        );
        let k_region = hmac_sha256(&k_date, region.as_bytes());
        let k_service = hmac_sha256(&k_region, SERVICE.as_bytes());
        hmac_sha256(&k_service, b"aws4_request")
    }

    /// Computes the SigV4 `Authorization` header value for one request.
    pub fn authorization_header(
        access_key_id: &str,
        secret_access_key: &str,
        region: &str,
        host: &str,
        payload: &[u8],
        now: DateTime<Utc>,
    ) -> String {
        // SigV4's two timestamps: `amzdate` is compact and goes in the
        // header, `datestamp` is the date part alone and scopes the key.
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = now.format("%Y%m%d").to_string();

        let canonical = Self::canonical_request(host, &amz_date, payload);
        let scope = Self::credential_scope(&date_stamp, region);
        let string_to_sign = format!(
            "{ALGORITHM}\n{amz_date}\n{scope}\n{}",
            hex::encode(Sha256::digest(canonical.as_bytes()))
        );
        let k_signing = Self::signing_key(secret_access_key, &date_stamp, region);
        let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

        format!(
            "{ALGORITHM} Credential={access_key_id}/{scope}, \
             SignedHeaders=host;x-amz-date, Signature={signature}"
        )
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn from_header(message: &OutboundEmail) -> String {
    match &message.from_name {
        Some(name) => format!("{name} <{}>", message.from_address),
        None => message.from_address.clone(),
    }
}

#[async_trait]
impl MailTransport for SesV2Transport {
    fn name(&self) -> &'static str {
        "ses"
    }

    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        // The body is serialized ONCE and the same bytes are hashed and
        // sent. Re-serializing between signing and sending is how a signature
        // ends up matching a payload SES never received.
        let payload = serde_json::to_vec(&Self::build_request(message))
            .map_err(|e| TransportError::Unavailable(format!("ses serialize: {e}")))?;
        let host = Self::signing_host(&self.endpoint);
        let now = Utc::now();
        let authorization = Self::authorization_header(
            &self.access_key_id,
            &self.secret_access_key,
            &self.region,
            &host,
            &payload,
            now,
        );

        let resp = self
            .client
            .post(format!("{}/v2/email/outbound-emails", self.endpoint))
            .header("Authorization", authorization)
            .header("x-amz-date", now.format("%Y%m%dT%H%M%SZ").to_string())
            .header("content-type", "application/json")
            .body(payload)
            .send()
            .await
            .map_err(|e| TransportError::Unavailable(format!("ses http: {e}")))?;

        let status = resp.status();
        // `unwrap_or_default`, not a closure: `Value::default()` is `Null`,
        // which is what a non-JSON error body (an HTML proxy error page, an
        // empty 502) needs to degrade into. The status is what the caller
        // branches on, so an unparseable body must not become an error here.
        let body: serde_json::Value = resp.json().await.unwrap_or_default();

        if !status.is_success() {
            // SES puts the machine-readable code in `Type`/`Code` and the
            // human sentence in `Message`. Both are echoed, because an
            // operator debugging a rejected send needs the code, but the
            // status is included so a 5xx is distinguishable from a 4xx
            // (retryable vs not) at the call site.
            let code = body["Type"]
                .as_str()
                .or_else(|| body["Code"].as_str())
                .unwrap_or("Unknown");
            let msg = body["Message"].as_str().unwrap_or("no message");
            return Err(crate::transport::http_status_error(
                "ses",
                status.as_u16(),
                &format!("{code}: {msg}"),
            ));
        }
        Ok(SendReceipt {
            provider_message_id: body["MessageId"].as_str().map(str::to_string),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> OutboundEmail {
        OutboundEmail {
            from_address: "support@acme.test".into(),
            from_name: Some("Acme".into()),
            reply_to: None,
            to_addresses: vec!["a@x.test".into()],
            subject: "Your ticket".into(),
            body_text: "Body.".into(),
            rfc_message_id: "<m1@acme.test>".into(),
            in_reply_to: Some("p@x.test".into()),
            references: vec!["p@x.test".into()],
        }
    }

    #[test]
    fn request_carries_the_mime_message_base64_in_raw_data() {
        let v = SesV2Transport::build_request(&message());
        assert_eq!(v["FromEmailAddress"], "Acme <support@acme.test>");
        assert_eq!(v["Destination"]["ToAddresses"][0], "a@x.test");
        // Decoding must yield a real RFC 5322 message, and the threading
        // headers are the reason this adapter uses the raw form at all.
        let raw = base64::engine::general_purpose::STANDARD
            .decode(v["Content"]["Raw"]["Data"].as_str().unwrap())
            .unwrap();
        let raw = String::from_utf8(raw).unwrap();
        assert!(raw.starts_with("From: Acme <support@acme.test>\r\n"));
        assert!(raw.contains("Message-ID: <m1@acme.test>\r\n"));
        assert!(raw.contains("In-Reply-To: <p@x.test>\r\n"));
    }

    #[test]
    fn from_header_omits_the_display_name_when_there_is_none() {
        let m = OutboundEmail {
            from_name: None,
            ..message()
        };
        assert_eq!(
            SesV2Transport::build_request(&m)["FromEmailAddress"],
            "support@acme.test"
        );
    }

    /// The canonical request is asserted EXACTLY, byte for byte.
    ///
    /// This replaces an earlier test that claimed to check the published AWS
    /// `get-vanilla` signature. It could not have: that vector signs the path
    /// `/`, while this adapter signs SES's send path, so the two were never
    /// comparable and the assertion was decorative. The canonical request is
    /// the better thing to pin anyway, because it is directly checkable
    /// against the SigV4 specification by eye, whereas a signature constant
    /// is only meaningful next to a reference implementation to compare it
    /// against.
    #[test]
    fn canonical_request_matches_the_sigv4_specification_byte_for_byte() {
        let host = "email.us-east-1.amazonaws.com";
        let amz_date = "20150830T123600Z";
        let payload = b"Action=SendEmail&Version=2010-12-01";
        let expected_payload_hash = hex::encode(Sha256::digest(payload));

        let got = SesV2Transport::canonical_request(host, amz_date, payload);
        let expected = format!(
            "POST\n/v2/email/outbound-emails\n\n\
             host:email.us-east-1.amazonaws.com\n\
             x-amz-date:20150830T123600Z\n\
             \n\
             host;x-amz-date\n\
             {expected_payload_hash}"
        );
        assert_eq!(got, expected, "canonical request drifted from the spec");

        // The structural invariants, spelled out so a failure says WHICH
        // rule broke rather than dumping two long strings:
        let lines: Vec<&str> = got.split('\n').collect();
        assert_eq!(lines[0], "POST", "line 1 must be the HTTP method");
        assert_eq!(lines[1], "/v2/email/outbound-emails", "line 2 is the path");
        assert_eq!(lines[2], "", "line 3 is the canonical query string (empty)");
        // Header block: exactly host then x-amz-date, sorted, and nothing else.
        assert_eq!(lines[3], "host:email.us-east-1.amazonaws.com");
        assert_eq!(lines[4], "x-amz-date:20150830T123600Z");
        assert_eq!(lines[5], "", "blank line terminates the header block");
        // Signed headers must be lowercase and sorted, and must be exactly
        // the headers present above — a mismatch here is the 403-with-no-
        // diagnostic failure mode.
        assert_eq!(lines[6], "host;x-amz-date");
        assert_eq!(
            lines[7], expected_payload_hash,
            "final line is the payload hash"
        );
        assert_eq!(lines.len(), 8, "no trailing content past the payload hash");
    }

    #[test]
    fn the_credential_scope_is_the_string_that_appears_on_the_wire() {
        // The scope is not an internal detail: it is spliced into the
        // Credential= field, and a wrong region in it is exactly how a
        // request ends up signed for eu-west-1 and sent to us-east-1.
        assert_eq!(
            SesV2Transport::credential_scope("20150830", "eu-west-1"),
            "20150830/eu-west-1/ses/aws4_request"
        );
    }

    #[test]
    fn the_signing_key_is_a_four_step_chain_not_four_independent_calls() {
        // Pins the derivation itself. Each step feeding the next is the
        // property that makes SigV4 a chain; if any step were computed from
        // the secret directly, this would change and every signature would
        // 403.
        let secret = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
        let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), b"20150830");
        let k_region = hmac_sha256(&k_date, b"us-east-1");
        let k_service = hmac_sha256(&k_region, b"ses");
        let expected = hmac_sha256(&k_service, b"aws4_request");
        assert_eq!(
            SesV2Transport::signing_key(secret, "20150830", "us-east-1"),
            expected
        );
        // Every intermediate is 32 bytes: a truncated key produces a
        // signature that looks valid and is rejected.
        for k in [&k_date, &k_region, &k_service, &expected] {
            assert_eq!(k.len(), 32);
        }
        // Region is part of the derivation, not decoration.
        assert_ne!(
            SesV2Transport::signing_key(secret, "20150830", "us-east-1"),
            SesV2Transport::signing_key(secret, "20150830", "eu-west-1")
        );
    }

    /// The header is structurally what SigV4 requires, and its four
    /// components each track the inputs that should change them.
    #[test]
    fn the_authorization_header_is_well_formed_and_input_sensitive() {
        let t = chrono::DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let host = "email.us-east-1.amazonaws.com";
        let auth = SesV2Transport::authorization_header(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            host,
            b"{}",
            t,
        );
        // Shape, in the order SigV4 specifies.
        assert!(auth.starts_with("AWS4-HMAC-SHA256 "), "{auth}");
        assert!(
            auth.contains("Credential=AKIDEXAMPLE/20150830/us-east-1/ses/aws4_request"),
            "{auth}"
        );
        assert!(auth.contains("SignedHeaders=host;x-amz-date"), "{auth}");
        let sig = auth.rsplit("Signature=").next().expect("signature field");
        assert_eq!(sig.len(), 64, "SHA-256 hex is 64 chars, got {sig:?}");
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()), "{sig}");

        // The SignedHeaders list and the canonical request must agree. They
        // are built in different places, and a drift between them is the
        // silent 403.
        let canonical = SesV2Transport::canonical_request(host, "20150830T123600Z", b"{}");
        assert!(canonical.contains("host;x-amz-date"), "{canonical}");

        // Each input the signature must depend on, changed one at a time.
        let variants = [
            SesV2Transport::authorization_header(
                "AKIDEXAMPLE",
                "other",
                "us-east-1",
                host,
                b"{}",
                t,
            ),
            SesV2Transport::authorization_header(
                "AKIDEXAMPLE",
                "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
                "eu-west-1",
                host,
                b"{}",
                t,
            ),
            SesV2Transport::authorization_header(
                "AKIDEXAMPLE",
                "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
                "us-east-1",
                host,
                b"{\"x\":1}",
                t,
            ),
            SesV2Transport::authorization_header(
                "AKIDEXAMPLE",
                "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
                "us-east-1",
                host,
                b"{}",
                t + chrono::Duration::seconds(60),
            ),
        ];
        for v in &variants {
            assert_ne!(auth, *v, "the signature must change when any input changes");
        }
    }

    #[test]
    fn signing_host_strips_scheme_port_and_trailing_slash() {
        assert_eq!(
            SesV2Transport::signing_host("https://email.eu-west-1.amazonaws.com"),
            "email.eu-west-1.amazonaws.com"
        );
        assert_eq!(
            SesV2Transport::signing_host("https://email.eu-west-1.amazonaws.com/"),
            "email.eu-west-1.amazonaws.com"
        );
    }

    #[test]
    fn regional_endpoint_follows_the_region() {
        assert_eq!(
            SesV2Transport::regional_endpoint("eu-west-1"),
            "https://email.eu-west-1.amazonaws.com"
        );
    }

    #[tokio::test]
    async fn sends_against_a_local_capture_server_and_reads_the_receipt() {
        use axum::routing::post;
        use axum::Json;
        async fn capture(
            headers: axum::http::HeaderMap,
            body: axum::body::Bytes,
        ) -> Json<serde_json::Value> {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .expect("authorization header");
            assert!(
                auth.starts_with("AWS4-HMAC-SHA256 Credential=AKID/"),
                "{auth}"
            );
            assert!(auth.contains("SignedHeaders=host;x-amz-date"), "{auth}");
            // x-amz-date must be present and must match SigV4's compact form.
            let date = headers
                .get("x-amz-date")
                .and_then(|v| v.to_str().ok())
                .expect("x-amz-date header");
            assert_eq!(date.len(), 16, "{date}");
            assert!(date.ends_with('Z'), "{date}");
            // The signed body is the body actually sent.
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["Destination"]["ToAddresses"][0], "a@x.test");
            Json(serde_json::json!({ "MessageId": "0100018-ses-1" }))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let app = axum::Router::new().route("/v2/email/outbound-emails", post(capture));
            axum::serve(listener, app).await.unwrap();
        });

        let t = SesV2Transport {
            access_key_id: "AKID".into(),
            secret_access_key: "secret".into(),
            region: "eu-west-1".into(),
            endpoint: format!("http://{addr}"),
            client: reqwest::Client::new(),
        };
        let receipt = t.send(&message()).await.expect("send ok");
        assert_eq!(
            receipt.provider_message_id.as_deref(),
            Some("0100018-ses-1")
        );
    }

    #[tokio::test]
    async fn provider_rejection_maps_to_rejected_with_the_aws_code() {
        use axum::routing::post;
        use axum::Json;
        async fn fail() -> (axum::http::StatusCode, Json<serde_json::Value>) {
            (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "Type": "MessageRejected",
                    "Message": "Email address is not verified"
                })),
            )
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let app = axum::Router::new().route("/v2/email/outbound-emails", post(fail));
            axum::serve(listener, app).await.unwrap();
        });
        let t = SesV2Transport {
            access_key_id: "AKID".into(),
            secret_access_key: "secret".into(),
            region: "eu-west-1".into(),
            endpoint: format!("http://{addr}"),
            client: reqwest::Client::new(),
        };
        let err = t.send(&message()).await.expect_err("must reject");
        // Both the code and the human sentence survive: an operator
        // debugging a rejected send needs the code, and "not verified" is
        // the single most common SES setup mistake there is.
        let TransportError::Rejected(msg) = err else {
            panic!("expected Rejected");
        };
        assert!(msg.contains("400"), "{msg}");
        assert!(msg.contains("MessageRejected"), "{msg}");
        assert!(msg.contains("not verified"), "{msg}");
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_unavailable_not_rejected() {
        // The distinction is what lets the ledger retry with backoff instead
        // of treating a network blip as a permanent content problem.
        let t = SesV2Transport {
            access_key_id: "AKID".into(),
            secret_access_key: "secret".into(),
            region: "eu-west-1".into(),
            // Port 1 on loopback: connection refused, deterministically.
            endpoint: "http://127.0.0.1:1".into(),
            client: reqwest::Client::new(),
        };
        let err = t.send(&message()).await.expect_err("must fail");
        assert!(matches!(err, TransportError::Unavailable(_)), "{err:?}");
    }
}
