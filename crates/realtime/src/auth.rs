//! WebSocket authentication via Meridian access tokens.
//!
//! Purpose: verify the same short-lived JWTs that `core-api` issues, with
//! an identical claims shape, so the hub can trust `sub`/`org` for channel
//! routing and tenancy filtering without talking to core-api.
//!
//! Two verification postures (Wave 34 lane 6, mirroring core-api):
//! EdDSA (Ed25519 public key) when the deployment configured the
//! asymmetric pair, and HS256 on the shared secret. EdDSA is preferred;
//! HS256 stays accepted while the secret is deployed, so a rolling
//! upgrade never drops a live socket handshake.
//!
//! Non-purpose: no token issuing, no refresh sessions, no session-revocation
//! lookups — expiry is enforced by signature validation only, exactly as
//! documented for the shared access-token format.

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
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
/// `None` for any invalid or expired token. The algorithm is pinned per
/// attempt (`Validation::new`), so an EdDSA token can never verify as
/// HS256 or the reverse.
pub fn verify_token(
    secret: &str,
    eddsa_public_pem: Option<&str>,
    token: &str,
) -> Option<WsClaims> {
    if let Some(pem) = eddsa_public_pem {
        if let Ok(key) = DecodingKey::from_ed_pem(pem.as_bytes()) {
            if let Ok(data) = decode::<WsClaims>(token, &key, &Validation::new(Algorithm::EdDSA)) {
                return Some(data.claims);
            }
        }
    }
    decode::<WsClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
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

    /// Throwaway Ed25519 key pair (test material only — the same pair is
    /// used in core-api's jwt tests; generated with `openssl genpkey
    /// -algorithm ED25519`).
    const TEST_EDDSA_PRIVATE: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIGls7GSdvJVi9emSo7IXPNbManY+4G8QbmS8eaXatRFV
-----END PRIVATE KEY-----";
    const TEST_EDDSA_PUBLIC: &str = "-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAHu5rvYJQ7hMNx8O1NXjxk/9o3s+yqDLRetK1HxH+ldo=
-----END PUBLIC KEY-----";

    #[test]
    fn valid_token_roundtrips_claims() {
        let c = claims(900);
        let verified = verify_token("secret", None, &mint("secret", &c)).expect("valid");
        assert_eq!(verified, c);
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let c = claims(900);
        assert!(verify_token("other", None, &mint("secret", &c)).is_none());
    }

    #[test]
    fn expired_token_is_rejected() {
        // Default validation leeway is 60s; one hour past is safely expired.
        let c = claims(-3_600);
        assert!(verify_token("secret", None, &mint("secret", &c)).is_none());
    }

    #[test]
    fn garbage_token_is_rejected() {
        assert!(verify_token("secret", None, "not-a-jwt").is_none());
    }

    #[test]
    fn eddsa_public_key_verifies_eddsa_tokens_and_keeps_hs256_fallback() {
        // EdDSA-signed token: minted with the test private key and verified
        // through the public half.
        let c = claims(900);
        let eddsa = encode(
            &Header::new(Algorithm::EdDSA),
            &c,
            &EncodingKey::from_ed_pem(TEST_EDDSA_PRIVATE.as_bytes()).expect("test key parses"),
        )
        .expect("eddsa token encodes");
        assert_eq!(
            jsonwebtoken::decode_header(&eddsa).unwrap().alg,
            Algorithm::EdDSA
        );
        let verified = verify_token("secret", Some(TEST_EDDSA_PUBLIC), &eddsa).expect("valid");
        assert_eq!(verified, c);

        // HS256 tokens minted by a not-yet-upgraded core-api still verify
        // while the shared secret is deployed.
        let legacy = mint("secret", &c);
        assert!(verify_token("secret", Some(TEST_EDDSA_PUBLIC), &legacy).is_some());

        // A malformed public key degrades to HS256-only (config validation
        // is what refuses a bad key at boot; verification never panics).
        assert!(verify_token("secret", Some("not-a-pem"), &legacy).is_some());
        assert!(verify_token("secret", Some("not-a-pem"), &eddsa).is_none());
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
