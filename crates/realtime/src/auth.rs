//! WebSocket authentication via Meridian access tokens.
//!
//! Purpose: verify the same short-lived HS256 JWTs that `core-api` issues,
//! with an identical claims shape, so the hub can trust `sub`/`org` for
//! channel routing and tenancy filtering without talking to core-api.
//!
//! Non-purpose: no token issuing, no refresh sessions, no session-revocation
//! lookups — expiry is enforced by signature validation only, exactly as
//! documented for the shared access-token format.

use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Claims carried by a Meridian access token. Field names mirror
/// `core_api::auth::AccessClaims` byte-for-byte (`sub`, `org`, `sid`,
/// `exp`, `iat`); duplicated here on purpose so this crate never depends
/// on the HTTP API crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsClaims {
    /// User id.
    pub sub: Uuid,
    /// Org id (single-tenant deployments have exactly one).
    pub org: Uuid,
    /// Auth session id that issued this token.
    pub sid: Uuid,
    /// Expiry, seconds since epoch.
    pub exp: i64,
    /// Issued-at, seconds since epoch.
    pub iat: i64,
}

/// Verifies signature and expiry, returning the claims on success and
/// `None` for any invalid or expired token.
pub fn verify_token(secret: &str, token: &str) -> Option<WsClaims> {
    decode::<WsClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|data| data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn claims(exp_in_secs: i64) -> WsClaims {
        let now = chrono::Utc::now().timestamp();
        WsClaims {
            sub: Uuid::now_v7(),
            org: Uuid::now_v7(),
            sid: Uuid::now_v7(),
            exp: now + exp_in_secs,
            iat: now,
        }
    }

    fn mint(secret: &str, c: &WsClaims) -> String {
        encode(
            &Header::default(),
            c,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("test token encodes")
    }

    #[test]
    fn valid_token_roundtrips_claims() {
        let c = claims(900);
        let verified = verify_token("secret", &mint("secret", &c)).expect("valid");
        assert_eq!(verified, c);
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let c = claims(900);
        assert!(verify_token("other", &mint("secret", &c)).is_none());
    }

    #[test]
    fn expired_token_is_rejected() {
        // Default validation leeway is 60s; one hour past is safely expired.
        let c = claims(-3_600);
        assert!(verify_token("secret", &mint("secret", &c)).is_none());
    }

    #[test]
    fn garbage_token_is_rejected() {
        assert!(verify_token("secret", "not-a-jwt").is_none());
    }

    /// Mechanical guard that our claim field names stay identical to the
    /// tokens core-api mints (see core_api::auth::AccessClaims).
    #[test]
    fn claim_field_names_match_core_api_shape() {
        let value = serde_json::to_value(claims(60)).expect("serializes");
        let keys: Vec<&str> = value
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(keys.len(), 5);
        for key in ["exp", "iat", "org", "sid", "sub"] {
            assert!(keys.contains(&key), "missing claim key {key}");
        }
    }
}
