//! JWT access-token issuing and verification (HS256 on the configured
//! secret; short TTL — the refresh session is the real credential).

use super::{AccessClaims, PortalClaims};
use crate::config::Config;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Claims of a purpose-scoped short-lived token (`typ` discriminates).
#[derive(Debug, Serialize, Deserialize)]
pub struct ScopedClaims {
    pub typ: String,
    pub sub: Uuid,
    pub org: Uuid,
    /// Purpose binding: challenge row id (empty for mfa_pending).
    pub jti: String,
    pub exp: i64,
}

const KIND_STAFF: &str = "staff";
const KIND_PORTAL: &str = "portal";

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
        kind: KIND_STAFF.into(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
}

/// Verifies signature + expiry and returns the claims. Portal-family
/// tokens are rejected HERE so staff middleware can never be tricked into
/// accepting one.
pub fn verify_access_token(config: &Config, token: &str) -> Option<AccessClaims> {
    decode::<AccessClaims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|data| data.claims)
    .filter(|claims| claims.kind != KIND_PORTAL)
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

// --- short-lived purpose-scoped tokens -------------------------------------
//
// The MFA bridge (`mfa_pending`) and WebAuthn login challenges are carried
// in signed, tightly-scoped JWTs: they prove "password succeeded for this
// user" / "this challenge was issued by us" but grant NO API access.

fn encode_scoped(
    config: &Config,
    claims: &ScopedClaims,
) -> Result<String, jsonwebtoken::errors::Error> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
}

fn decode_scoped(config: &Config, token: &str, typ: &str) -> Option<ScopedClaims> {
    let mut validation = Validation::default();
    validation.validate_exp = true;
    // Bridge tokens must die exactly at exp — no default leeway.
    validation.leeway = 0;
    decode::<ScopedClaims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|data| data.claims)
    .filter(|claims| claims.typ == typ)
}

/// Issues a token proving password success but requiring second factor.
pub fn issue_mfa_pending_token(
    config: &Config,
    user_id: Uuid,
    org_id: Uuid,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = chrono::Utc::now().timestamp();
    encode_scoped(
        config,
        &ScopedClaims {
            typ: "mfa_pending".into(),
            sub: user_id,
            org: org_id,
            jti: String::new(),
            exp: now + config.mfa_pending_ttl_secs,
        },
    )
}

/// Verifies an `mfa_pending` token; None on expiry, tampering, or wrong type.
pub fn verify_mfa_pending_token(config: &Config, token: &str) -> Option<(Uuid, Uuid)> {
    decode_scoped(config, token, "mfa_pending").map(|c| (c.sub, c.org))
}

/// Issues a token binding a WebAuthn login ceremony to its challenge row.
pub fn issue_webauthn_challenge_token(
    config: &Config,
    user_id: Uuid,
    org_id: Uuid,
    challenge_id: Uuid,
    ttl_secs: i64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = chrono::Utc::now().timestamp();
    encode_scoped(
        config,
        &ScopedClaims {
            typ: "webauthn_challenge".into(),
            sub: user_id,
            org: org_id,
            jti: challenge_id.to_string(),
            exp: now + ttl_secs,
        },
    )
}

/// Verifies a WebAuthn ceremony token → (user_id, org_id, challenge_id).
pub fn verify_webauthn_challenge_token(config: &Config, token: &str) -> Option<(Uuid, Uuid, Uuid)> {
    decode_scoped(config, token, "webauthn_challenge")
        .and_then(|c| c.jti.parse().ok().map(|jti| (c.sub, c.org, jti)))
}

// --- client-portal token family ---------------------------------------------
//
// Same signing secret as staff tokens, DIFFERENT audience: every portal
// token carries kind:"portal" and a distinct claim set (client binding +
// portal session id). Verification is family-strict in BOTH directions —
// see the tests below.

pub fn issue_portal_access_token(
    config: &Config,
    portal_user_id: Uuid,
    org_id: Uuid,
    client_id: Uuid,
    session_id: Uuid,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = chrono::Utc::now().timestamp();
    encode(
        &Header::default(),
        &PortalClaims {
            kind: KIND_PORTAL.into(),
            sub: portal_user_id,
            org: org_id,
            client: client_id,
            sid: session_id,
            exp: now + config.access_ttl_secs,
            iat: now,
        },
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
}

/// Verifies a portal access token; None for staff tokens, expired or
/// tampered ones. Returns (portal_user_id, org_id, client_id, session_id).
pub fn verify_portal_access_token(
    config: &Config,
    token: &str,
) -> Option<(Uuid, Uuid, Uuid, Uuid)> {
    decode::<PortalClaims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|data| data.claims)
    .filter(|claims| claims.kind == KIND_PORTAL)
    .map(|claims| (claims.sub, claims.org, claims.client, claims.sid))
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
            secret_key: crate::config::SecretKey([3u8; 32]),
            mfa_pending_ttl_secs: 300,
            dispatch_rate_limit_per_min: 600,
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origin: "http://localhost:3000".into(),
            public_base_url: "http://localhost:8080".into(),
            web_base_url: "http://localhost:3000".into(),
            email_inbound_hmac_secret: None,
            email_internal_token: None,
            email_transport: core_mail::TransportKind::None,
            email_scan_clamd_addr: None,
            email_from_address: "support@meridian.local".into(),
            migration_rate_limit_per_min: 600,
            email_from_name: None,
            internal_token: None,
            object_store: core_store::StoreChoice::Local("./data/objects-test".into()),
            max_upload_bytes: core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES,
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

    #[test]
    fn mfa_pending_roundtrip_and_typ_isolation() {
        let cfg = test_config();
        let (u, o) = (Uuid::now_v7(), Uuid::now_v7());
        let token = issue_mfa_pending_token(&cfg, u, o).unwrap();
        assert_eq!(verify_mfa_pending_token(&cfg, &token), Some((u, o)));
        // An access token must never satisfy the mfa_pending check.
        let access = issue_access_token(&cfg, u, o, Uuid::now_v7()).unwrap();
        assert_eq!(verify_mfa_pending_token(&cfg, &access), None);
    }

    #[test]
    fn mfa_pending_expiry_is_enforced() {
        let cfg = test_config();
        let (u, o) = (Uuid::now_v7(), Uuid::now_v7());
        let token = issue_mfa_pending_token(&cfg, u, o).unwrap();
        // Simulate expiry by verifying against a config whose clock the
        // token predates: jsonwebtoken rejects exp in the past.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let expired_cfg = Config {
            mfa_pending_ttl_secs: -1,
            ..test_config()
        };
        let stale = issue_mfa_pending_token(&expired_cfg, u, o).unwrap();
        assert_eq!(verify_mfa_pending_token(&cfg, &stale), None);
        assert_eq!(verify_mfa_pending_token(&cfg, &token), Some((u, o)));
    }

    #[test]
    fn webauthn_challenge_tokens_bind_their_row() {
        let cfg = test_config();
        let (u, o, ch) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let token = issue_webauthn_challenge_token(&cfg, u, o, ch, 300).unwrap();
        assert_eq!(
            verify_webauthn_challenge_token(&cfg, &token),
            Some((u, o, ch))
        );
        // Cross-type rejection.
        assert_eq!(verify_mfa_pending_token(&cfg, &token), None);
        // Tampered payload rejected (deterministically corrupt the
        // signature's leading character).
        let mut parts: Vec<String> = token.split('.').map(String::from).collect();
        let mut sig: Vec<u8> = parts[2].clone().into_bytes();
        sig[0] = if sig[0] == b'A' { b'B' } else { b'A' };
        parts[2] = String::from_utf8(sig).expect("base64 remains utf8");
        let tampered = parts.join(".");
        assert_eq!(verify_webauthn_challenge_token(&cfg, &tampered), None);
    }

    /// The binding v1 audience-separation rule: portal and staff tokens are
    /// signed with the same secret but NEVER verify across families — a
    /// portal token must not open staff endpoints and vice versa.
    #[test]
    fn portal_and_staff_token_families_are_strictly_separate() {
        let cfg = test_config();
        let (pu, u, o, c, s) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let portal = issue_portal_access_token(&cfg, pu, o, c, s).unwrap();
        let staff = issue_access_token(&cfg, u, o, s).unwrap();

        // Staff verification refuses the portal family…
        assert!(verify_access_token(&cfg, &portal).is_none());
        let verified_staff = verify_access_token(&cfg, &staff).expect("staff token verifies");
        assert_eq!(verified_staff.sub, u);
        assert_eq!(verified_staff.org, o);
        assert_eq!(verified_staff.sid, s);
        // …and the portal verifier refuses the staff family.
        assert_eq!(verify_portal_access_token(&cfg, &staff), None);
        assert_eq!(
            verify_portal_access_token(&cfg, &portal),
            Some((pu, o, c, s))
        );
    }
}
