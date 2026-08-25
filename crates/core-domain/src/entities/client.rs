//! Client hierarchy: company → site → contact, plus devices/assets.

use crate::ids::{ClientId, ContactId, DeviceAssetId, OrgId, SiteId, UserId};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Client {
    pub id: ClientId,
    pub org_id: OrgId,
    pub name: String,
    /// Optional per-client visibility: when set, only these users (plus
    /// admins) can see the client. Empty = all org members.
    pub restricted_to: Vec<UserId>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Site {
    pub id: SiteId,
    pub org_id: OrgId,
    pub client_id: ClientId,
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    /// lat/lon in decimal degrees for routing/travel estimates.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Contact {
    pub id: ContactId,
    pub org_id: OrgId,
    pub client_id: ClientId,
    pub site_id: Option<SiteId>,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub title: Option<String>,
    pub is_primary: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAsset {
    pub id: DeviceAssetId,
    pub org_id: OrgId,
    pub client_id: ClientId,
    pub site_id: Option<SiteId>,
    /// Human asset tag, e.g. "EXCH-01".
    pub tag: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
