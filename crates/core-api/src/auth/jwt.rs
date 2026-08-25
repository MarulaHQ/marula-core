//! JWT access-token issuing and verification (HS256 on the configured
//! secret; short TTL — the refresh session is the real credential).

use super::AccessClaims;
use crate::config::Config;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use uuid::Uuid;

pub fn issue_access_token(
    config: &Config,
    user_id: Uuid,
    org_id: Uuid,
    session_id: Uuid,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = chrono::Utc::now().timestamp();
    let claims = AccessClaims {
        sub: user_id,
        org: org_id,
        sid: session_id,
        exp: now + config.access_ttl_secs,
        iat: now,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
}

/// Verifies signature + expiry and returns the claims.
pub fn verify_access_token(config: &Config, token: &str) -> Option<AccessClaims> {
    decode::<AccessClaims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|data| data.claims)
}

/// Generates a cryptographically random refresh token (returned to the
/// client once; only its SHA-256 is stored).
pub fn new_refresh_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Refresh tokens are stored hashed so a DB leak does not leak sessions.
pub fn hash_refresh_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            database_url: String::new(),
            redis_url: String::new(),
            bind_addr: String::new(),
            jwt_secret: "test-secret".into(),
            access_ttl_secs: 900,
            refresh_ttl_secs: 86_400,
            cors_origin: None,
        }
    }

    #[test]
    fn access_token_roundtrip() {
        let cfg = test_config();
        let (u, o, s) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let token = issue_access_token(&cfg, u, o, s).unwrap();
        let claims = verify_access_token(&cfg, &token).unwrap();
        assert_eq!(claims.sub, u);
        assert_eq!(claims.org, o);
        assert_eq!(claims.sid, s);
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let cfg = test_config();
        let token =
            issue_access_token(&cfg, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()).unwrap();
        let mut other = test_config();
        other.jwt_secret = "different".into();
        assert!(verify_access_token(&other, &token).is_none());
    }

    #[test]
    fn refresh_hash_is_deterministic_and_binds_token() {
        let t = new_refresh_token();
        assert_eq!(hash_refresh_token(&t), hash_refresh_token(&t));
        assert_ne!(
            hash_refresh_token(&t),
            hash_refresh_token(&new_refresh_token())
        );
    }
}
