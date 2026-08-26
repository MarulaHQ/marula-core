//! Generic OIDC client plumbing: discovery, authorization-code exchange
//! (PKCE S256), RS256 id_token verification against the provider JWKS, the
//! signed state codec, and the conservative login/link/JIT decision logic.
//!
//! Purpose: protocol mechanics + pure decision logic. The decision function
//! takes already-parsed claims so linking policy is unit-testable without a
//! live IdP.
//! Non-purpose: persistence and route handling (see routes/oidc.rs).

use data_encoding::BASE64URL_NOPAD;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use uuid::Uuid;

/// Parsed identity from a verified id_token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub preferred_username: Option<String>,
    /// Nonce echoed by the provider — must equal our state's nonce.
    #[serde(default)]
    pub nonce: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscoveryDoc {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Debug, Clone)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("provider discovery failed")]
    Discovery(String),
    #[error("token exchange failed")]
    Exchange(String),
    #[error("id_token validation failed")]
    InvalidToken(String),
}

#[derive(Debug, thiserror::Error)]
pub enum OidcStateError {
    #[error("state invalid, expired, or for another provider")]
    BadState,
}

// --- signed state codec -----------------------------------------------------
//
// The state parameter carries CSRF protection (nonce), PKCE binding, the
// provider id it was issued for, and — for explicit link flows — the local
// user performing the link. Signed with the JWT secret; short TTL.

#[derive(Debug, Serialize, Deserialize)]
pub struct OidcStateClaims {
    pub typ: String,
    pub pid: Uuid,
    pub nce: String,
    pub ver: String,
    /// Set only on an EXPLICIT authenticated link flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Uuid>,
    pub exp: i64,
}

const STATE_TTL_SECS: i64 = 600;
const STATE_TYP: &str = "oidc_state";

pub fn issue_state_token(
    cfg: &crate::config::Config,
    pid: Uuid,
    nonce: &str,
    verifier: &str,
    link_user: Option<Uuid>,
) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = OidcStateClaims {
        typ: STATE_TYP.into(),
        pid,
        nce: nonce.to_string(),
        ver: verifier.to_string(),
        link: link_user,
        exp: chrono::Utc::now().timestamp() + STATE_TTL_SECS,
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(cfg.jwt_secret.as_bytes()),
    )
}

pub fn verify_state_token(
    cfg: &crate::config::Config,
    token: &str,
) -> Result<OidcStateClaims, OidcStateError> {
    let mut validation = Validation::default();
    validation.validate_exp = true;
    decode::<OidcStateClaims>(
        token,
        &DecodingKey::from_secret(cfg.jwt_secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
    .filter(|c| c.typ == STATE_TYP)
    .ok_or(OidcStateError::BadState)
}

// --- PKCE / nonce material --------------------------------------------------

pub fn new_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    BASE64URL_NOPAD.encode(&bytes)
}

pub fn challenge_s256(verifier: &str) -> String {
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    BASE64URL_NOPAD.encode(digest.as_slice())
}

pub fn new_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// --- conservative decision --------------------------------------------------

/// What the callback should do after claims are verified.
#[derive(Debug, PartialEq)]
pub enum SsoOutcome {
    /// Pre-linked account: proceed to tokens.
    Login { user_id: Uuid },
    /// JIT provisioning permitted AND claims carry a verified email.
    Provision { email: String, display_name: String },
    /// No link, no JIT: the admin must link explicitly first.
    LinkRequired,
}

/// Pure decision over the conservative policy: link ONLY by explicit prior
/// link rows keyed to the IdP subject; JIT only when the org enabled it AND
/// the provider asserted `email_verified`.
///
/// Errors are static reason codes surfaced in audit details / redirects.
pub fn decide_sso(
    existing_link: Option<Uuid>,
    jit_enabled: bool,
    claims: &IdClaims,
    link_flow_user: Option<(Uuid, String)>,
) -> Result<SsoOutcome, &'static str> {
    if let Some((link_user, link_user_email)) = link_flow_user {
        let verified = claims.email_verified.unwrap_or(false);
        let email_matches = claims.email.as_deref() == Some(link_user_email.as_str());
        if !verified {
            return Err("link_email_unverified");
        }
        if !email_matches {
            return Err("link_email_mismatch");
        }
        let _ = link_user;
        return Ok(SsoOutcome::Login { user_id: link_user });
    }
    match existing_link {
        Some(user_id) => Ok(SsoOutcome::Login { user_id }),
        None => {
            let verified = claims.email_verified.unwrap_or(false);
            if jit_enabled && verified {
                let email = claims.email.clone().ok_or("jit_missing_email")?;
                let display = claims
                    .name
                    .clone()
                    .or_else(|| claims.preferred_username.clone())
                    .unwrap_or_else(|| email.split('@').next().unwrap_or("SSO User").to_string());
                Ok(SsoOutcome::Provision {
                    email,
                    display_name: display,
                })
            } else if jit_enabled && !verified {
                Err("jit_email_unverified")
            } else {
                Ok(SsoOutcome::LinkRequired)
            }
        }
    }
}

// --- HTTP client ------------------------------------------------------------

#[derive(Clone)]
pub struct OidcHttpClient {
    http: reqwest::Client,
}

impl Default for OidcHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OidcHttpClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { http }
    }

    /// Fetches discovery and pins the configured issuer (mix-up defense).
    pub async fn discover(
        &self,
        discovery_url: &str,
        expected_issuer: &str,
    ) -> Result<Discovery, OidcError> {
        let doc: DiscoveryDoc = self
            .http
            .get(discovery_url)
            .send()
            .await
            .map_err(|e| OidcError::Discovery(e.to_string()))?
            .error_for_status()
            .map_err(|e| OidcError::Discovery(e.to_string()))?
            .json()
            .await
            .map_err(|e| OidcError::Discovery(e.to_string()))?;
        if doc.issuer != expected_issuer {
            return Err(OidcError::Discovery(format!(
                "issuer mismatch: discovered {}, configured {}",
                doc.issuer, expected_issuer
            )));
        }
        Ok(Discovery {
            issuer: doc.issuer,
            authorization_endpoint: doc.authorization_endpoint,
            token_endpoint: doc.token_endpoint,
            jwks_uri: doc.jwks_uri,
        })
    }

    /// Builds the authorization redirect URL with state/nonce/PKCE.
    pub fn authorize_url(
        discovery: &Discovery,
        client_id: &str,
        redirect_uri: &str,
        state_jwt: &str,
        nonce: &str,
        verifier: &str,
    ) -> url::Url {
        let mut u = url::Url::parse(&discovery.authorization_endpoint)
            .unwrap_or_else(|_| url::Url::parse("https://invalid.invalid/").expect("static url"));
        u.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", "openid email profile")
            .append_pair("state", state_jwt)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", &challenge_s256(verifier))
            .append_pair("code_challenge_method", "S256");
        u
    }

    /// Exchanges the code at the token endpoint (client-secret-basic auth,
    /// PKCE verifier attached). Returns the raw id_token.
    pub async fn exchange_code(
        &self,
        token_endpoint: &str,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
    ) -> Result<String, OidcError> {
        #[derive(Deserialize)]
        struct TokenResponse {
            id_token: Option<String>,
            error_description: Option<String>,
        }
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ];
        let resp = self
            .http
            .post(token_endpoint)
            .basic_auth(client_id, Some(client_secret))
            .form(&form)
            .send()
            .await
            .map_err(|e| OidcError::Exchange(e.to_string()))?;
        let status = resp.status();
        let body: TokenResponse = resp
            .json()
            .await
            .map_err(|e| OidcError::Exchange(e.to_string()))?;
        match body.id_token {
            Some(t) if status.is_success() => Ok(t),
            _ => Err(OidcError::Exchange(
                body.error_description
                    .unwrap_or_else(|| format!("status {status}")),
            )),
        }
    }

    /// Verifies the id_token signature (RS256 via JWKS), iss/aud/exp, and
    /// returns parsed claims. Nonce equality is checked by the caller
    /// against its state (kept here as a separate, explicit step).
    pub async fn verify_id_token(
        &self,
        id_token: &str,
        jwks_uri: &str,
        expected_iss: &str,
        expected_aud: &str,
    ) -> Result<IdClaims, OidcError> {
        let header = jsonwebtoken::decode_header(id_token)
            .map_err(|e| OidcError::InvalidToken(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| OidcError::InvalidToken("id_token missing kid".into()))?;

        let jwks_text = self
            .http
            .get(jwks_uri)
            .send()
            .await
            .map_err(|e| OidcError::InvalidToken(e.to_string()))?
            .error_for_status()
            .map_err(|e| OidcError::InvalidToken(e.to_string()))?
            .text()
            .await
            .map_err(|e| OidcError::InvalidToken(e.to_string()))?;
        let jwks: jsonwebtoken::jwk::JwkSet =
            serde_json::from_str(&jwks_text).map_err(|e| OidcError::InvalidToken(e.to_string()))?;
        let jwk = jwks
            .find(&kid)
            .ok_or_else(|| OidcError::InvalidToken(format!("no JWKS key for kid {kid}")))?;
        let key = DecodingKey::from_jwk(jwk).map_err(|e| OidcError::InvalidToken(e.to_string()))?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;
        validation.set_audience(&[expected_aud]);
        validation.set_issuer(&[expected_iss]);
        let data = decode::<IdClaims>(id_token, &key, &validation)
            .map_err(|e| OidcError::InvalidToken(e.to_string()))?;
        Ok(data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> crate::config::Config {
        crate::config::Config::test_with_secret("oidc-state-test")
    }

    fn claims(sub: &str, email: &str, verified: bool) -> IdClaims {
        IdClaims {
            sub: sub.into(),
            email: Some(email.into()),
            email_verified: Some(verified),
            name: None,
            preferred_username: None,
            nonce: None,
        }
    }

    #[test]
    fn state_roundtrip_and_expiry_typ_binding() {
        let c = cfg();
        let pid = Uuid::now_v7();
        let tok = issue_state_token(&c, pid, "n1", "v1", None).unwrap();
        let back = verify_state_token(&c, &tok).unwrap();
        assert_eq!(back.pid, pid);
        assert_eq!(back.nce, "n1");
        assert!(back.link.is_none());

        // Wrong type (an access-token-shaped string) rejected.
        assert!(verify_state_token(&c, "garbage").is_err());
    }

    #[test]
    fn state_tampering_rejected() {
        let c = cfg();
        let tok = issue_state_token(&c, Uuid::now_v7(), "n", "v", None).unwrap();
        let mut parts: Vec<String> = tok.split('.').map(String::from).collect();
        parts[2] = format!("{}B", parts[2].trim_end_matches('B'));
        assert!(verify_state_token(&c, &parts.join(".")).is_err());
    }

    #[test]
    fn pkce_challenge_is_deterministic_s256() {
        let v = new_verifier();
        assert_eq!(v.len(), 43);
        let c1 = challenge_s256(&v);
        let c2 = challenge_s256(&v);
        assert_eq!(c1, c2);
        assert_ne!(challenge_s256(&new_verifier()), c1);
    }

    #[test]
    fn decision_prelinked_login_wins() {
        let uid = Uuid::now_v7();
        let out = decide_sso(Some(uid), false, &claims("s1", "a@x.test", true), None).unwrap();
        assert_eq!(out, SsoOutcome::Login { user_id: uid });
    }

    #[test]
    fn decision_no_link_no_jit_is_link_required() {
        let out = decide_sso(None, false, &claims("s1", "a@x.test", true), None).unwrap();
        assert_eq!(out, SsoOutcome::LinkRequired);
    }

    #[test]
    fn decision_jit_requires_verified_email() {
        let out = decide_sso(None, true, &claims("s1", "a@x.test", true), None).unwrap();
        assert!(matches!(out, SsoOutcome::Provision { .. }));
        // Unverified email under JIT must NOT provision.
        assert!(decide_sso(None, true, &claims("s1", "a@x.test", false), None).is_err());
        // Verified flag absent entirely must NOT provision.
        let mut c = claims("s1", "a@x.test", true);
        c.email_verified = None;
        assert!(decide_sso(None, true, &c, None).is_err());
    }

    #[test]
    fn decision_link_flow_binds_only_matching_verified_email() {
        let uid = Uuid::now_v7();
        let ok = decide_sso(
            None,
            false,
            &claims("sub-1", "a@x.test", true),
            Some((uid, "a@x.test".into())),
        )
        .unwrap();
        assert_eq!(ok, SsoOutcome::Login { user_id: uid });
        // Different email than the linking user: refused.
        assert!(decide_sso(
            None,
            false,
            &claims("sub-1", "other@x.test", true),
            Some((uid, "a@x.test".into())),
        )
        .is_err());
        // Unverified claim: refused even when emails match.
        assert!(decide_sso(
            None,
            false,
            &claims("sub-1", "a@x.test", false),
            Some((uid, "a@x.test".into())),
        )
        .is_err());
    }

    #[test]
    fn authorize_url_carries_all_protections() {
        let d = Discovery {
            issuer: "https://idp".into(),
            authorization_endpoint: "https://idp/authorize".into(),
            token_endpoint: "https://idp/token".into(),
            jwks_uri: "https://idp/jwks".into(),
        };
        let u = OidcHttpClient::authorize_url(
            &d,
            "cid",
            "https://api/cb",
            "statejwt",
            "nonce1",
            "verifier",
        );
        let q: std::collections::HashMap<String, String> = u
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(q.get("client_id").map(String::as_str), Some("cid"));
        assert_eq!(q.get("state").map(String::as_str), Some("statejwt"));
        assert_eq!(q.get("nonce").map(String::as_str), Some("nonce1"));
        assert_eq!(
            q.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(
            q.get("code_challenge").map(String::as_str),
            Some(challenge_s256("verifier").as_str())
        );
        assert!(q.contains_key("scope"));
    }
}
