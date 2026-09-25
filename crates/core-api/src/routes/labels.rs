//! Label templates + label rendering API (Wave 32 lane 5,
//! `p32-labels-scanning`; migration 0105).
//!
//! Public surface:
//!   * `/label-templates` CRUD — typed, org-scoped print recipes per
//!     PURPOSE (stock item / location bin / serialized unit / loan
//!     device), rendered under the settings IA. Why not a settings key:
//!     the registry carries three scalar payload types by doctrine
//!     ("richer structures are entities"), and a template is an ordered
//!     token list + size + symbology config.
//!   * `/label-templates/preview` — render a DRAFT template against
//!     deterministic sample values (the designer's live preview; one
//!     renderer, so the preview cannot drift from the print).
//!   * `/labels/render` — resolve a REAL entity and render the label the
//!     two targets consume: browser print geometry (`label`) and raw ZPL
//!     (`zpl`, for the desktop shell's Zebra transport).
//!
//! The QR payload is always the entity's canonical deep link
//! (`meridian://<kind>/<uuid>`) — the SAME string the scan bar resolves, so
//! a printed label scans back to exactly the entity it names.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::http::Extensions;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use core_domain::entities::{Actor, AuditEvent, LabelTemplate};
use core_domain::labels::{render_label, to_zpl, LabelInput, ResolvedLabel};
use core_domain::policy::{Action, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const TEMPLATE_ENTITY: &str = "label_template";
const LIST_LIMIT: i64 = 200;
const SAMPLE_MARKER: Uuid = Uuid::nil();

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Admin,
        action,
    }
}

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Commercial,
        action,
    }
}

fn assets(action: Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Clients,
        action,
    }
}

/// Render permission follows the ENTITY being labelled.
fn require_render_permission(
    principal: &core_domain::entities::Principal,
    purpose: &str,
) -> ApiResult<()> {
    match purpose {
        "device_asset" => require_permission(principal, assets(Action::View)),
        _ => require_permission(principal, commercial(Action::View)),
    }
}

fn default_true() -> bool {
    true
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/label-templates"), get(list).post(create))
        .route(&format!("{base}/label-templates/preview"), post(preview))
        .route(
            &format!("{base}/label-templates/{{id}}"),
            get(get_one).patch(patch),
        )
        .route(
            &format!("{base}/label-templates/{{id}}/archive"),
            post(archive),
        )
        .route(&format!("{base}/labels/render"), post(render))
}

// ---------------------------------------------------------------------------
// Views + requests
// ---------------------------------------------------------------------------

/// The template as the API exposes it; `width_mm`/`height_mm` are the
/// resolved physical size (preset or custom), so clients never re-derive it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LabelTemplateView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub name: String,
    pub purpose: String,
    pub size_preset: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi: i32,
    #[schema(nullable = true)]
    pub barcode_token: Option<String>,
    pub include_qr: bool,
    pub fields: Vec<String>,
    pub is_default: bool,
    pub active: bool,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub created_by: Option<Uuid>,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<Utc>,
}

impl LabelTemplateView {
    fn from_template(t: LabelTemplate) -> Self {
        let (w, h) = core_domain::entities::size_tenths_mm(&t).unwrap_or((508, 254));
        Self {
            id: t.id,
            org_id: t.org_id,
            name: t.name,
            purpose: t.purpose,
            size_preset: t.size_preset,
            width_mm: w as f64 / 10.0,
            height_mm: h as f64 / 10.0,
            dpi: t.dpi,
            barcode_token: t.barcode_token,
            include_qr: t.include_qr,
            fields: t.fields,
            is_default: t.is_default,
            active: t.active,
            created_by: t.created_by,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LabelTemplateMutationResponse {
    #[serde(flatten)]
    pub template: LabelTemplateView,
    pub mutation: instant::MutationMeta,
}

/// Create body. `width_mm`/`height_mm` are required exactly when
/// `size_preset = "custom"`; millimetres are rounded to a tenth.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateLabelTemplateRequest {
    pub name: String,
    pub purpose: String,
    pub size_preset: String,
    #[serde(default)]
    pub width_mm: Option<f64>,
    #[serde(default)]
    pub height_mm: Option<f64>,
    #[serde(default)]
    pub dpi: Option<i32>,
    #[serde(default)]
    pub barcode_token: Option<String>,
    #[serde(default = "default_true")]
    pub include_qr: bool,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateLabelTemplateRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub size_preset: Option<String>,
    #[serde(default)]
    pub width_mm: Option<f64>,
    #[serde(default)]
    pub height_mm: Option<f64>,
    #[serde(default)]
    pub dpi: Option<i32>,
    /// Absent keeps the current token; `null` clears it (no barcode).
    #[serde(default, deserialize_with = "double_option")]
    pub barcode_token: Option<Option<String>>,
    #[serde(default)]
    pub include_qr: Option<bool>,
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub is_default: Option<bool>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Distinguishes "field absent" from "explicit null" in PATCH bodies.
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct TemplateListQuery {
    pub purpose: Option<String>,
    /// Defaults to true: the pickers only ever want live templates.
    #[serde(default)]
    pub active_only: Option<bool>,
}

/// Draft render body (the designer preview). Same shape as create minus the
/// template identity: nothing is persisted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PreviewLabelRequest {
    #[serde(default)]
    pub name: Option<String>,
    pub purpose: String,
    pub size_preset: String,
    #[serde(default)]
    pub width_mm: Option<f64>,
    #[serde(default)]
    pub height_mm: Option<f64>,
    #[serde(default)]
    pub dpi: Option<i32>,
    #[serde(default)]
    pub barcode_token: Option<String>,
    #[serde(default = "default_true")]
    pub include_qr: bool,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RenderLabelRequest {
    #[schema(value_type = uuid::Uuid)]
    pub template_id: Uuid,
    /// The entity the template's purpose points at (stock item id, serial
    /// id, location id, or device asset id).
    #[schema(value_type = uuid::Uuid)]
    pub entity_id: Uuid,
    #[serde(default = "default_true")]
    pub include_zpl: bool,
}

/// Which entity a rendered label names.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LabelEntityView {
    pub purpose: String,
    #[schema(value_type = uuid::Uuid)]
    pub entity_id: Uuid,
    pub display_name: String,
    /// The canonical deep link — the QR payload, and what `POST
    /// /api/scan/resolve` hands back for the printed label.
    pub deep_link: String,
}

/// The rendered label: geometry in millimetres, the QR and Code128 as
/// inline SVG markup (render them as images — never as script), and
/// optionally the exact ZPL the desktop shell sends to a Zebra printer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LabelRenderResponse {
    pub label: ResolvedLabel,
    #[schema(nullable = true)]
    pub qr_svg: Option<String>,
    #[schema(nullable = true)]
    pub code128_svg: Option<String>,
    #[schema(nullable = true)]
    pub zpl: Option<String>,
    pub entity: LabelEntityView,
    /// True when deterministic sample values were rendered (designer preview).
    pub sample: bool,
    pub template: LabelTemplateView,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tenths(mm: Option<f64>) -> Option<i32> {
    mm.map(|v| (v * 10.0).round() as i32)
}

fn template_from_create(
    org_id: Uuid,
    user_id: Uuid,
    req: &CreateLabelTemplateRequest,
) -> LabelTemplate {
    let now = Utc::now();
    LabelTemplate {
        id: Uuid::now_v7(),
        org_id,
        name: req.name.trim().to_string(),
        purpose: req.purpose.clone(),
        size_preset: req.size_preset.clone(),
        width_tenths_mm: tenths(req.width_mm),
        height_tenths_mm: tenths(req.height_mm),
        dpi: req.dpi.unwrap_or(203),
        barcode_token: req.barcode_token.clone(),
        include_qr: req.include_qr,
        fields: req.fields.clone(),
        is_default: req.is_default,
        active: true,
        created_by: Some(user_id),
        created_at: now,
        updated_at: now,
    }
}

fn template_from_preview(req: &PreviewLabelRequest) -> LabelTemplate {
    let now = Utc::now();
    LabelTemplate {
        id: SAMPLE_MARKER,
        org_id: SAMPLE_MARKER,
        name: req
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Preview".to_string()),
        purpose: req.purpose.clone(),
        size_preset: req.size_preset.clone(),
        width_tenths_mm: tenths(req.width_mm),
        height_tenths_mm: tenths(req.height_mm),
        dpi: req.dpi.unwrap_or(203),
        barcode_token: req.barcode_token.clone(),
        include_qr: req.include_qr,
        fields: req.fields.clone(),
        is_default: false,
        active: true,
        created_by: None,
        created_at: now,
        updated_at: now,
    }
}

/// Deterministic sample values per purpose — the designer preview's data.
fn sample_values(purpose: &str, org_name: &str) -> Vec<(String, String)> {
    let org = org_name.to_string();
    match purpose {
        "stock_item" => vec![
            ("name".into(), "Cat6 patch cable (1 m)".into()),
            ("sku".into(), "C6-1M".into()),
            ("part_number".into(), "PN-20431".into()),
            ("org_name".into(), org),
        ],
        "stock_location" => vec![
            ("name".into(), "Shelf A-3".into()),
            ("code".into(), "A-3".into()),
            ("org_name".into(), org),
        ],
        "serialized_unit" => vec![
            ("name".into(), "Torque wrench 1/2in".into()),
            ("sku".into(), "TW-12".into()),
            ("serial".into(), "TW-000123".into()),
            ("location".into(), "Van 2".into()),
            ("org_name".into(), org),
        ],
        "device_asset" => vec![
            ("name".into(), "ThinkPad T14".into()),
            ("tag".into(), "EXCH-01".into()),
            ("serial".into(), "PF-2AB123".into()),
            ("org_name".into(), org),
        ],
        _ => vec![],
    }
}

/// Renders a QR to inline SVG markup. Failure (a payload too large for any
/// QR version) returns `None` — an honest missing symbol, never a fake one.
fn qr_svg(data: &str) -> Option<String> {
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(96, 96)
            .build(),
    )
}

/// Renders a Code128 to inline SVG markup at the band's physical size.
/// Module-level bars stretch to the reserved width (`preserveAspectRatio`
/// off) exactly like the ZPL `^BCN` band does. Encoding failure (data that
/// Code128 cannot carry, e.g. non-ASCII) returns `None` and the preview
/// falls back to the human-readable text.
fn code128_svg(data: &str, width_mm: f64, height_mm: f64) -> Option<String> {
    if data.trim().is_empty() {
        return None;
    }
    // barcoders signals Code128 character sets with sentinel characters:
    // U+0181 starts set B, which carries every printable ASCII byte — the
    // right alphabet for SKUs, serials, location codes and asset tags.
    let payload = format!("\u{0181}{data}");
    let barcode = barcoders::sym::code128::Code128::new(payload).ok()?;
    let bits = barcode.encode();
    let modules = bits.len();
    if modules == 0 {
        return None;
    }
    let mut bars = String::new();
    for (i, bit) in bits.iter().enumerate() {
        if *bit == 1 {
            bars.push_str(&format!(
                "<rect x=\"{i}\" y=\"0\" width=\"1\" height=\"100\"/>"
            ));
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width_mm}mm\" height=\"{height_mm}mm\" \
         viewBox=\"0 0 {modules} 100\" preserveAspectRatio=\"none\">{bars}</svg>"
    ))
}

fn render_response(
    template: LabelTemplate,
    entity: LabelEntityView,
    values: Vec<(String, String)>,
    sample: bool,
    include_zpl: bool,
) -> LabelRenderResponse {
    let label = render_label(
        &template,
        &LabelInput {
            values,
            qr_data: entity.deep_link.clone(),
        },
    );
    let qr_svg = label.qr.as_ref().and_then(|qr| qr_svg(&qr.data));
    let code128_svg = label.barcode.as_ref().and_then(|barcode| {
        let text_w = label.width_mm - 2.0 * 1.5;
        code128_svg(&barcode.data, text_w, barcode.height_mm)
    });
    let zpl = include_zpl.then(|| to_zpl(&label));
    LabelRenderResponse {
        label,
        qr_svg,
        code128_svg,
        zpl,
        entity,
        sample,
        template: LabelTemplateView::from_template(template),
    }
}

async fn org_name(conn: &mut sqlx::PgConnection, org_id: Uuid) -> String {
    core_db::orgs::get_org(&mut *conn, org_id)
        .await
        .map(|o| o.name)
        .unwrap_or_else(|_| "Your organization".to_string())
}

/// Resolves the real entity values behind a render request. Returns
/// `(display_name, values)`.
async fn resolve_entity_values(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    purpose: &str,
    entity_id: Uuid,
    org: &str,
) -> ApiResult<(String, Vec<(String, String)>)> {
    let missing = || ApiError::NotFound;
    match purpose {
        "stock_item" => {
            let item = core_db::labels::get_item_by_id(&mut *conn, org_id, entity_id)
                .await?
                .ok_or_else(missing)?;
            Ok((
                item.name.clone(),
                vec![
                    ("name".into(), item.name),
                    ("sku".into(), item.sku.unwrap_or_default()),
                    ("part_number".into(), item.part_number.unwrap_or_default()),
                    ("org_name".into(), org.to_string()),
                ],
            ))
        }
        "stock_location" => {
            let loc = core_db::labels::get_location_by_id(&mut *conn, org_id, entity_id)
                .await?
                .ok_or_else(missing)?;
            Ok((
                loc.name.clone(),
                vec![
                    ("name".into(), loc.name),
                    ("code".into(), loc.code.unwrap_or_default()),
                    ("org_name".into(), org.to_string()),
                ],
            ))
        }
        "serialized_unit" => {
            let serial = core_db::labels::get_serial_by_id(&mut *conn, org_id, entity_id)
                .await?
                .ok_or_else(missing)?;
            let location = match serial.location_id {
                Some(location_id) => {
                    core_db::labels::get_location_by_id(&mut *conn, org_id, location_id)
                        .await?
                        .map(|l| l.name)
                        .unwrap_or_default()
                }
                None => String::new(),
            };
            Ok((
                format!("{} · {}", serial.item_name, serial.serial_number),
                vec![
                    ("name".into(), serial.item_name),
                    ("sku".into(), serial.item_sku.unwrap_or_default()),
                    ("serial".into(), serial.serial_number),
                    ("location".into(), location),
                    ("org_name".into(), org.to_string()),
                ],
            ))
        }
        "device_asset" => {
            let asset = core_db::labels::get_asset_by_id(&mut *conn, org_id, entity_id)
                .await?
                .ok_or_else(missing)?;
            let serial = asset_serial(&mut *conn, org_id, entity_id).await;
            Ok((
                asset.name.clone(),
                vec![
                    ("name".into(), asset.name),
                    ("tag".into(), asset.tag),
                    ("serial".into(), serial),
                    ("org_name".into(), org.to_string()),
                ],
            ))
        }
        other => Err(ApiError::Validation(format!(
            "unknown label purpose: {other}"
        ))),
    }
}

/// Best-effort serial from a device asset's deterministic identity keys
/// (CI3): a string, or a list whose first entry is a string.
async fn asset_serial(conn: &mut sqlx::PgConnection, org_id: Uuid, asset_id: Uuid) -> String {
    let keys: Option<Value> = sqlx::query_scalar!(
        r#"SELECT identity_keys FROM device_assets WHERE org_id = $1 AND id = $2"#,
        org_id,
        asset_id
    )
    .fetch_optional(&mut *conn)
    .await
    .ok()
    .flatten();
    let Some(keys) = keys else {
        return String::new();
    };
    match keys.get("serial") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .find_map(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/label-templates — org templates, optionally per purpose.
#[utoipa::path(operation_id = "label_templates_list", get, path = "/api/label-templates",
    tag = "labels", security(("bearer" = [])),
    params(("purpose" = Option<String>, Query, description = "Filter to one print purpose"),
           ("active_only" = Option<bool>, Query, description = "Defaults to true")),
    responses((status = 200, body = [LabelTemplateView])))]
pub async fn list(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<TemplateListQuery>,
) -> ApiResult<Json<Vec<LabelTemplateView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::labels::list_templates(
        &mut tx,
        org_id,
        q.purpose.as_deref(),
        q.active_only.unwrap_or(true),
        LIST_LIMIT,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(LabelTemplateView::from_template)
            .collect(),
    ))
}

/// GET /api/label-templates/{id}.
#[utoipa::path(operation_id = "label_templates_get", get, path = "/api/label-templates/{id}",
    tag = "labels", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path, description = "Template id")),
    responses((status = 200, body = LabelTemplateView), (status = 404)))]
pub async fn get_one(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<LabelTemplateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let template = core_db::labels::get_template(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(LabelTemplateView::from_template(template)))
}

/// POST /api/label-templates — create a template (Admin:edit).
#[utoipa::path(operation_id = "label_templates_create", post, path = "/api/label-templates",
    tag = "labels", security(("bearer" = [])), request_body = CreateLabelTemplateRequest,
    responses((status = 200, body = LabelTemplateMutationResponse), (status = 409), (status = 422)))]
pub async fn create(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateLabelTemplateRequest>,
) -> ApiResult<Json<LabelTemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let template = template_from_create(org_id, user_id, &req);
    template.validate().map_err(ApiError::from)?;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: TEMPLATE_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        if template.is_default {
            core_db::labels::clear_default(&mut ctx.tx, org_id, &template.purpose, template.id)
                .await?;
        }
        core_db::labels::insert_template(&mut ctx.tx, &template)
            .await
            .map_err(|e| unique_conflict(e, "a label template with that name already exists"))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: TEMPLATE_ENTITY,
                entity_id: template.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::LABEL_TEMPLATE_CREATED,
            TEMPLATE_ENTITY,
            Some(template.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::LabelTemplateCreated {
                template: template.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "label.template_created".into(),
                entity_type: Some(TEMPLATE_ENTITY.into()),
                entity_id: Some(template.id.to_string()),
                detail: serde_json::json!({
                    "name": template.name,
                    "purpose": template.purpose,
                    "size_preset": template.size_preset,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = LabelTemplateView::from_template(template.clone());
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: TEMPLATE_ENTITY,
                entity_id: template.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&template).unwrap_or(Value::Null),
                updated_at: template.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(LabelTemplateMutationResponse {
                template: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/label-templates/{id} — absent fields keep their value
/// (Admin:edit).
#[utoipa::path(operation_id = "label_templates_update", patch, path = "/api/label-templates/{id}",
    tag = "labels", security(("bearer" = [])), request_body = UpdateLabelTemplateRequest,
    params(("id" = uuid::Uuid, Path, description = "Template id")),
    responses((status = 200, body = LabelTemplateMutationResponse), (status = 404), (status = 409), (status = 422)))]
pub async fn patch(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<UpdateLabelTemplateRequest>,
) -> ApiResult<Json<LabelTemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: TEMPLATE_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::labels::get_template(&mut ctx.tx, org_id, id).await?;
        let mut next = before.clone();
        if let Some(name) = req.name.as_deref() {
            next.name = name.trim().to_string();
        }
        if let Some(purpose) = req.purpose.as_deref() {
            next.purpose = purpose.to_string();
        }
        if let Some(preset) = req.size_preset.as_deref() {
            next.size_preset = preset.to_string();
            if preset != "custom" {
                // A preset owns its dimensions; stale customs would lie.
                next.width_tenths_mm = None;
                next.height_tenths_mm = None;
            }
        }
        if req.width_mm.is_some() {
            next.width_tenths_mm = tenths(req.width_mm);
        }
        if req.height_mm.is_some() {
            next.height_tenths_mm = tenths(req.height_mm);
        }
        if req.dpi.is_some() {
            next.dpi = req.dpi.unwrap_or(203);
        }
        if let Some(token) = &req.barcode_token {
            next.barcode_token = token.clone();
        }
        if let Some(include_qr) = req.include_qr {
            next.include_qr = include_qr;
        }
        if let Some(fields) = &req.fields {
            next.fields = fields.clone();
        }
        if let Some(is_default) = req.is_default {
            next.is_default = is_default;
        }
        if let Some(active) = req.active {
            next.active = active;
            if !active {
                // An archived template cannot stay the default.
                next.is_default = false;
            }
        }
        next.updated_at = Utc::now();
        next.validate().map_err(ApiError::from)?;

        if next.is_default && !before.is_default {
            core_db::labels::clear_default(&mut ctx.tx, org_id, &next.purpose, id).await?;
        }
        core_db::labels::update_template(&mut ctx.tx, &next)
            .await
            .map_err(|e| unique_conflict(e, "a label template with that name already exists"))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: TEMPLATE_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::LABEL_TEMPLATE_UPDATED,
            TEMPLATE_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::LabelTemplateUpdated {
                template: next.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "label.template_updated".into(),
                entity_type: Some(TEMPLATE_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": next.name,
                    "active": next.active,
                    "is_default": next.is_default,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = LabelTemplateView::from_template(next.clone());
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: TEMPLATE_ENTITY,
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&next).unwrap_or(Value::Null),
                updated_at: next.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(LabelTemplateMutationResponse {
                template: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/label-templates/{id}/archive — archival is a flag; the
/// archived template frees the purpose's default slot.
#[utoipa::path(operation_id = "label_templates_archive", post, path = "/api/label-templates/{id}/archive",
    tag = "labels", security(("bearer" = [])), params(("id" = uuid::Uuid, Path, description = "Template id")),
    responses((status = 200, body = LabelTemplateMutationResponse), (status = 404)))]
pub async fn archive(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<LabelTemplateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::labels::get_template(&mut ctx.tx, org_id, id).await?;
        let mut next = before.clone();
        next.active = false;
        next.is_default = false;
        next.updated_at = Utc::now();
        core_db::labels::update_template(&mut ctx.tx, &next).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: TEMPLATE_ENTITY,
                entity_id: id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::LABEL_TEMPLATE_UPDATED,
            TEMPLATE_ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::LabelTemplateUpdated {
                template: next.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "label.template_archived".into(),
                entity_type: Some(TEMPLATE_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": next.name }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = LabelTemplateView::from_template(next.clone());
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: TEMPLATE_ENTITY,
                entity_id: id,
                op: "archive",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&next).unwrap_or(Value::Null),
                updated_at: next.updated_at,
                key: None,
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(LabelTemplateMutationResponse {
                template: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/label-templates/preview — render a DRAFT template against
/// deterministic sample values (the designer's live preview).
#[utoipa::path(operation_id = "label_templates_preview", post, path = "/api/label-templates/preview",
    tag = "labels", security(("bearer" = [])), request_body = PreviewLabelRequest,
    responses((status = 200, body = LabelRenderResponse), (status = 422)))]
pub async fn preview(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<PreviewLabelRequest>,
) -> ApiResult<Json<LabelRenderResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_render_permission(&authed.principal, &req.purpose)?;
    let org_id = authed.principal.user.org_id;
    let template = template_from_preview(&req);
    template.validate().map_err(ApiError::from)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let org = org_name(&mut tx, org_id).await;
    tx.commit().await.map_err(ApiError::from)?;

    let deep_link = core_domain::entities::deep_link(&template.purpose, SAMPLE_MARKER);
    let entity = LabelEntityView {
        purpose: template.purpose.clone(),
        entity_id: SAMPLE_MARKER,
        display_name: "Sample label".to_string(),
        deep_link,
    };
    Ok(Json(render_response(
        template.clone(),
        entity,
        sample_values(&template.purpose, &org),
        true,
        true,
    )))
}

/// POST /api/labels/render — resolve a real entity and render its label:
/// browser-print geometry + the exact ZPL the desktop shell transports.
#[utoipa::path(operation_id = "labels_render", post, path = "/api/labels/render",
    tag = "labels", security(("bearer" = [])), request_body = RenderLabelRequest,
    responses((status = 200, body = LabelRenderResponse), (status = 404), (status = 422)))]
pub async fn render(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<RenderLabelRequest>,
) -> ApiResult<Json<LabelRenderResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let template = core_db::labels::get_template(&mut tx, org_id, req.template_id).await?;
    require_render_permission(&authed.principal, &template.purpose)?;
    let org = org_name(&mut tx, org_id).await;
    let (display_name, values) =
        resolve_entity_values(&mut tx, org_id, &template.purpose, req.entity_id, &org).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let entity = LabelEntityView {
        purpose: template.purpose.clone(),
        entity_id: req.entity_id,
        display_name,
        deep_link: core_domain::entities::deep_link(&template.purpose, req.entity_id),
    };
    Ok(Json(render_response(
        template,
        entity,
        values,
        false,
        req.include_zpl,
    )))
}

fn unique_conflict(e: core_db::DbError, what: &str) -> ApiError {
    let unique = matches!(
        &e,
        core_db::DbError::Sql(sqlx::Error::Database(db_err))
            if db_err.is_unique_violation()
    );
    if unique {
        ApiError::Conflict(what.to_string())
    } else {
        e.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code128_svg_is_deterministic_and_carries_the_data() {
        let a = code128_svg("TW-000123", 47.8, 7.1).expect("encodes");
        let b = code128_svg("TW-000123", 47.8, 7.1).expect("encodes");
        assert_eq!(a, b, "same data renders identical bytes");
        assert!(a.contains("width=\"47.8mm\""), "{a}");
        assert!(a.contains("<rect"), "bar modules present");
        assert!(a.starts_with("<svg"), "{a}");
        // Data Code128 cannot carry falls back honestly (no fake symbol).
        assert!(code128_svg("", 20.0, 5.0).is_none());
    }

    #[test]
    fn qr_svg_renders_a_scannable_payload_shape() {
        let svg = qr_svg("meridian://stock-item/abc").expect("qr");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<path"), "qr modules as a path");
    }
}
