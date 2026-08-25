//! Organization, user, and role entities.

use crate::ids::{OrgId, RoleId, SessionId, UserId};
use crate::policy::{BuiltinRole, Permission};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Org {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: OrgId,
    pub name: String,
    pub slug: String,
    /// Prefix for human ticket numbers (e.g. "MER").
    pub ticket_prefix: String,
    /// Next numeric value for ticket numbers (monotonic per org).
    pub ticket_sequence: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct User {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub email: String,
    pub display_name: String,
    pub timezone: String,
    pub is_active: bool,
    pub mfa_totp_secret: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A role: either a built-in default or a custom permission set.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Role {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: RoleId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub builtin: Option<BuiltinRole>,
    pub permissions: Vec<Permission>,
    pub is_system: bool,
}

/// Resolved identity for authorization: the user plus the union of grants
/// from every role they hold. Produced by the auth layer, consumed by `policy`.
#[derive(Debug, Clone)]
pub struct Principal {
    pub user: User,
    pub roles: Vec<Role>,
    pub permissions: Vec<Permission>,
}

impl Principal {
    pub fn is_owner_or_admin(&self) -> bool {
        self.roles.iter().any(|r| {
            matches!(
                r.builtin,
                Some(BuiltinRole::Owner) | Some(BuiltinRole::Admin)
            )
        })
    }
}

/// A refresh-token session (rotating). Access tokens are stateless JWTs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthSession {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: SessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub refresh_token_hash: String,
    pub user_agent: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}
