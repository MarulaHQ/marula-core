//! Client hierarchy: company → site → contact, plus devices/assets.

use crate::ids::{ClientId, ContactId, DeviceAssetId, DocumentLayoutId, OrgId, SiteId, UserId};
use chrono::{DateTime, NaiveDate, Utc};

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
    /// (docs-platform v1 shape; app-validated). The EFFECTIVE view:
    /// curated wins over observed (CI5, migration 0061).
    #[serde(default)]
    pub fields: serde_json::Value,
    // ---- inventory lifecycle (migration 0061) ---------------------------
    /// owned | leased | rented.
    #[serde(default = "default_ownership")]
    pub ownership: String,
    #[serde(default)]
    pub rental_vendor: Option<String>,
    #[serde(default)]
    pub rental_start: Option<NaiveDate>,
    #[serde(default)]
    pub rental_end: Option<NaiveDate>,
    #[serde(default)]
    pub rental_cost_minor: Option<i64>,
    #[serde(default)]
    pub rental_currency: Option<String>,
    /// monthly | quarterly | annual.
    #[serde(default)]
    pub rental_cycle: Option<String>,
    #[serde(default)]
    pub purchase_date: Option<NaiveDate>,
    #[serde(default)]
    pub purchase_cost_minor: Option<i64>,
    #[serde(default)]
    pub purchase_currency: Option<String>,
    /// none | straight_line | declining_balance.
    #[serde(default = "default_depreciation_method")]
    pub depreciation_method: String,
    #[serde(default)]
    pub useful_life_months: Option<i32>,
    #[serde(default)]
    pub salvage_value_minor: Option<i64>,
    #[serde(default)]
    pub depreciation_rate_bps: Option<i32>,
    // ---- configuration identity (CI1/CI3/CI5, migration 0061) -----------
    /// CI3 deterministic identity keys (source GUID, serial, MACs, ...).
    #[serde(default)]
    pub identity_keys: serde_json::Value,
    /// CI5: feed-written tier; churns silently with its source.
    #[serde(default)]
    pub observed_fields: serde_json::Value,
    /// CI5: human-set tier; sticky forever, never auto-overwritten.
    #[serde(default)]
    pub curated_fields: serde_json::Value,
    /// CI5: per observed field `{source_system, source_link_id, observed_at}`.
    #[serde(default)]
    pub field_sources: serde_json::Value,
    /// CI4: non-NULL means this row was absorbed by the target (tombstone).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    #[serde(default)]
    pub merged_into_id: Option<DeviceAssetId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_ownership() -> String {
    "owned".into()
}

fn default_depreciation_method() -> String {
    "none".into()
}
