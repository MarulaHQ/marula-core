//! Authentication: password hashing (argon2id), JWT access tokens, rotating
//! refresh sessions, and the request middleware that resolves a Principal.

pub mod handlers;
pub mod jwt;
pub mod middleware;
pub mod password;

use core_domain::entities::Principal;
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
}

/// The authenticated request context extracted by the middleware.
#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub principal: Principal,
}

impl std::ops::Deref for AuthedUser {
    type Target = Principal;
    fn deref(&self) -> &Principal {
        &self.principal
    }
}
