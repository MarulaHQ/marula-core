//! Authentication: password hashing (argon2id), JWT access tokens, rotating
//! refresh sessions with chain reuse detection, TOTP + WebAuthn MFA, and
//! the request middleware that resolves a Principal.

pub mod handlers;
pub mod jwt;
pub mod mfa_handlers;
pub mod middleware;
pub mod oidc_client;
pub mod password;
pub mod totp_service;
pub mod webauthn_service;

use core_domain::entities::{PortalUser, Principal};
use uuid::Uuid;

/// Claims carried by the short-lived access token. Stateless: the middleware
/// still loads fresh roles/permissions from the DB per request.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct AccessClaims {
    /// User id.
    pub sub: Uuid,
    /// Org id (single-tenant deployments have exactly one).
    pub org: Uuid,
    /// Auth session id — lets logout revoke outstanding access tokens'
    /// refresh lineage.
    pub sid: Uuid,
    pub exp: i64,
    pub iat: i64,
    /// Token-family discriminant: `"staff"` on newly issued access tokens
    /// (absent = legacy pre-portal token), `"portal"` on client-portal
    /// tokens. Verification REJECTS cross-family use both ways, so a
    /// portal token can never satisfy a staff endpoint or vice versa —
    /// even though both are signed with the same secret.
    #[serde(default)]
    pub kind: String,
}

/// Claims of a client-portal access token (`kind:"portal"`). Separate
/// struct + verifier from [`AccessClaims`]: the two families share only
/// the signing secret, never an audience.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PortalClaims {
    pub kind: String,
    /// Portal user id (a `portal_users.id`, NOT a `users.id`).
    pub sub: Uuid,
    pub org: Uuid,
    /// The caller's client — every portal query is additionally scoped by it.
    pub client: Uuid,
    /// Portal session id (a `portal_sessions.id`).
    pub sid: Uuid,
    pub exp: i64,
    pub iat: i64,
}

/// The authenticated PORTAL request context extracted by the portal
/// middleware. Deliberately a different type from `AuthedUser`: Axum
/// extension lookup by type means a handler taking `PortalAuthed` can
/// never receive staff credentials.
#[derive(Debug, Clone)]
pub struct PortalAuthed {
    /// Freshly loaded per request — disabling a portal user applies to
    /// outstanding access tokens immediately.
    pub user: PortalUser,
    pub session_id: Option<Uuid>,
}

/// The authenticated request context extracted by the middleware.
#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub principal: Principal,
    /// Session id from the access token (`sid`) when present.
    pub session_id: Option<Uuid>,
}

impl std::ops::Deref for AuthedUser {
    type Target = Principal;
    fn deref(&self) -> &Principal {
        &self.principal
    }
}
