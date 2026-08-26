//! Client-portal identity + exposure entities (portal v1).
//!
//! Purpose: the serde shapes for portal users (client-scoped personnel,
//! never staff rows), their three-tier role model, and the per-board
//! client-exposure record that gates which boards' tickets are listable.
//!
//! Non-purpose: no authorization decisions here — portal role checks live
//! in `core_domain::policy`; no note visibility math — that is
//! `core_domain::notes`; no persistence.

use crate::ids::{BoardId, ClientId, OrgId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Portal role tiers. `contact` is the default read/reply tier;
/// `approver` is behaviorally identical in v1 but exists so approval
/// grants (Stage 3 quote approval / Stage 4 optional client approvals)
/// can be layered on without re-provisioning identities; `it_manager`
/// ADDITIONALLY reads `in_house_update` notes. NO tier ever reads
/// `internal` notes — enforced by `notes::visible_kinds`, tested in
/// `policy` and `notes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PortalRole {
    Contact,
    Approver,
    ItManager,
}

impl PortalRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Approver => "approver",
            Self::ItManager => "it_manager",
        }
    }

    /// The audit actor_kind this tier writes under — provenance is stored,
    /// not derived at read time.
    pub const fn actor_kind(self) -> &'static str {
        match self {
            Self::Contact => "portal_contact",
            Self::Approver => "portal_approver",
            Self::ItManager => "portal_it_manager",
        }
    }
}

impl std::str::FromStr for PortalRole {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "contact" => Ok(Self::Contact),
            "approver" => Ok(Self::Approver),
            "it_manager" => Ok(Self::ItManager),
            other => Err(crate::DomainError::InvalidPortalRole(other.to_string())),
        }
    }
}

impl std::fmt::Display for PortalRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lifecycle of a portal identity. `invited` = admin-provisioned, has not
/// logged in yet; first successful login flips to `active`. `disabled`
/// blocks login AND every authenticated portal request (checked per
/// request, never cached in the token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PortalUserStatus {
    Invited,
    Active,
    Disabled,
}

impl PortalUserStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invited => "invited",
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }
}

impl std::str::FromStr for PortalUserStatus {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "invited" => Ok(Self::Invited),
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            other => Err(crate::DomainError::InvalidPortalStatus(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PortalUser {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    pub email: String,
    pub display_name: String,
    pub role: PortalRole,
    pub status: PortalUserStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One board×client exposure row. `exposed == false` hides the board's
/// tickets from that client entirely; when true, `visible_status_ids`
/// narrows which statuses are visible (`[]` = v1 default: ALL statuses).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardPortalExposure {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    pub exposed: bool,
    pub visible_status_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
