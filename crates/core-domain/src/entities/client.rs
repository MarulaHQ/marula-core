//! Client hierarchy: company → site → contact, plus devices/assets.

use crate::ids::{ClientId, ContactId, DeviceAssetId, DocumentLayoutId, OrgId, SiteId, UserId};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Client {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: ClientId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    /// Optional per-client visibility: when set, only these users (plus
    /// admins) can see the client. Empty = all org members.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub restricted_to: Vec<UserId>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Site {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: SiteId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
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
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: ContactId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
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
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub site_id: Option<SiteId>,
    /// Human asset tag, e.g. "EXCH-01".
    pub tag: String,
    pub name: String,
    /// Free-text connector compatibility column; typed classification
    /// lives in `layout_id` + `fields` (asset taxonomy, migration 0054).
    pub kind: String,
    pub status: String,
    /// The typed layout this asset instantiates; NULL = untyped.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub layout_id: Option<DocumentLayoutId>,
    /// Custom-field values validated against the layout's schema
    /// (docs-platform v1 shape; app-validated).
    #[serde(default)]
    pub fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
