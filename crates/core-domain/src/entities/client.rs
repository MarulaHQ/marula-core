//! Client hierarchy: company → site → contact, plus devices/assets.

use crate::ids::{ClientId, ContactId, DeviceAssetId, DocumentLayoutId, OrgId, SiteId, UserId};
use chrono::{DateTime, Datelike, NaiveDate, Utc};

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
    /// CI5 tiers generalized to contacts (ADR 0003 extension): observed
    /// feed-written fields, human-curated sticky fields, per-field source
    /// provenance. Empty maps for pre-0101 rows.
    #[cfg_attr(feature = "openapi", schema(value_type = serde_json::Value))]
    #[serde(default)]
    pub observed_fields: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = serde_json::Value))]
    #[serde(default)]
    pub curated_fields: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = serde_json::Value))]
    #[serde(default)]
    pub field_sources: serde_json::Value,
    /// Merge tombstone: when set, this row was folded into the target and
    /// only its audit row keeps it queryable.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    #[serde(default)]
    pub merged_into_id: Option<ContactId>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAsset {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// NULL for internal-org assets; the DB CHECK (migration 0103) ties
    /// `scope` and `client_id` together structurally.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    /// client | internal_org (migration 0103): the MSP's OWN equipment
    /// rides the SAME DeviceAsset lifecycle under an org-scoped row —
    /// client_id NULL, never a fake client record.
    #[serde(default = "default_asset_scope")]
    pub scope: String,
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
    /// FX conversion snapshot (ADR 0002 lock moment: inventory receipt) —
    /// locked once when the purchase cost is written in a non-base
    /// currency; immutable afterwards (DB trigger enforced).
    #[serde(default)]
    pub purchase_fx_rate_used: Option<String>,
    #[serde(default)]
    pub purchase_fx_rate_date: Option<NaiveDate>,
    #[serde(default)]
    pub purchase_fx_source: Option<String>,
    #[serde(default)]
    pub base_purchase_cost_minor: Option<i64>,
    /// none | straight_line | declining_balance.
    #[serde(default = "default_depreciation_method")]
    pub depreciation_method: String,
    /// The warranty horizon, derived from this row's own columns through
    /// `asset_warranty_from_fields` at read time. It travels WITH the asset so
    /// no surface has to re-derive it from `purchase_date` plus
    /// `useful_life_months` — that duplication is how a ledger and a scan
    /// sheet end up disagreeing about which side of the window an asset sits
    /// on. NULL when the asset carries nothing to derive a horizon from.
    #[cfg_attr(feature = "openapi", schema(nullable = true))]
    pub warranty: Option<WarrantyWindow>,
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

/// Asset scopes (migration 0103): client-scoped rows carry a client;
/// internal_org rows are the MSP's own fleet with client_id NULL.
pub const ASSET_SCOPES: [&str; 2] = ["client", "internal_org"];

pub fn is_valid_asset_scope(scope: &str) -> bool {
    ASSET_SCOPES.contains(&scope.trim())
}

fn default_asset_scope() -> String {
    "client".into()
}

fn default_depreciation_method() -> String {
    "none".into()
}

/// How close to its end a warranty is before a surface should call it out.
/// The same window the assets page filters on, now shared by the API so a
/// scan-resolved asset and the assets list cannot disagree about "ending".
pub const WARRANTY_ENDING_WINDOW_DAYS: i64 = 90;

/// Which side of the warranty horizon an asset sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WarrantyState {
    /// Inside the horizon, beyond the callout window: covered, nothing to say.
    Active,
    /// Inside the horizon, within `WARRANTY_ENDING_WINDOW_DAYS` of its end.
    Ending,
    /// Past the horizon.
    Ended,
}

impl WarrantyState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Ending => "ending",
            Self::Ended => "ended",
        }
    }
}

/// The warranty horizon for one device asset: the end date, which side of it
/// today falls, and how many whole days are left (negative once ended).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WarrantyWindow {
    pub state: WarrantyState,
    pub ends_on: NaiveDate,
    pub days_remaining: i64,
}

/// The `warranty_end` taxonomy field's date, when it holds a parseable ISO
/// date. A hand-edited or unparsable value reads as no explicit field rather
/// than failing the whole read that touched it.
pub fn parse_warranty_end(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()
}

/// The warranty horizon of a device asset, or `None` when the asset carries
/// nothing to derive one from. The explicit taxonomy field wins — a human
/// typed that date into the asset's `warranty_end` field (`standard_layouts`,
/// `crates/core-db/src/document_layouts.rs`) — and the purchase date plus
/// useful life is the fallback, the same derivation the assets page performs
/// client-side. `today` is passed in so the rule is deterministic under test.
pub fn asset_warranty(
    explicit_end: Option<NaiveDate>,
    purchase_date: Option<NaiveDate>,
    useful_life_months: Option<i32>,
    today: NaiveDate,
) -> Option<WarrantyWindow> {
    let ends_on = match explicit_end {
        Some(end) => end,
        None => {
            let purchase_date = purchase_date?;
            let life = useful_life_months?;
            if life <= 0 {
                return None;
            }
            add_months_clamped(purchase_date, life as u32)
        }
    };
    let days_remaining = (ends_on - today).num_days();
    let state = if days_remaining < 0 {
        WarrantyState::Ended
    } else if days_remaining <= WARRANTY_ENDING_WINDOW_DAYS {
        WarrantyState::Ending
    } else {
        WarrantyState::Active
    };
    Some(WarrantyWindow {
        state,
        ends_on,
        days_remaining,
    })
}

/// The warranty horizon carried by a device asset's own row, so every read of
/// that row — the scan resolver, the ledger list, the record page — answers
/// from the same rule instead of re-deriving it client-side.
pub fn asset_warranty_from_fields(
    fields: &serde_json::Value,
    purchase_date: Option<NaiveDate>,
    useful_life_months: Option<i32>,
    today: NaiveDate,
) -> Option<WarrantyWindow> {
    let explicit = fields
        .get("warranty_end")
        .and_then(|value| value.as_str())
        .and_then(parse_warranty_end);
    asset_warranty(explicit, purchase_date, useful_life_months, today)
}

/// Adds whole calendar months, clamping the day to the target month's length
/// so a Jan-31 anchor lands on Feb-28 rather than overflowing into March.
/// Same rule as `core_billing::periods::add_months`.
fn add_months_clamped(date: NaiveDate, months: u32) -> NaiveDate {
    let total = date.year() * 12 + (date.month0() as i32) + months as i32;
    let year = total.div_euclid(12);
    let month0 = total.rem_euclid(12) as u32;
    let day = date.day();
    let days_in_month = days_in_month(year, month0 + 1);
    NaiveDate::from_ymd_opt(year, month0 + 1, day.min(days_in_month))
        .expect("clamped y/m/d is always a valid date")
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_explicit_warranty_end_wins_over_the_purchase_horizon() {
        let window = asset_warranty(
            Some(date(2027, 3, 1)),
            Some(date(2020, 1, 15)),
            Some(24),
            date(2026, 10, 12),
        );
        assert_eq!(
            window,
            Some(WarrantyWindow {
                state: WarrantyState::Active,
                ends_on: date(2027, 3, 1),
                days_remaining: 140,
            })
        );
    }

    #[test]
    fn a_derived_horizon_inside_the_window_reads_ending() {
        let window = asset_warranty(None, Some(date(2026, 9, 1)), Some(2), date(2026, 10, 12));
        assert_eq!(
            window.map(|w| (w.state, w.ends_on, w.days_remaining)),
            Some((WarrantyState::Ending, date(2026, 11, 1), 20))
        );
    }

    #[test]
    fn a_past_horizon_reads_ended_with_negative_days() {
        let window = asset_warranty(
            Some(date(2026, 10, 1)),
            Some(date(2026, 1, 1)),
            Some(24),
            date(2026, 10, 12),
        );
        assert_eq!(
            window.map(|w| (w.state, w.ends_on, w.days_remaining)),
            Some((WarrantyState::Ended, date(2026, 10, 1), -11))
        );
    }

    #[test]
    fn an_asset_without_purchase_data_has_no_horizon() {
        assert_eq!(
            asset_warranty(None, None, Some(24), date(2026, 10, 12)),
            None
        );
        assert_eq!(
            asset_warranty(None, Some(date(2026, 1, 1)), None, date(2026, 10, 12)),
            None
        );
        // A zero or negative useful life derives nothing rather than a nonsense date.
        assert_eq!(
            asset_warranty(None, Some(date(2026, 1, 1)), Some(0), date(2026, 10, 12)),
            None
        );
    }

    #[test]
    fn a_row_with_only_the_taxonomy_field_still_has_a_horizon() {
        // No purchase date, no useful life: the typed field alone answers.
        let fields = serde_json::json!({ "warranty_end": "2026-11-01" });
        let window = asset_warranty_from_fields(&fields, None, None, date(2026, 10, 12));
        assert_eq!(
            window.map(|w| (w.state, w.ends_on, w.days_remaining)),
            Some((WarrantyState::Ending, date(2026, 11, 1), 20))
        );
    }

    #[test]
    fn the_taxonomy_field_beats_a_purchase_horizon_that_would_disagree() {
        // Purchase says 24 months from 2026-01-01 (2028-01-01); the typed
        // field says 2026-12-01. The human-entered date wins, and a surface
        // reading the row cannot accidentally see the other one.
        let fields = serde_json::json!({ "warranty_end": "2026-12-01" });
        let window = asset_warranty_from_fields(
            &fields,
            Some(date(2026, 1, 1)),
            Some(24),
            date(2026, 10, 12),
        );
        assert_eq!(
            window.map(|w| (w.state, w.ends_on)),
            Some((WarrantyState::Ending, date(2026, 12, 1)))
        );
    }

    #[test]
    fn a_broken_taxonomy_field_falls_back_to_the_purchase_horizon() {
        // A hand-edited field that is not an ISO date degrades to the
        // fallback rather than failing the whole read that touched it.
        let fields = serde_json::json!({ "warranty_end": "next tuesday" });
        let window = asset_warranty_from_fields(
            &fields,
            Some(date(2026, 8, 1)),
            Some(2),
            date(2026, 10, 12),
        );
        assert_eq!(
            window.map(|w| (w.state, w.ends_on, w.days_remaining)),
            Some((WarrantyState::Ended, date(2026, 10, 1), -11))
        );
    }

    #[test]
    fn an_empty_fields_object_has_horizon_only_from_the_purchase() {
        let window = asset_warranty_from_fields(
            &serde_json::json!({}),
            Some(date(2026, 9, 1)),
            Some(2),
            date(2026, 10, 12),
        );
        assert_eq!(
            window.map(|w| (w.state, w.ends_on, w.days_remaining)),
            Some((WarrantyState::Ending, date(2026, 11, 1), 20))
        );
    }

    #[test]
    fn warranty_end_parses_only_an_exact_iso_date() {
        assert_eq!(
            parse_warranty_end("2026-11-01"),
            Some(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap())
        );
        assert_eq!(
            parse_warranty_end(" 2026-11-01 "),
            parse_warranty_end("2026-11-01")
        );
        assert_eq!(parse_warranty_end("2026-11-01T00:00:00Z"), None);
        assert_eq!(parse_warranty_end("01/11/2026"), None);
        assert_eq!(parse_warranty_end(""), None);
    }

    #[test]
    fn month_addition_clamps_to_the_shorter_month() {
        assert_eq!(add_months_clamped(date(2026, 1, 31), 1), date(2026, 2, 28));
        assert_eq!(add_months_clamped(date(2024, 1, 31), 1), date(2024, 2, 29));
        assert_eq!(add_months_clamped(date(2026, 8, 15), 3), date(2026, 11, 15));
        assert_eq!(add_months_clamped(date(2026, 12, 1), 1), date(2027, 1, 1));
    }
}
