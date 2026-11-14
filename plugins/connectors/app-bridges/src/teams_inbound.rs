//! The inbound half of the Teams bridge: Bot Framework authentication and the
//! activity shapes a press or a messaging-extension query arrives in (G7/G9).
//!
//! ## What authenticates a callback
//!
//! Teams does not sign a callback body the way Slack does. Every request from
//! the Bot Framework carries `Authorization: Bearer <JWT>`, and Microsoft's
//! documented validation is: check the signature against the key the published
//! OpenID configuration names for the token's `kid`, check `exp`/`nbf`, check
//! the issuer is the Bot Framework service or the tenant's Azure AD issuer, and
//! check the audience is **this bot's** Microsoft App ID. The `serviceurl`
//! claim must also match the activity's `serviceUrl`, which is what stops a
//! valid token being replayed at a service URL of the attacker's choosing.
//!
//! Two consequences shape this module:
//!
//! * keys are fetched over the network in production, so verification sits
//!   behind [`BotFrameworkAuth`]. The production implementation is
//!   [`OpenIdBotFrameworkAuth`]; a test injects its own and never opens a socket
//!   (R10: conformance against stubs, not live tenants);
//! * the audience is per bot, so the route resolves the candidate connection
//!   from the unauthenticated body field first and verifies with **that**
//!   connection's app id. The body field only selects a candidate; the token is
//!   what authenticates it.
//!
//! Non-purpose: no card rendering (see [`crate::card`]), no action semantics
//! (Core's registry), no outbound sending.

use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::{json, Value};

/// The issuer the Bot Framework channel service uses.
const BOT_FRAMEWORK_ISSUER: &str = "https://api.botframework.com";
/// Where Microsoft publishes the keys used to sign Bot Framework tokens.
pub const OPENID_CONFIG_URL: &str =
    "https://login.botframework.com/v1/.well-known/openidconfiguration";
/// How long fetched JWKS are trusted before a refresh.
const JWKS_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// The authenticated facts a Teams callback proves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotIdentity {
    /// The audience the token was issued for: this bot's app id.
    pub app_id: String,
    pub tenant_id: Option<String>,
    /// The service URL the token was bound to, when the token carries one.
    pub service_url: Option<String>,
}

/// Why a Bot Framework token did not verify.
///
/// Like Slack's, one variant per check and never surfaced: the route answers a
/// single typed 401 and records the variant in the ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamsAuthError {
    MissingToken,
    MalformedToken,
    /// No published key matches the token's `kid`.
    UnknownKey,
    BadSignature,
    Expired,
    WrongAudience,
    WrongIssuer,
    /// The `serviceurl` claim disagrees with the activity's `serviceUrl`.
    ServiceUrlMismatch,
    /// Keys could not be fetched (network, or the configuration changed shape).
    KeysUnavailable,
}

impl TeamsAuthError {
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::MissingToken => "teams_token_missing",
            Self::MalformedToken => "teams_token_malformed",
            Self::UnknownKey => "teams_key_unknown",
            Self::BadSignature => "teams_signature_bad",
            Self::Expired => "teams_token_expired",
            Self::WrongAudience => "teams_audience_mismatch",
            Self::WrongIssuer => "teams_issuer_mismatch",
            Self::ServiceUrlMismatch => "teams_service_url_mismatch",
            Self::KeysUnavailable => "teams_keys_unavailable",
        }
    }
}

/// Verifies one Bot Framework bearer token.
///
/// Behind a trait so a test never reaches the network: the OpenID document is
/// fetched from Microsoft in production, and R10 says a connector's contract is
/// proven against stubs.
#[async_trait]
pub trait BotFrameworkAuth: Send + Sync {
    async fn verify(
        &self,
        jwt: &str,
        expected_audience: &str,
        expected_service_url: Option<&str>,
    ) -> Result<BotIdentity, TeamsAuthError>;
}

/// The claims this bridge reads. Anything else in the token is ignored.
#[derive(Debug, Clone, Deserialize)]
struct BotClaims {
    #[serde(default)]
    aud: Audience,
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    tid: Option<String>,
    #[serde(default)]
    serviceurl: Option<String>,
}

/// `aud` is a string in Bot Framework tokens and an array in AAD v2 tokens.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
    #[default]
    Absent,
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(one) => one == expected,
            Self::Many(many) => many.iter().any(|one| one == expected),
            Self::Absent => false,
        }
    }
}

/// Production verification: OpenID configuration + JWKS over HTTPS, cached.
pub struct OpenIdBotFrameworkAuth {
    http: reqwest::Client,
    config_url: String,
    cache: std::sync::Mutex<Option<CachedJwks>>,
}

struct CachedJwks {
    text: String,
    fetched_at: std::time::Instant,
}

impl Default for OpenIdBotFrameworkAuth {
    fn default() -> Self {
        Self::new(OPENID_CONFIG_URL)
    }
}

impl OpenIdBotFrameworkAuth {
    pub fn new(config_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            config_url: config_url.into(),
            cache: std::sync::Mutex::new(None),
        }
    }

    /// The JWKS document, from cache when it is younger than [`JWKS_TTL`].
    ///
    /// The lock is never held across an await: the cached text is cloned out,
    /// then the network happens outside it.
    async fn jwks_text(&self) -> Result<String, TeamsAuthError> {
        if let Ok(guard) = self.cache.lock() {
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < JWKS_TTL {
                    return Ok(cached.text.clone());
                }
            }
        }

        let config: Value = self
            .http
            .get(&self.config_url)
            .send()
            .await
            .map_err(|_| TeamsAuthError::KeysUnavailable)?
            .error_for_status()
            .map_err(|_| TeamsAuthError::KeysUnavailable)?
            .json()
            .await
            .map_err(|_| TeamsAuthError::KeysUnavailable)?;
        let jwks_uri = config
            .get("jwks_uri")
            .and_then(Value::as_str)
            .ok_or(TeamsAuthError::KeysUnavailable)?
            .to_string();
        let text = self
            .http
            .get(&jwks_uri)
            .send()
            .await
            .map_err(|_| TeamsAuthError::KeysUnavailable)?
            .error_for_status()
            .map_err(|_| TeamsAuthError::KeysUnavailable)?
            .text()
            .await
            .map_err(|_| TeamsAuthError::KeysUnavailable)?;

        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some(CachedJwks {
                text: text.clone(),
                fetched_at: std::time::Instant::now(),
            });
        }
        Ok(text)
    }
}

#[async_trait]
impl BotFrameworkAuth for OpenIdBotFrameworkAuth {
    async fn verify(
        &self,
        jwt: &str,
        expected_audience: &str,
        expected_service_url: Option<&str>,
    ) -> Result<BotIdentity, TeamsAuthError> {
        let header = decode_header(jwt).map_err(|_| TeamsAuthError::MalformedToken)?;
        let kid = header.kid.ok_or(TeamsAuthError::MalformedToken)?;
        let text = self.jwks_text().await?;
        let jwks: jsonwebtoken::jwk::JwkSet =
            serde_json::from_str(&text).map_err(|_| TeamsAuthError::KeysUnavailable)?;
        let jwk = jwks.find(&kid).ok_or(TeamsAuthError::UnknownKey)?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| TeamsAuthError::MalformedToken)?;

        // Signature, audience and expiry are jsonwebtoken's job. Issuers are
        // two shapes (the service's, and the tenant's AAD issuer), so the set
        // is built below rather than fixed here.
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;
        validation.set_audience(&[expected_audience]);
        validation.validate_aud = true;
        validation.required_spec_claims = ["exp", "aud", "iss"]
            .into_iter()
            .map(str::to_string)
            .collect();

        let unverified_issuer = unverified_issuer(jwt).ok_or(TeamsAuthError::MalformedToken)?;
        validation.set_issuer(&[unverified_issuer.as_str()]);

        let data = decode::<BotClaims>(jwt, &key, &validation).map_err(|e| {
            use jsonwebtoken::errors::ErrorKind;
            match e.kind() {
                ErrorKind::ExpiredSignature => TeamsAuthError::Expired,
                ErrorKind::InvalidAudience => TeamsAuthError::WrongAudience,
                ErrorKind::InvalidIssuer => TeamsAuthError::WrongIssuer,
                ErrorKind::InvalidSignature => TeamsAuthError::BadSignature,
                _ => TeamsAuthError::MalformedToken,
            }
        })?;
        let claims = data.claims;

        // Issuer allowlist: the Bot Framework service, or the tenant's own AAD
        // issuer (`https://sts.windows.net/{tenant}/`). The narrow check above
        // already pinned the exact string; this refuses anything else.
        let issuer = claims.iss.as_deref().unwrap_or_default();
        let tenant_scoped = claims
            .tid
            .as_deref()
            .map(|tid| issuer == format!("https://sts.windows.net/{tid}/"))
            .unwrap_or(false);
        if issuer != BOT_FRAMEWORK_ISSUER && !tenant_scoped {
            return Err(TeamsAuthError::WrongIssuer);
        }
        if !claims.aud.contains(expected_audience) {
            return Err(TeamsAuthError::WrongAudience);
        }
        if let (Some(expected), Some(claim)) = (expected_service_url, claims.serviceurl.as_deref())
        {
            if trim_slash(expected) != trim_slash(claim) {
                return Err(TeamsAuthError::ServiceUrlMismatch);
            }
        }

        Ok(BotIdentity {
            app_id: expected_audience.to_string(),
            tenant_id: claims.tid,
            service_url: claims.serviceurl,
        })
    }
}

fn trim_slash(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// Reads `iss` without verifying anything, so the validator can be given the
/// exact issuer to require. Safe: the value only selects which issuer is
/// *demanded*, and a forged one fails the signature check that follows.
fn unverified_issuer(jwt: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Iss {
        iss: Option<String>,
    }
    let payload = jwt.split('.').nth(1)?;
    let decoded = base64url_decode(payload)?;
    serde_json::from_slice::<Iss>(&decoded).ok()?.iss
}

/// Minimal base64url decoding for the unverified middle segment.
fn base64url_decode(raw: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len() * 3 / 4 + 3);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for byte in raw.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

// ---- activity shapes -------------------------------------------------------

/// One Bot Framework activity. Only what a press or a query needs is typed.
#[derive(Debug, Clone, Deserialize)]
pub struct TeamsActivity {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "serviceUrl", default)]
    pub service_url: Option<String>,
    #[serde(default)]
    pub from: Option<ChannelAccount>,
    #[serde(default)]
    pub recipient: Option<ChannelAccount>,
    #[serde(default)]
    pub conversation: Option<ConversationAccount>,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(rename = "channelData", default)]
    pub channel_data: Option<Value>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelAccount {
    pub id: Option<String>,
    /// The AAD object id, present for a signed-in Teams user.
    #[serde(default)]
    pub aid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConversationAccount {
    pub id: Option<String>,
    #[serde(rename = "tenantId", default)]
    pub tenant_id: Option<String>,
}

impl TeamsActivity {
    /// The bot this activity is addressed to. Used only to select candidate
    /// connections; the token's audience is what authenticates it.
    pub fn bot_app_id(&self) -> Option<&str> {
        self.recipient
            .as_ref()
            .and_then(|r| r.id.as_deref())
            .filter(|id| !id.is_empty() && id.len() <= 200)
    }

    /// The vendor identity of the person who acted: the AAD object id when the
    /// client sent one, else the channel account id (`29:…`).
    ///
    /// `user_external_identities.identity_kind = 'teams_user'` stores whichever
    /// this returns, and the mapping route documents it, so a mapping cannot be
    /// half-right.
    pub fn vendor_user_id(&self) -> Option<&str> {
        self.from
            .as_ref()
            .and_then(|f| f.aid.as_deref().or(f.id.as_deref()))
    }

    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation.as_ref().and_then(|c| c.id.as_deref())
    }

    /// The action registry id this activity presses, from either documented
    /// shape: an `Action.Submit` message (`value.marulaAction`) or an
    /// `adaptiveCard/action` invoke (`value.action.data.marulaAction`).
    pub fn action_id(&self) -> Option<&str> {
        let value = self.value.as_ref()?;
        value
            .get("marulaAction")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .get("action")
                    .and_then(|a| a.get("data"))
                    .and_then(|d| d.get("marulaAction"))
                    .and_then(Value::as_str)
            })
    }

    /// True when this activity is a messaging-extension query (`G9`).
    pub fn is_compose_extension_query(&self) -> bool {
        self.kind == "invoke"
            && self
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("composeExtension/query"))
    }
}

/// The `value` of a messaging-extension query.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MessagingExtensionQuery {
    #[serde(rename = "commandId", default)]
    pub command_id: Option<String>,
    #[serde(default)]
    pub parameters: Vec<MesParameter>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MesParameter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
}

impl MessagingExtensionQuery {
    /// The ticket number the caller typed.
    ///
    /// The first parameter with a value wins: the manifest usually declares one
    /// parameter, and Teams echoes whatever order the client sent.
    pub fn ticket_number(&self) -> Option<&str> {
        self.parameters
            .iter()
            .find_map(|p| p.value.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

// ---- response shapes -------------------------------------------------------

/// A messaging-extension result carrying one card.
pub fn compose_extension_card(card: Value) -> Value {
    json!({
        "composeExtension": {
            "type": "result",
            "attachmentLayout": "list",
            "attachments": [card],
        }
    })
}

/// A messaging-extension refusal: a message, not an empty result list.
///
/// The distinction is the acceptance test for G9 — an empty `result` is a
/// silent no-op, and a `message` is something the channel can render.
pub fn compose_extension_message(text: &str) -> Value {
    json!({
        "composeExtension": { "type": "message", "text": text }
    })
}

/// A reply to an `Action.Submit` message activity.
pub fn activity_message(text: &str) -> Value {
    json!({ "statusCode": 200, "type": "message", "text": text })
}

/// A reply to an `adaptiveCard/action` invoke, which must carry the activity
/// content type rather than a plain message.
pub fn adaptive_card_action_response(body: Value) -> Value {
    json!({
        "statusCode": 200,
        "type": "application/vnd.microsoft.activity.message",
        "value": body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubAuth;

    #[async_trait]
    impl BotFrameworkAuth for StubAuth {
        async fn verify(
            &self,
            jwt: &str,
            expected_audience: &str,
            _expected_service_url: Option<&str>,
        ) -> Result<BotIdentity, TeamsAuthError> {
            if jwt == "good" {
                Ok(BotIdentity {
                    app_id: expected_audience.to_string(),
                    tenant_id: Some("tenant".into()),
                    service_url: Some("https://smba.trafficmanager.net/".into()),
                })
            } else {
                Err(TeamsAuthError::BadSignature)
            }
        }
    }

    /// The trait is the seam: a test proves the contract without a socket, and
    /// the production fetch is the only network path.
    #[tokio::test]
    async fn the_verifier_is_injectable() {
        let auth = StubAuth;
        let good = auth
            .verify("good", "app-id", Some("https://smba.trafficmanager.net/"))
            .await
            .expect("stub verifies");
        assert_eq!(good.app_id, "app-id");
        assert_eq!(
            auth.verify("bad", "app-id", None).await,
            Err(TeamsAuthError::BadSignature)
        );
    }

    /// Both documented press shapes resolve the same registry id.
    #[test]
    fn both_teams_press_shapes_resolve_one_action() {
        let submit: TeamsActivity = serde_json::from_str(
            r#"{"type":"message","value":{"marulaAction":"take"},
                "from":{"id":"29:x","aid":"aad-1"},"recipient":{"id":"bot-app"},
                "conversation":{"id":"c1"},"serviceUrl":"https://smba/"}"#,
        )
        .expect("submit activity");
        assert_eq!(submit.action_id(), Some("take"));
        assert_eq!(submit.bot_app_id(), Some("bot-app"));
        assert_eq!(submit.vendor_user_id(), Some("aad-1"));

        let invoke: TeamsActivity = serde_json::from_str(
            r#"{"type":"invoke","name":"adaptiveCard/action",
                "value":{"action":{"data":{"marulaAction":"status:in_progress"}}},
                "recipient":{"id":"bot-app"}}"#,
        )
        .expect("invoke activity");
        assert_eq!(invoke.action_id(), Some("status:in_progress"));
    }

    /// A query resolves the typed ticket number, and an invoke that is not a
    /// query does not claim to be one.
    #[test]
    fn a_messaging_extension_query_resolves_its_ticket_number() {
        let activity: TeamsActivity = serde_json::from_str(
            r#"{"type":"invoke","name":"composeExtension/query","value":{
                 "commandId":"marula","parameters":[{"name":"ticket","value":" T-1042 "}]}}"#,
        )
        .expect("query activity");
        assert!(activity.is_compose_extension_query());
        let query: MessagingExtensionQuery =
            serde_json::from_value(activity.value.clone().unwrap_or(Value::Null)).expect("query");
        assert_eq!(query.ticket_number(), Some("T-1042"));
        assert_eq!(query.command_id.as_deref(), Some("marula"));
    }

    /// A refusal is a message the channel renders, never an empty result list.
    #[test]
    fn a_refusal_is_a_message_not_an_empty_card() {
        let refusal = compose_extension_message("You cannot read that ticket.");
        assert_eq!(refusal["composeExtension"]["type"], "message");
        assert!(refusal["composeExtension"]["attachments"].is_null());
    }

    /// The unverified issuer is read from the token's middle segment without a
    /// key, which is all it is used for: choosing which issuer to demand.
    #[test]
    fn the_unverified_issuer_is_read_from_the_payload_segment() {
        let payload = json!({"iss":"https://api.botframework.com","aud":"x"});
        let encoded = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &payload,
            &jsonwebtoken::EncodingKey::from_secret(b"x"),
        )
        .expect("encode");
        // `encode` HS256s; only the segment split matters here.
        assert_eq!(
            unverified_issuer(&encoded).as_deref(),
            Some("https://api.botframework.com")
        );
    }
}
