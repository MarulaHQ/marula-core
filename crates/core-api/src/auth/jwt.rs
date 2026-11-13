//! JWT access-token issuing and verification. Two signing postures
//! (threat-model §A4): HS256 on the configured secret by default, EdDSA
//! (Ed25519) with a `kid`-stamped header when the EdDSA key pair is
//! configured (Wave 34 lane 6 upgrade). Verification prefers EdDSA and
//! keeps accepting HS256 while `jwt_secret` is deployed, so a rolling
//! upgrade never invalidates live tokens; remove the secret only after
//! every issued token is EdDSA.

use super::{AccessClaims, PortalClaims};
use crate::config::Config;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
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

/// Signing header: EdDSA + `kid` when asymmetric signing is configured,
/// otherwise the HS256 default. Tokens self-describe their algorithm via
/// the `alg` header, so verification never guesses.
fn signing_header(config: &Config) -> Header {
    match &config.jwt_signing {
        Some(key) => {
            let mut header = Header::new(Algorithm::EdDSA);
            header.kid = Some(key.kid.clone());
            header
        }
        None => Header::default(),
    }
}

/// The Ed25519 PKCS#8 PEM is parsed per issuance (login/refresh paths,
/// not per request). The config layer validates it at boot, so a failure
/// here means the in-memory config was mutated after validation — the
/// error surfaces instead of quietly downgrading to HS256.
fn encoding_key(config: &Config) -> Result<EncodingKey, jsonwebtoken::errors::Error> {
    match &config.jwt_signing {
        Some(key) => EncodingKey::from_ed_pem(key.private_pem.as_bytes()),
        None => Ok(EncodingKey::from_secret(config.jwt_secret.as_bytes())),
    }
}

/// Verifies under the deployment's posture: EdDSA first when configured,
/// then HS256 while the secret remains deployed. `configure` binds the
/// algorithm into the validation (jsonwebtoken pins `alg`), so an EdDSA
/// token can never be replayed as HS256 or the reverse.
fn decode_claims<T: serde::de::DeserializeOwned>(
    config: &Config,
    token: &str,
    configure: impl Fn(Algorithm) -> Validation,
) -> Option<T> {
    if let Some(key) = &config.jwt_signing {
        if let Ok(decoding) = DecodingKey::from_ed_pem(key.public_pem.as_bytes()) {
            if let Ok(data) = decode::<T>(token, &decoding, &configure(Algorithm::EdDSA)) {
                return Some(data.claims);
            }
        }
    }
    decode::<T>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &configure(Algorithm::HS256),
    )
    .ok()
    .map(|data| data.claims)
}

/// The default validation (exp required, default leeway) bound to `alg`.
fn default_validation(algorithm: Algorithm) -> Validation {
    Validation::new(algorithm)
}

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
    encode(&signing_header(config), &claims, &encoding_key(config)?)
}

/// Verifies signature + expiry and returns the claims. Portal-family
/// tokens are rejected HERE so staff middleware can never be tricked into
/// accepting one.
pub fn verify_access_token(config: &Config, token: &str) -> Option<AccessClaims> {
    decode_claims::<AccessClaims>(config, token, default_validation)
        .filter(|claims| claims.kind != KIND_PORTAL)
}

/// Generates a cryptographically random refresh token (returned to the
/// client once; only its SHA-256 is stored).
pub fn new_refresh_token() -> String {
    use rand::TryRng;
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("OS CSPRNG unavailable");
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
    encode(&signing_header(config), claims, &encoding_key(config)?)
}

fn decode_scoped(config: &Config, token: &str, typ: &str) -> Option<ScopedClaims> {
    decode_claims::<ScopedClaims>(config, token, |algorithm| {
        let mut validation = Validation::new(algorithm);
        validation.validate_exp = true;
        // Bridge tokens must die exactly at exp — no default leeway.
        validation.leeway = 0;
        validation
    })
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
        &signing_header(config),
        &PortalClaims {
            kind: KIND_PORTAL.into(),
            sub: portal_user_id,
            org: org_id,
            client: client_id,
            sid: session_id,
            exp: now + config.access_ttl_secs,
            iat: now,
        },
        &encoding_key(config)?,
    )
}

/// Verifies a portal access token; None for staff tokens, expired or
/// tampered ones. Returns (portal_user_id, org_id, client_id, session_id).
pub fn verify_portal_access_token(
    config: &Config,
    token: &str,
) -> Option<(Uuid, Uuid, Uuid, Uuid)> {
    decode_claims::<PortalClaims>(config, token, default_validation)
        .filter(|claims| claims.kind == KIND_PORTAL)
        .map(|claims| (claims.sub, claims.org, claims.client, claims.sid))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Throwaway Ed25519 key pair A (test material only — generated with
    /// `openssl genpkey -algorithm ED25519`; never used outside tests).
    const TEST_EDDSA_PRIVATE_A: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIGls7GSdvJVi9emSo7IXPNbManY+4G8QbmS8eaXatRFV
-----END PRIVATE KEY-----";
    const TEST_EDDSA_PUBLIC_A: &str = "-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAHu5rvYJQ7hMNx8O1NXjxk/9o3s+yqDLRetK1HxH+ldo=
-----END PUBLIC KEY-----";
    /// Public half of an UNRELATED pair B (wrong-key rejection test).
    const TEST_EDDSA_PUBLIC_B: &str = "-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEACifTR92KS0nfpnia/3uCznrkBrmzfHxV+nqX3Xhxw0w=
-----END PUBLIC KEY-----";

    fn test_config() -> Config {
        Config {
            database_url: String::new(),
            listen_database_url: None,
            redis_url: String::new(),
            bind_addr: String::new(),
            jwt_secret: "test-secret".into(),
            jwt_signing: None,
            access_ttl_secs: 900,
            refresh_ttl_secs: 86_400,
            cors_origin: None,
            secret_key: crate::config::SecretKey([3u8; 32]),
            mfa_pending_ttl_secs: 300,
            dispatch_rate_limit_per_min: 600,
            scheduling_links_rate_limit_per_min: 60,
            vault_write_budget_per_min: 600,
            vault_bulk_rows_per_sec: 50,
            plugin_signing_key: None,
            plugin_signing_keys: None,
            dev_tools: false,
            routing: crate::config::RoutingChoice::None,
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origin: "http://localhost:3000".into(),
            public_base_url: "http://localhost:8080".into(),
            web_base_url: "http://localhost:3000".into(),
            email_inbound_hmac_secret: None,
            email_internal_token: None,
            serves_machine_seam: false,
            email_transport: core_mail::TransportKind::None,
            email_scan_clamd_addr: None,
            email_from_address: "support@marula.local".into(),
            migration_rate_limit_per_min: 600,
            email_from_name: None,
            internal_token: None,
            object_store: core_store::StoreChoice::Local("./data/objects-test".into()),
            max_upload_bytes: core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES,
            // Unit-test profile: no env reads, so the narrow default trust
            // set (loopback + RFC1918) is what these tokens are issued under.
            trusted_proxies: crate::client_ip::TrustedProxies::default(),
            // No env reads here either: the shipped retry preset.
            retry: core_domain::retry::RetryMatrix::from_lookup(|_| None),
            embedding: crate::config::shipped_embedding().0,
            embedding_host: crate::config::shipped_embedding().1,
            embedding_api_key: None,
            retrieval: core_domain::retrieval::RetrievalMode::Exact,
            db_pool_max: core_db::DEFAULT_POOL_MAX,
            auth_hash_max_concurrency: crate::auth::hash_gate::default_slots(),
            auth_hash_queue_wait_ms: crate::auth::hash_gate::DEFAULT_QUEUE_WAIT_MS,
            reporting_pool_max: crate::state::DEFAULT_REPORTING_POOL_MAX,
            statement_cache_capacity: core_db::DEFAULT_STATEMENT_CACHE_CAPACITY,
            ref_cache_ttl_secs: crate::cache::DEFAULT_TTL_SECS,
            ref_cache_max_entries: crate::cache::DEFAULT_MAX_ENTRIES as u32,
            all_in_one: false,
            worker_database_url: None,
            worker_bind_addr: "0.0.0.0:8082".into(),
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

    // --- EdDSA signing upgrade (Wave 34 lane 6, threat-model §A4) --------

    fn eddsa_config() -> Config {
        Config {
            jwt_signing: Some(crate::config::JwtSigningKey {
                private_pem: TEST_EDDSA_PRIVATE_A.into(),
                public_pem: TEST_EDDSA_PUBLIC_A.into(),
                kid: "m1".into(),
            }),
            ..test_config()
        }
    }

    #[test]
    fn eddsa_tokens_roundtrip_and_carry_their_kid() {
        let cfg = eddsa_config();
        let (u, o, s) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let token = issue_access_token(&cfg, u, o, s).unwrap();
        let header = jsonwebtoken::decode_header(&token).expect("header decodes");
        assert_eq!(header.alg, Algorithm::EdDSA);
        assert_eq!(header.kid.as_deref(), Some("m1"));

        // Every family self-verifies under the same key pair.
        let claims = verify_access_token(&cfg, &token).expect("access verifies");
        assert_eq!((claims.sub, claims.org, claims.sid), (u, o, s));
        let portal = issue_portal_access_token(&cfg, u, o, u, s).unwrap();
        assert_eq!(
            verify_portal_access_token(&cfg, &portal),
            Some((u, o, u, s))
        );
        let scoped = issue_mfa_pending_token(&cfg, u, o).unwrap();
        assert_eq!(verify_mfa_pending_token(&cfg, &scoped), Some((u, o)));
    }

    #[test]
    fn hs256_tokens_still_verify_while_the_secret_is_deployed() {
        // Rolling-upgrade posture: an EdDSA-configured process keeps
        // accepting HS256 tokens minted by a not-yet-upgraded instance.
        let hs_cfg = test_config();
        let eddsa_cfg = eddsa_config();
        let (u, o, s) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let legacy = issue_access_token(&hs_cfg, u, o, s).unwrap();
        assert_eq!(
            jsonwebtoken::decode_header(&legacy).unwrap().alg,
            Algorithm::HS256
        );
        assert!(
            verify_access_token(&eddsa_cfg, &legacy).is_some(),
            "legacy HS256 access token must verify during the migration"
        );
        let legacy_scoped = issue_mfa_pending_token(&hs_cfg, u, o).unwrap();
        assert_eq!(
            verify_mfa_pending_token(&eddsa_cfg, &legacy_scoped),
            Some((u, o))
        );
    }

    #[test]
    fn an_unrelated_eddsa_public_key_rejects_the_token() {
        let cfg = eddsa_config();
        let wrong = Config {
            jwt_signing: Some(crate::config::JwtSigningKey {
                private_pem: TEST_EDDSA_PRIVATE_A.into(),
                public_pem: TEST_EDDSA_PUBLIC_B.into(),
                kid: "m1".into(),
            }),
            ..test_config()
        };
        let token =
            issue_access_token(&cfg, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()).unwrap();
        assert!(verify_access_token(&wrong, &token).is_none());
        // HS256 fallback must not accept an EdDSA token either: the secret
        // is the same in both configs, but the algorithm is pinned per
        // attempt, so an EdDSA signature can never pass as HS256.
        assert!(verify_access_token(
            &Config {
                jwt_secret: cfg.jwt_secret.clone(),
                ..test_config()
            },
            &token
        )
        .is_none());
    }
}
