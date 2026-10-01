//! Scan resolution API (Wave 32 lane 5, `p32-labels-scanning`).
//!
//! `POST /api/scan/resolve` turns ONE scanned string into ONE typed meaning,
//! and that meaning is the shared contract every scanner surface consumes —
//! the keyboard-wedge scan bar today, the mobile camera leg in Wave 33, and
//! any future automation. The rule is deliberately boring and total:
//!
//!   1. A canonical deep link (`marula://stock-item/<uuid>`,
//!      `marula://stock-location/<uuid>`, `marula://stock-serial/<uuid>`,
//!      `marula://device-asset/<uuid>` — what printed labels encode)
//!      resolves directly by id.
//!   2. Otherwise the string is treated as a human/Code128 code and matched,
//!      case-insensitively and org-scoped, in this FIXED priority order:
//!      serial number → stock item SKU or part number → location code →
//!      device asset tag. The first kind with a hit wins.
//!   3. More than one row of the winning kind comes back as `candidates` —
//!      the surface asks the user to pick. A scan never guesses between two
//!      physical items.
//!   4. Nothing matches → `kind = "unknown"` with the raw input echoed. The
//!      surface says so; it does not invent an entity.
//!
//! Resolution is READ-ONLY and has no side effects: a scan means the same
//! thing wherever it is fired. What a surface DOES with the meaning (fill a
//! PO line, add a ticket part, pick a tool to check out) is the caller's
//! typed routing, never the resolver's business.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::http::Extensions;
use axum::routing::post;
use axum::{Json, Router};
use core_domain::policy::{Action, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

const MAX_SCAN_LEN: usize = 512;
const CANDIDATE_LIMIT: i64 = 8;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Commercial,
        action,
    }
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new().route(&format!("{base}/scan/resolve"), post(resolve))
}

/// One candidate when a code matches more than one row of the same kind.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScanCandidateView {
    pub kind: String,
    #[schema(value_type = uuid::Uuid)]
    pub entity_id: Uuid,
    pub display_name: String,
    #[schema(nullable = true)]
    pub secondary: Option<String>,
}

/// The typed meaning of one scanned string. `entity_id` is the resolved
/// entity: the stock item, the serialized unit, the location, or the device
/// asset. `deep_link` is always the canonical link for that entity, so a
/// caller can print/re-show exactly what it scanned.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScanResolutionView {
    pub input: String,
    /// "stock_item" | "serialized_unit" | "stock_location" | "device_asset" | "unknown"
    pub kind: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub entity_id: Option<Uuid>,
    /// Present for stock_item and serialized_unit: the parent item.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub stock_item_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub location_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub device_asset_id: Option<Uuid>,
    pub display_name: String,
    #[schema(nullable = true)]
    pub secondary: Option<String>,
    #[schema(nullable = true)]
    pub deep_link: Option<String>,
    /// Serialized units only: the unit is a tool (`is_tool` on its item).
    pub is_tool: bool,
    /// Stock items/serials only: "serialized" | "consumable".
    #[schema(nullable = true)]
    pub item_kind: Option<String>,
    /// Serialized units only: in_stock | out | checked_out | loaned.
    #[schema(nullable = true)]
    pub serial_status: Option<String>,
    /// Populated when the winning kind matched more than one row.
    #[serde(default)]
    pub candidates: Vec<ScanCandidateView>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ScanResolveRequest {
    /// The raw scanned text (keyboard-wedge burst, camera decode, or a
    /// pasted deep link).
    pub text: String,
}

fn unknown(input: &str) -> ScanResolutionView {
    ScanResolutionView {
        input: input.to_string(),
        kind: "unknown".into(),
        entity_id: None,
        stock_item_id: None,
        serial_id: None,
        location_id: None,
        device_asset_id: None,
        display_name: String::new(),
        secondary: None,
        deep_link: None,
        is_tool: false,
        item_kind: None,
        serial_status: None,
        candidates: vec![],
    }
}

/// Parses the canonical deep-link forms this product prints. Returns the
/// purpose + id. Tolerates a trailing slash and surrounding whitespace;
/// anything else is not a deep link.
pub fn parse_deep_link(text: &str) -> Option<(&'static str, Uuid)> {
    let trimmed = text.trim().trim_end_matches('/');
    let rest = trimmed.strip_prefix("marula://")?;
    let (kind, id) = rest.split_once('/')?;
    let purpose = match kind {
        "stock-item" => "stock_item",
        "stock-location" => "stock_location",
        "stock-serial" => "serialized_unit",
        "device-asset" => "device_asset",
        "tool-kit" => "tool_kit",
        _ => return None,
    };
    let id = Uuid::parse_str(id).ok()?;
    Some((purpose, id))
}

/// POST /api/scan/resolve — resolve scanned text into a typed entity.
#[utoipa::path(operation_id = "scan_resolve", post, path = "/api/scan/resolve",
    tag = "labels", security(("bearer" = [])), request_body = ScanResolveRequest,
    responses((status = 200, body = ScanResolutionView), (status = 422)))]
pub async fn resolve(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<ScanResolveRequest>,
) -> ApiResult<Json<ScanResolutionView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return Err(ApiError::Validation("scanned text is empty".into()));
    }
    if text.chars().count() > MAX_SCAN_LEN {
        return Err(ApiError::Validation(format!(
            "scanned text is longer than {MAX_SCAN_LEN} characters"
        )));
    }
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;

    let resolution = if let Some((purpose, id)) = parse_deep_link(&text) {
        resolve_by_id(&mut tx, org_id, purpose, id, &text).await?
    } else {
        resolve_by_code(&mut tx, org_id, &text).await?
    };
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(resolution))
}

async fn resolve_by_id(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    purpose: &str,
    id: Uuid,
    input: &str,
) -> ApiResult<ScanResolutionView> {
    use core_domain::entities::deep_link;
    let mut out = unknown(input);
    match purpose {
        "stock_item" => {
            let Some(item) = core_db::labels::get_item_by_id(&mut *conn, org_id, id).await? else {
                return Ok(out);
            };
            out.kind = "stock_item".into();
            out.entity_id = Some(item.id);
            out.stock_item_id = Some(item.id);
            out.display_name = item.name.clone();
            out.secondary = item.sku.clone().or_else(|| item.part_number.clone());
            out.deep_link = Some(deep_link("stock_item", item.id));
            out.item_kind = Some(item.kind);
            out.is_tool = item.is_tool;
        }
        "stock_location" => {
            let Some(loc) = core_db::labels::get_location_by_id(&mut *conn, org_id, id).await?
            else {
                return Ok(out);
            };
            out.kind = "stock_location".into();
            out.entity_id = Some(loc.id);
            out.location_id = Some(loc.id);
            out.display_name = loc.name.clone();
            out.secondary = loc.code.clone();
            out.deep_link = Some(deep_link("stock_location", loc.id));
        }
        "serialized_unit" => {
            let Some(serial) = core_db::labels::get_serial_by_id(&mut *conn, org_id, id).await?
            else {
                return Ok(out);
            };
            out.kind = "serialized_unit".into();
            out.entity_id = Some(serial.serial_id);
            out.serial_id = Some(serial.serial_id);
            out.stock_item_id = Some(serial.stock_item_id);
            out.display_name = serial.item_name.clone();
            out.secondary = Some(serial.serial_number.clone());
            out.deep_link = Some(deep_link("serialized_unit", serial.serial_id));
            out.is_tool = serial.is_tool;
            out.item_kind = Some(serial.item_kind);
            out.serial_status = Some(serial.status);
        }
        "device_asset" => {
            let Some(asset) = core_db::labels::get_asset_by_id(&mut *conn, org_id, id).await?
            else {
                return Ok(out);
            };
            out.kind = "device_asset".into();
            out.entity_id = Some(asset.id);
            out.device_asset_id = Some(asset.id);
            out.display_name = asset.name.clone();
            out.secondary = Some(asset.tag.clone());
            out.deep_link = Some(deep_link("device_asset", asset.id));
        }
        "tool_kit" => {
            let Ok(kit) = core_db::tools::get_kit(&mut *conn, org_id, id).await else {
                return Ok(out);
            };
            out.kind = "tool_kit".into();
            out.entity_id = Some(kit.id);
            out.display_name = kit.name.clone();
            out.secondary = kit.code.clone();
            out.deep_link = Some(deep_link("tool_kit", kit.id));
        }
        _ => {}
    }
    Ok(out)
}

async fn resolve_by_code(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    text: &str,
) -> ApiResult<ScanResolutionView> {
    use core_domain::entities::deep_link;

    // 1. Serial number.
    let serials =
        core_db::labels::find_serials_by_number(&mut *conn, org_id, text, CANDIDATE_LIMIT).await?;
    if serials.len() == 1 {
        let serial = &serials[0];
        return Ok(ScanResolutionView {
            input: text.to_string(),
            kind: "serialized_unit".into(),
            entity_id: Some(serial.serial_id),
            stock_item_id: Some(serial.stock_item_id),
            serial_id: Some(serial.serial_id),
            location_id: serial.location_id,
            device_asset_id: None,
            display_name: serial.item_name.clone(),
            secondary: Some(serial.serial_number.clone()),
            deep_link: Some(deep_link("serialized_unit", serial.serial_id)),
            is_tool: serial.is_tool,
            item_kind: Some(serial.item_kind.clone()),
            serial_status: Some(serial.status.clone()),
            candidates: vec![],
        });
    }
    if serials.len() > 1 {
        let mut out = unknown(text);
        out.kind = "serialized_unit".into();
        out.candidates = serials
            .iter()
            .map(|s| ScanCandidateView {
                kind: "serialized_unit".into(),
                entity_id: s.serial_id,
                display_name: s.item_name.clone(),
                secondary: Some(s.serial_number.clone()),
            })
            .collect();
        return Ok(out);
    }

    // 2. Stock item SKU / part number.
    let items =
        core_db::labels::find_items_by_code(&mut *conn, org_id, text, CANDIDATE_LIMIT).await?;
    if items.len() == 1 {
        let item = &items[0];
        return Ok(ScanResolutionView {
            input: text.to_string(),
            kind: "stock_item".into(),
            entity_id: Some(item.id),
            stock_item_id: Some(item.id),
            serial_id: None,
            location_id: None,
            device_asset_id: None,
            display_name: item.name.clone(),
            secondary: item.sku.clone().or_else(|| item.part_number.clone()),
            deep_link: Some(deep_link("stock_item", item.id)),
            is_tool: item.is_tool,
            item_kind: Some(item.kind.clone()),
            serial_status: None,
            candidates: vec![],
        });
    }
    if items.len() > 1 {
        let mut out = unknown(text);
        out.kind = "stock_item".into();
        out.candidates = items
            .iter()
            .map(|i| ScanCandidateView {
                kind: "stock_item".into(),
                entity_id: i.id,
                display_name: i.name.clone(),
                secondary: i.sku.clone().or_else(|| i.part_number.clone()),
            })
            .collect();
        return Ok(out);
    }

    // 3. Location code.
    let locations =
        core_db::labels::find_locations_by_code(&mut *conn, org_id, text, CANDIDATE_LIMIT).await?;
    if locations.len() == 1 {
        let loc = &locations[0];
        return Ok(ScanResolutionView {
            input: text.to_string(),
            kind: "stock_location".into(),
            entity_id: Some(loc.id),
            stock_item_id: None,
            serial_id: None,
            location_id: Some(loc.id),
            device_asset_id: None,
            display_name: loc.name.clone(),
            secondary: loc.code.clone(),
            deep_link: Some(deep_link("stock_location", loc.id)),
            is_tool: false,
            item_kind: None,
            serial_status: None,
            candidates: vec![],
        });
    }
    if locations.len() > 1 {
        let mut out = unknown(text);
        out.kind = "stock_location".into();
        out.candidates = locations
            .iter()
            .map(|l| ScanCandidateView {
                kind: "stock_location".into(),
                entity_id: l.id,
                display_name: l.name.clone(),
                secondary: l.code.clone(),
            })
            .collect();
        return Ok(out);
    }

    // 4. Device asset tag.
    let assets =
        core_db::labels::find_assets_by_tag(&mut *conn, org_id, text, CANDIDATE_LIMIT).await?;
    if assets.len() == 1 {
        let asset = &assets[0];
        return Ok(ScanResolutionView {
            input: text.to_string(),
            kind: "device_asset".into(),
            entity_id: Some(asset.id),
            stock_item_id: None,
            serial_id: None,
            location_id: None,
            device_asset_id: Some(asset.id),
            display_name: asset.name.clone(),
            secondary: Some(asset.tag.clone()),
            deep_link: Some(deep_link("device_asset", asset.id)),
            is_tool: false,
            item_kind: None,
            serial_status: None,
            candidates: vec![],
        });
    }
    if assets.len() > 1 {
        let mut out = unknown(text);
        out.kind = "device_asset".into();
        out.candidates = assets
            .iter()
            .map(|a| ScanCandidateView {
                kind: "device_asset".into(),
                entity_id: a.id,
                display_name: a.name.clone(),
                secondary: Some(a.tag.clone()),
            })
            .collect();
        return Ok(out);
    }

    // 5. Toolbox short code (the Code128 band on a toolbox tag).
    let kits = core_db::tools::find_kits_by_code(&mut *conn, org_id, text, CANDIDATE_LIMIT).await?;
    if kits.len() == 1 {
        let kit = &kits[0];
        return Ok(ScanResolutionView {
            input: text.to_string(),
            kind: "tool_kit".into(),
            entity_id: Some(kit.id),
            stock_item_id: None,
            serial_id: None,
            location_id: None,
            device_asset_id: None,
            display_name: kit.name.clone(),
            secondary: kit.code.clone(),
            deep_link: Some(deep_link("tool_kit", kit.id)),
            is_tool: false,
            item_kind: None,
            serial_status: None,
            candidates: vec![],
        });
    }
    if kits.len() > 1 {
        let mut out = unknown(text);
        out.kind = "tool_kit".into();
        out.candidates = kits
            .iter()
            .map(|k| ScanCandidateView {
                kind: "tool_kit".into(),
                entity_id: k.id,
                display_name: k.name.clone(),
                secondary: k.code.clone(),
            })
            .collect();
        return Ok(out);
    }

    Ok(unknown(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_links_parse_for_every_printed_kind() {
        let id = Uuid::now_v7();
        assert_eq!(
            parse_deep_link(&format!("marula://stock-item/{id}")),
            Some(("stock_item", id))
        );
        assert_eq!(
            parse_deep_link(&format!("marula://stock-location/{id}/")),
            Some(("stock_location", id))
        );
        assert_eq!(
            parse_deep_link(&format!(" marula://stock-serial/{id} ")),
            Some(("serialized_unit", id))
        );
        assert_eq!(
            parse_deep_link(&format!("marula://device-asset/{id}")),
            Some(("device_asset", id))
        );
        assert_eq!(
            parse_deep_link(&format!("marula://tool-kit/{id}")),
            Some(("tool_kit", id))
        );
    }

    #[test]
    fn non_links_are_codes_not_errors() {
        assert_eq!(parse_deep_link("C6-1M"), None);
        assert_eq!(parse_deep_link("marula://unknown-kind/abc"), None);
        assert_eq!(parse_deep_link("https://example.com"), None);
        assert_eq!(parse_deep_link("marula://stock-item/not-a-uuid"), None);
    }
}
