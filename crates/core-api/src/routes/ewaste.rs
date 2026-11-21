//! E-waste custody API (ADR 0035 decision 5, migration 0238).
//!
//! Client e-waste is received, stored, moved, handed off and disposed of. The
//! surface is deliberately one aggregate — an intake — with four answers:
//! what was received, where it is, how it left, and what is left. Every
//! mutation returns the whole intake so the caller's next write can carry the
//! version it just read.
//!
//! What this module does not do: it never posts to stock. An intake changes
//! owned quantity by zero; a client asset that arrives retires through the
//! class-movement ledger (written by migration 0238's trigger, not here), and
//! a disposed unit leaves the live class the same way. `stock_levels` and
//! `stock_movements` are not named anywhere in this file.
//!
//! Guards live in the database and are surfaced here as typed refusals:
//! pending data destruction blocks a disposition, a serialized unit leaves
//! once and at its full weight, a lot cannot be over-disposed, and an intake
//! cannot close while a gram is unaccounted for. This layer validates what a
//! person typed (vocabulary, positive weights, client coherence) so a mistake
//! is a 422 or 404 instead of a 409 from a trigger; the trigger is what
//! survives a race.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::documents::attached_document_exists;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::http::Extensions;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use core_db::ewaste::{
    self, DispositionFilter, EwasteDisposition, EwasteEvent, EwasteIntake, EwasteIntakeLine,
    IntakeFilter, IntakeSummary, NewDisposition, NewEvent, NewIntake, NewIntakeLine, WeightBalance,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const ENTITY: &str = ewaste::INTAKE_ENTITY;
const LIST_LIMIT: i64 = 200;

/// Whose client restrictions apply to a list read. The row-level clause is
/// what hides another client's e-waste when the caller names no client; the
/// owner/admin bypass and the retained-client rule live in that clause.
fn visibility_of(
    principal: &core_domain::entities::Principal,
) -> core_db::tickets::ClientVisibility {
    core_db::tickets::ClientVisibility {
        user_id: principal.user.id,
        is_admin: principal.is_owner_or_admin(),
    }
}

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/ewaste/intakes"), get(list).post(create))
        .route(&format!("{base}/ewaste/intakes/{{id}}"), get(get_one))
        .route(
            &format!("{base}/ewaste/intakes/{{id}}/lines"),
            post(add_line),
        )
        .route(
            &format!("{base}/ewaste/intakes/{{id}}/events"),
            post(record_event),
        )
        .route(
            &format!("{base}/ewaste/intakes/{{id}}/lines/{{line_id}}/destruction"),
            post(set_destruction),
        )
        .route(
            &format!("{base}/ewaste/intakes/{{id}}/dispositions"),
            post(record_disposition),
        )
        .route(
            &format!("{base}/ewaste/intakes/{{id}}/close"),
            post(close_intake),
        )
        .route(
            &format!("{base}/ewaste/dispositions"),
            get(list_dispositions),
        )
}

/// Client visibility for an e-waste record.
///
/// A live client is checked exactly as [`super::clients::require_client_visible`]
/// checks it. An intake's `client_id` is deliberately not a foreign key, because
/// the tenant erasure contract keeps the receipt when a client row is deleted;
/// a client that no longer exists carries no restriction, so the retained record
/// stays readable by anyone holding the module permission. Loading the client
/// first is what separates "gone" from "restricted": both are 404 through the
/// shared helper, and catching that 404 would turn a restriction into access.
async fn require_intake_client(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    principal: &core_domain::entities::Principal,
) -> ApiResult<Option<core_domain::entities::Client>> {
    match core_db::clients::get_client(&mut *conn, org_id, client_id).await {
        Ok(client) => {
            if !client.restricted_to.is_empty()
                && !client.restricted_to.contains(&principal.user.id)
                && !principal.is_owner_or_admin()
            {
                return Err(ApiError::NotFound);
            }
            Ok(Some(client))
        }
        Err(core_db::DbError::NotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn require_intake_client_visible(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    principal: &core_domain::entities::Principal,
) -> ApiResult<()> {
    require_intake_client(conn, org_id, client_id, principal)
        .await
        .map(|_| ())
}

/// Whether this intake's events may ride the org-wide stream.
///
/// The stream frame carries the entity and its id but no client, so the live
/// and replay paths cannot apply the restricted-client rule. Only a live client
/// with an empty restriction is safe for every subscriber; a restricted client
/// and a retained intake (client gone) are withheld until scoped delivery
/// exists. The outbox row is what the stream reads, so withholding means not
/// emitting it; the mutation trail in `entity_writes` and the custody ledger
/// still record the act.
fn can_stream(client: &Option<core_domain::entities::Client>) -> bool {
    client
        .as_ref()
        .is_some_and(|client| client.restricted_to.is_empty())
}

/// Emits a domain event only when its intake may be streamed.
async fn emit_if_streamable(
    ctx: &mut org_tx::OrgTx<'_>,
    streamable: bool,
    event_type: &str,
    intake_id: Uuid,
    payload: &Value,
) -> ApiResult<()> {
    if streamable {
        ctx.emit(
            event_type,
            ENTITY,
            Some(intake_id.to_string().as_str()),
            payload,
        )
        .await?;
    }
    Ok(())
}

/// A stored replay is still a read of a client's data. Visibility can have
/// narrowed since the write (a client can become restricted), so the replay
/// branch runs the same client check as the fresh path before it answers. The
/// stored shape is this module's own; a value without an intake client is
/// corruption, not a caller error, and fails closed.
async fn require_replay_visible(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    principal: &core_domain::entities::Principal,
    stored: &Value,
) -> ApiResult<()> {
    let client_id = stored
        .get("intake")
        .and_then(|intake| intake.get("client_id"))
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!(
                "stored e-waste replay carries no intake client"
            ))
        })?;
    require_intake_client_visible(conn, org_id, client_id, principal).await
}

// ---- responses -------------------------------------------------------------

/// One intake with everything that answers "where did it go".
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IntakeDetailResponse {
    pub intake: EwasteIntake,
    pub lines: Vec<EwasteIntakeLine>,
    pub events: Vec<EwasteEvent>,
    pub dispositions: Vec<EwasteDisposition>,
    pub balance: WeightBalance,
    /// The intake's write-sequence version. A client echoes it as `version` on
    /// the next conditional write, notably the close.
    pub version: i64,
}

/// The intake response plus the instant-first envelope metadata (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IntakeMutationResponse {
    #[serde(flatten)]
    pub detail: IntakeDetailResponse,
    pub mutation: instant::MutationMeta,
}

/// The answering query for a mutation: the aggregate, and the version it now
/// carries.
async fn load_detail(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
) -> ApiResult<IntakeDetailResponse> {
    let intake = ewaste::get_intake(&mut *conn, org_id, intake_id).await?;
    let lines = ewaste::list_lines(&mut *conn, org_id, intake_id).await?;
    let events = ewaste::list_events(&mut *conn, org_id, intake_id).await?;
    let dispositions = ewaste::list_dispositions(&mut *conn, org_id, intake_id).await?;
    let balance = ewaste::weight_balance(&mut *conn, org_id, intake_id).await?;
    let version = ewaste::intake_version(&mut *conn, org_id, intake_id).await?;
    Ok(IntakeDetailResponse {
        intake,
        lines,
        events,
        dispositions,
        balance,
        version,
    })
}

/// The aggregate as committed by the previous writer. Read after the sequence
/// lock is held and before this mutation writes. The lock keeps another writer
/// out, and the sequence row already carries this write's number, so the prior
/// state is one version behind it.
async fn prior_detail(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
    write_seq: i64,
) -> ApiResult<IntakeDetailResponse> {
    let mut prior = load_detail(conn, org_id, intake_id).await?;
    prior.version = write_seq.saturating_sub(1);
    Ok(prior)
}

// ---- requests --------------------------------------------------------------

/// One line of a received consignment.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct IntakeLineRequest {
    /// serial: a unit with a stock serial, quantity one. lot: a quantity of
    /// one category, identified by this line.
    pub line_kind: String,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
    /// The client's asset this unit is, when it is in their fleet.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub device_asset_id: Option<Uuid>,
    #[serde(default)]
    pub lot_label: Option<String>,
    pub category: String,
    /// The line's total weight in whole grams.
    pub weight_g: i64,
    #[serde(default = "one")]
    pub quantity: i32,
    #[serde(default)]
    pub data_bearing: bool,
}

fn one() -> i32 {
    1
}

/// POST /api/ewaste/intakes.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateIntakeRequest {
    pub client_id: Uuid,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub site_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[serde(default)]
    pub reference: Option<String>,
    /// Defaults to now.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub received_at: Option<DateTime<Utc>>,
    pub received_by_name: String,
    /// The signed intake sheet held in the documents platform.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub signature_document_id: Option<Uuid>,
    #[serde(default)]
    pub document_reference: Option<String>,
    /// The client's instruction: destroy data before disposal.
    #[serde(default)]
    pub data_destruction_required: bool,
    #[serde(default)]
    pub hazard_flags: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub lines: Vec<IntakeLineRequest>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/ewaste/intakes/{id}/lines.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AddLineRequest {
    #[serde(flatten)]
    pub line: IntakeLineRequest,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/ewaste/intakes/{id}/events — store, move or handoff.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RecordEventRequest {
    /// store | move | handoff. Intake and disposition events are written by
    /// the records that produce them.
    pub event_kind: String,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub line_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub occurred_at: Option<DateTime<Utc>>,
    /// Required for store, move and handoff.
    #[serde(default)]
    pub storage_class: Option<String>,
    #[serde(default)]
    pub location_note: Option<String>,
    /// Required for handoff.
    #[serde(default)]
    pub partner_name: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/ewaste/intakes/{id}/lines/{line_id}/destruction.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetDestructionRequest {
    /// pending | completed | not_required.
    pub status: String,
    /// The destruction certificate, when there is one to attach.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub document_id: Option<Uuid>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/ewaste/intakes/{id}/dispositions.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RecordDispositionRequest {
    pub line_id: Uuid,
    /// recycled | destroyed | resold | returned_to_client |
    /// transferred_to_partner.
    pub pathway: String,
    /// Required for recycled and transferred_to_partner.
    #[serde(default)]
    pub partner_name: Option<String>,
    #[schema(value_type = String)]
    pub disposed_at: NaiveDate,
    /// Whole grams; a unit line must carry its full weight.
    pub weight_g: i64,
    /// Required (with the document) for recycled, destroyed and
    /// transferred_to_partner.
    #[serde(default)]
    pub certificate_reference: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub document_id: Option<Uuid>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/ewaste/intakes/{id}/close. The body is optional: a caller that
/// has not read a version may close without one, and a caller that has read
/// the intake passes the version it saw.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CloseIntakeRequest {
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/ewaste/intakes.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ListIntakesQuery {
    /// open | closed.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/ewaste/dispositions.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ListDispositionsQuery {
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub pathway: Option<String>,
    #[serde(default)]
    pub from: Option<NaiveDate>,
    #[serde(default)]
    pub to: Option<NaiveDate>,
    #[serde(default)]
    pub limit: Option<i64>,
    /// Keyset cursor: the three fields of the last row on the previous page.
    /// Pass all three or none.
    #[serde(default)]
    pub cursor_disposed_at: Option<NaiveDate>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub cursor_created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub cursor_id: Option<Uuid>,
}

// ---- validation ------------------------------------------------------------

/// A length the migration's CHECK enforces. Checking it here turns a 500 from
/// a constraint violation into the 422 the caller can act on. The raw length is
/// counted, not the trimmed one: a value stored as it arrived must satisfy the
/// column's limit as it arrived.
fn require_max_len(value: &str, max: usize, field: &str) -> ApiResult<()> {
    if value.chars().count() > max {
        return Err(ApiError::Validation(format!(
            "{field} must be at most {max} characters"
        )));
    }
    Ok(())
}

/// Trims an optional free-text field and treats an all-whitespace value as
/// absent, so an empty string cannot reach a column whose CHECK requires a
/// non-empty trimmed value.
fn clean_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn validate_line(req: &IntakeLineRequest) -> ApiResult<NewIntakeLine> {
    let kind = req.line_kind.trim();
    if !ewaste::is_valid_line_kind(kind) {
        return Err(ApiError::Validation(
            "line_kind must be serial or lot".into(),
        ));
    }
    if req.weight_g <= 0 {
        return Err(ApiError::Validation(
            "weight_g must be a positive number of whole grams".into(),
        ));
    }
    if req.quantity < 1 {
        return Err(ApiError::Validation("quantity must be at least 1".into()));
    }
    if req.device_asset_id.is_some() && req.quantity != 1 {
        return Err(ApiError::Validation(
            "a line that names one client asset is one unit; quantity must be 1".into(),
        ));
    }
    let category = req.category.trim();
    if category.is_empty() {
        return Err(ApiError::Validation("category is required".into()));
    }
    if category.chars().count() > 120 {
        return Err(ApiError::Validation(
            "category must be at most 120 characters".into(),
        ));
    }
    match (kind, req.serial_id) {
        ("serial", None) => {
            return Err(ApiError::Validation(
                "a serial line needs a serial_id".into(),
            ))
        }
        ("serial", Some(_)) if req.quantity != 1 => {
            return Err(ApiError::Validation(
                "a serial line is one unit; quantity must be 1".into(),
            ))
        }
        ("lot", Some(_)) => {
            return Err(ApiError::Validation(
                "a lot line carries no serial; the line is the identity".into(),
            ))
        }
        _ => {}
    }
    let lot_label = req
        .lot_label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if lot_label
        .as_deref()
        .is_some_and(|l| l.chars().count() > 200)
    {
        return Err(ApiError::Validation(
            "lot_label must be at most 200 characters".into(),
        ));
    }
    Ok(NewIntakeLine {
        line_kind: kind.to_string(),
        serial_id: req.serial_id,
        device_asset_id: req.device_asset_id,
        lot_label,
        category: category.to_string(),
        weight_g: req.weight_g,
        quantity: req.quantity,
        data_bearing: req.data_bearing,
    })
}

/// The site and ticket on an intake must belong to the intake's client; a
/// mismatch is a cross-tenant read dressed as a write, so it 404s.
async fn require_site_and_ticket_match_client(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    site_id: Option<Uuid>,
    ticket_id: Option<Uuid>,
) -> ApiResult<()> {
    if let Some(site_id) = site_id {
        let owner = sqlx::query_scalar!(
            "SELECT client_id FROM sites WHERE id = $1 AND org_id = $2",
            site_id,
            org_id
        )
        .fetch_optional(&mut *conn)
        .await?;
        if owner != Some(client_id) {
            return Err(ApiError::NotFound);
        }
    }
    if let Some(ticket_id) = ticket_id {
        let owner = sqlx::query_scalar!(
            "SELECT client_id FROM tickets WHERE id = $1 AND org_id = $2",
            ticket_id,
            org_id
        )
        .fetch_optional(&mut *conn)
        .await?;
        if owner != Some(Some(client_id)) {
            return Err(ApiError::NotFound);
        }
    }
    Ok(())
}

/// The part of the guard a person can cause by typing: a certificate is
/// required where a downstream custodian takes the goods.
fn validate_disposition(req: &RecordDispositionRequest) -> ApiResult<()> {
    let pathway = req.pathway.trim();
    if !ewaste::is_valid_pathway(pathway) {
        return Err(ApiError::Validation(format!(
            "pathway must be one of {}",
            ewaste::PATHWAYS.join(", ")
        )));
    }
    if req.weight_g <= 0 {
        return Err(ApiError::Validation(
            "weight_g must be a positive number of whole grams".into(),
        ));
    }
    if matches!(pathway, "recycled" | "transferred_to_partner")
        && req
            .partner_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none()
    {
        return Err(ApiError::Validation(
            "this pathway names the partner that took the goods".into(),
        ));
    }
    if ewaste::pathway_needs_certificate(pathway)
        && req
            .certificate_reference
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none()
        && req.document_id.is_none()
    {
        return Err(ApiError::Validation(
            "this pathway needs a certificate reference or an attached document".into(),
        ));
    }
    if let Some(partner) = req.partner_name.as_deref() {
        require_max_len(partner, 200, "partner_name")?;
    }
    if let Some(certificate) = req.certificate_reference.as_deref() {
        require_max_len(certificate, 300, "certificate_reference")?;
    }
    Ok(())
}

// ---- handlers --------------------------------------------------------------

/// GET /api/ewaste/intakes — newest receipt first, weights aggregated.
#[utoipa::path(operation_id = "ewaste_list_intakes", get, path = "/api/ewaste/intakes", tag = "ewaste", security(("bearer" = [])),
    params(ListIntakesQuery),
    responses((status = 200, body = [IntakeSummary])))]
pub async fn list(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<ListIntakesQuery>,
) -> ApiResult<Json<Vec<IntakeSummary>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    if let Some(status) = q.status.as_deref() {
        if !ewaste::is_valid_intake_status(status) {
            return Err(ApiError::Validation("status must be open or closed".into()));
        }
    }
    let org_id = authed.principal.user.org_id;
    let filter = IntakeFilter {
        status: q.status.as_deref().map(str::trim).map(str::to_string),
        client_id: q.client_id,
        limit: q.limit.unwrap_or(LIST_LIMIT).clamp(1, LIST_LIMIT),
        visibility: Some(visibility_of(&authed.principal)),
    };
    // One transaction for the scope check and the read: a client can become
    // restricted between two statements, and a filtered list must not then
    // answer for it.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    if let Some(client_id) = q.client_id {
        require_intake_client_visible(&mut tx, org_id, client_id, &authed.principal).await?;
    }
    let rows = ewaste::list_intakes(&mut tx, org_id, &filter).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// GET /api/ewaste/intakes/{id}.
#[utoipa::path(operation_id = "ewaste_get_intake", get, path = "/api/ewaste/intakes/{id}", tag = "ewaste", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = IntakeDetailResponse),
              (status = 404, description = "intake not visible in scope")))]
pub async fn get_one(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<IntakeDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let intake = ewaste::get_intake(&mut tx, org_id, id).await?;
    require_intake_client_visible(&mut tx, org_id, intake.client_id, &authed.principal).await?;
    let detail = load_detail(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(detail))
}

/// POST /api/ewaste/intakes — instant-first (IF1/IF2).
#[utoipa::path(operation_id = "ewaste_create_intake", post, path = "/api/ewaste/intakes", tag = "ewaste", security(("bearer" = [])),
    request_body = CreateIntakeRequest,
    responses((status = 200, body = IntakeMutationResponse),
              (status = 404, description = "client, site, ticket or document not visible in scope"),
              (status = 409, description = "idempotency key reused or intake reference taken"),
              (status = 422, description = "line rejected")))]
pub async fn create(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateIntakeRequest>,
) -> ApiResult<Json<IntakeMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let received_by_name = req.received_by_name.trim().to_string();
    if received_by_name.is_empty() {
        return Err(ApiError::Validation("received_by_name is required".into()));
    }
    require_max_len(&received_by_name, 200, "received_by_name")?;
    if let Some(reference) = req.reference.as_deref() {
        require_max_len(reference, 100, "reference")?;
    }
    if let Some(reference) = req.document_reference.as_deref() {
        require_max_len(reference, 500, "document_reference")?;
    }
    if let Some(notes) = req.notes.as_deref() {
        require_max_len(notes, 4000, "notes")?;
    }
    // Validate and store the same trimmed values: the table's CHECK accepts
    // only the canonical spelling, and `is_valid_hazard_flag` trims.
    let hazard_flags: Vec<String> = req
        .hazard_flags
        .iter()
        .map(|flag| flag.trim().to_string())
        .collect();
    for flag in &hazard_flags {
        if !ewaste::is_valid_hazard_flag(flag) {
            return Err(ApiError::Validation(format!(
                "unknown hazard flag '{flag}'; allowed: {}",
                ewaste::HAZARD_FLAGS.join(", ")
            )));
        }
    }
    let lines = req
        .lines
        .iter()
        .map(validate_line)
        .collect::<ApiResult<Vec<_>>>()?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let input = NewIntake {
        client_id: req.client_id,
        site_id: req.site_id,
        ticket_id: req.ticket_id,
        reference: clean_text(req.reference.as_deref()),
        received_at: req.received_at.unwrap_or_else(Utc::now),
        received_by_name,
        received_by_user_id: Some(user_id),
        signature_document_id: req.signature_document_id,
        document_reference: clean_text(req.document_reference.as_deref()),
        data_destruction_required: req.data_destruction_required,
        hazard_flags,
        notes: clean_text(req.notes.as_deref()),
        lines,
    };

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            require_replay_visible(&mut ctx.tx, org_id, &authed.principal, &stored).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let client = super::clients::require_client_visible(
            &mut ctx.tx,
            org_id,
            input.client_id,
            &authed.principal,
        )
        .await?;
        let streamable = client.restricted_to.is_empty();
        require_site_and_ticket_match_client(
            &mut ctx.tx,
            org_id,
            input.client_id,
            input.site_id,
            input.ticket_id,
        )
        .await?;
        if let Some(document_id) = input.signature_document_id {
            attached_document_exists(&mut ctx.tx, org_id, document_id).await?;
        }

        let id = Uuid::now_v7();
        ewaste::create_intake(&mut ctx.tx, org_id, user_id, id, &input).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, id).await?;
        emit_if_streamable(
            &mut ctx,
            streamable,
            "ewaste.intake_recorded",
            id,
            &serde_json::json!({
                "intake_id": id,
                "client_id": input.client_id,
                "lines": detail.lines.len(),
                "total_in_g": detail.balance.total_in_g,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: detail.intake.updated_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(IntakeMutationResponse { detail, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/ewaste/intakes/{id}/lines — add one line while the intake is open.
#[utoipa::path(operation_id = "ewaste_add_line", post, path = "/api/ewaste/intakes/{id}/lines", tag = "ewaste", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddLineRequest,
    responses((status = 200, body = IntakeMutationResponse),
              (status = 404, description = "intake or its client not visible in scope"),
              (status = 409, description = "the intake is closed")))]
pub async fn add_line(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<AddLineRequest>,
) -> ApiResult<Json<IntakeMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let line = validate_line(&req.line)?;
    let fingerprint = serde_json::to_value((id, &req)).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            require_replay_visible(&mut ctx.tx, org_id, &authed.principal, &stored).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let intake = ewaste::get_intake(&mut ctx.tx, org_id, id).await?;
        let client =
            require_intake_client(&mut ctx.tx, org_id, intake.client_id, &authed.principal).await?;
        let streamable = can_stream(&client);
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let prior = prior_detail(&mut ctx.tx, org_id, id, write_seq).await?;
        ewaste::add_line(&mut ctx.tx, org_id, user_id, id, &line).await?;
        let detail = load_detail(&mut ctx.tx, org_id, id).await?;
        emit_if_streamable(
            &mut ctx,
            streamable,
            "ewaste.line_added",
            id,
            &serde_json::json!({
                "intake_id": id,
                "line_kind": line.line_kind,
                "weight_g": line.weight_g,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "line_add",
                seq: write_seq,
                before: Some(serde_json::to_value(&prior).unwrap_or(Value::Null)),
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: detail.intake.updated_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(IntakeMutationResponse { detail, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/ewaste/intakes/{id}/events — store, move or handoff.
#[utoipa::path(operation_id = "ewaste_record_event", post, path = "/api/ewaste/intakes/{id}/events", tag = "ewaste", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = RecordEventRequest,
    responses((status = 200, body = IntakeMutationResponse),
              (status = 404, description = "intake or its client not visible in scope"),
              (status = 409, description = "the intake is closed"),
              (status = 422, description = "unknown event kind or missing storage class")))]
pub async fn record_event(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<RecordEventRequest>,
) -> ApiResult<Json<IntakeMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let kind = req.event_kind.trim();
    if !ewaste::is_valid_caller_event_kind(kind) {
        return Err(ApiError::Validation(format!(
            "event_kind must be one of {}",
            ewaste::CALLER_EVENT_KINDS.join(", ")
        )));
    }
    let storage_class = req
        .storage_class
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if storage_class.is_none() {
        return Err(ApiError::Validation(
            "storage_class is required for store, move and handoff".into(),
        ));
    }
    if let Some(class) = storage_class.as_deref() {
        if !ewaste::is_valid_storage_class(class) {
            return Err(ApiError::Validation(format!(
                "unknown storage class '{class}'; allowed: {}",
                ewaste::STORAGE_CLASSES.join(", ")
            )));
        }
    }
    if kind == "handoff"
        && req
            .partner_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none()
    {
        return Err(ApiError::Validation(
            "a handoff names the partner that took the goods".into(),
        ));
    }
    if let Some(partner) = req.partner_name.as_deref() {
        require_max_len(partner, 200, "partner_name")?;
    }
    if let Some(note) = req.location_note.as_deref() {
        require_max_len(note, 300, "location_note")?;
    }
    if let Some(notes) = req.notes.as_deref() {
        require_max_len(notes, 2000, "notes")?;
    }

    let fingerprint = serde_json::to_value((id, &req)).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let input = NewEvent {
        line_id: req.line_id,
        event_kind: kind.to_string(),
        occurred_at: req.occurred_at,
        storage_class,
        location_note: clean_text(req.location_note.as_deref()),
        partner_name: clean_text(req.partner_name.as_deref()),
        actor_user_id: Some(user_id),
        notes: clean_text(req.notes.as_deref()),
    };

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            require_replay_visible(&mut ctx.tx, org_id, &authed.principal, &stored).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let intake = ewaste::get_intake(&mut ctx.tx, org_id, id).await?;
        let client =
            require_intake_client(&mut ctx.tx, org_id, intake.client_id, &authed.principal).await?;
        let streamable = can_stream(&client);
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let prior = prior_detail(&mut ctx.tx, org_id, id, write_seq).await?;
        ewaste::record_event(&mut ctx.tx, org_id, id, &input).await?;
        let detail = load_detail(&mut ctx.tx, org_id, id).await?;
        emit_if_streamable(
            &mut ctx,
            streamable,
            "ewaste.event_recorded",
            id,
            &serde_json::json!({
                "intake_id": id,
                "event_kind": input.event_kind,
                "storage_class": input.storage_class,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "custody_event",
                seq: write_seq,
                before: Some(serde_json::to_value(&prior).unwrap_or(Value::Null)),
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: detail.intake.updated_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(IntakeMutationResponse { detail, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/ewaste/intakes/{id}/lines/{line_id}/destruction — record the
/// destruction state of a data-bearing line.
#[utoipa::path(operation_id = "ewaste_set_destruction", post, path = "/api/ewaste/intakes/{id}/lines/{line_id}/destruction", tag = "ewaste", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("line_id" = Uuid, Path)), request_body = SetDestructionRequest,
    responses((status = 200, body = IntakeMutationResponse),
              (status = 404, description = "intake, line or document not visible in scope"),
              (status = 409, description = "the intake is closed or the line is not data-bearing")))]
pub async fn set_destruction(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path((id, line_id)): axum::extract::Path<(Uuid, Uuid)>,
    axum::extract::Json(req): axum::extract::Json<SetDestructionRequest>,
) -> ApiResult<Json<IntakeMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let status = req.status.trim();
    if !ewaste::is_valid_destruction_state(status) {
        return Err(ApiError::Validation(
            "status must be not_required, pending or completed".into(),
        ));
    }
    let fingerprint = serde_json::to_value((id, line_id, &req)).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            require_replay_visible(&mut ctx.tx, org_id, &authed.principal, &stored).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let intake = ewaste::get_intake(&mut ctx.tx, org_id, id).await?;
        let client =
            require_intake_client(&mut ctx.tx, org_id, intake.client_id, &authed.principal).await?;
        let streamable = can_stream(&client);
        if let Some(document_id) = req.document_id {
            attached_document_exists(&mut ctx.tx, org_id, document_id).await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let prior = prior_detail(&mut ctx.tx, org_id, id, write_seq).await?;
        ewaste::set_destruction_state(
            &mut ctx.tx,
            org_id,
            user_id,
            id,
            line_id,
            status,
            req.document_id,
        )
        .await?;
        let detail = load_detail(&mut ctx.tx, org_id, id).await?;
        emit_if_streamable(
            &mut ctx,
            streamable,
            "ewaste.destruction_recorded",
            id,
            &serde_json::json!({
                "intake_id": id,
                "line_id": line_id,
                "destruction_status": status,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "destruction_state",
                seq: write_seq,
                before: Some(serde_json::to_value(&prior).unwrap_or(Value::Null)),
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: detail.intake.updated_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(IntakeMutationResponse { detail, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/ewaste/intakes/{id}/dispositions — record a way out.
#[utoipa::path(operation_id = "ewaste_record_disposition", post, path = "/api/ewaste/intakes/{id}/dispositions", tag = "ewaste", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = RecordDispositionRequest,
    responses((status = 200, body = IntakeMutationResponse),
              (status = 404, description = "intake, line or document not visible in scope"),
              (status = 409, description = "destruction pending, weight overdraw, unit already disposed, or intake closed"),
              (status = 422, description = "unknown pathway or missing certificate/partner")))]
pub async fn record_disposition(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<RecordDispositionRequest>,
) -> ApiResult<Json<IntakeMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    validate_disposition(&req)?;
    let fingerprint = serde_json::to_value((id, &req)).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let input = NewDisposition {
        line_id: req.line_id,
        pathway: req.pathway.trim().to_string(),
        partner_name: clean_text(req.partner_name.as_deref()),
        disposed_at: req.disposed_at,
        weight_g: req.weight_g,
        certificate_reference: clean_text(req.certificate_reference.as_deref()),
        document_id: req.document_id,
    };

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            require_replay_visible(&mut ctx.tx, org_id, &authed.principal, &stored).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let intake = ewaste::get_intake(&mut ctx.tx, org_id, id).await?;
        let client =
            require_intake_client(&mut ctx.tx, org_id, intake.client_id, &authed.principal).await?;
        let streamable = can_stream(&client);
        if let Some(document_id) = input.document_id {
            attached_document_exists(&mut ctx.tx, org_id, document_id).await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let prior = prior_detail(&mut ctx.tx, org_id, id, write_seq).await?;
        let disposition =
            ewaste::record_disposition(&mut ctx.tx, org_id, user_id, id, &input).await?;
        let detail = load_detail(&mut ctx.tx, org_id, id).await?;
        emit_if_streamable(
            &mut ctx,
            streamable,
            "ewaste.disposition_recorded",
            id,
            &serde_json::json!({
                "intake_id": id,
                "disposition_id": disposition.id,
                "pathway": input.pathway,
                "weight_g": input.weight_g,
                "certificate_reference": input.certificate_reference,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "disposition",
                seq: write_seq,
                before: Some(serde_json::to_value(&prior).unwrap_or(Value::Null)),
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: detail.intake.updated_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(IntakeMutationResponse { detail, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/ewaste/intakes/{id}/close — the balanced end of a receipt.
///
/// The body is optional. It may carry `version`, checked before the close
/// writes, and an `idempotency_key`, so a retry after a lost response replays
/// the stored result instead of colliding with the close it already made.
#[utoipa::path(operation_id = "ewaste_close_intake", post, path = "/api/ewaste/intakes/{id}/close", tag = "ewaste", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = IntakeMutationResponse),
              (status = 404, description = "intake not visible in scope"),
              (status = 409, description = "weight remains, destruction pending, or already closed")))]
pub async fn close_intake(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    body: Option<Json<CloseIntakeRequest>>,
) -> ApiResult<Json<IntakeMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let (version, key) = match body {
        Some(Json(body)) => (body.instant.version, body.instant.idempotency_key),
        None => (None, None),
    };
    let fingerprint = serde_json::json!({ "intake_id": id, "version": version });

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            require_replay_visible(&mut ctx.tx, org_id, &authed.principal, &stored).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let intake = ewaste::get_intake(&mut ctx.tx, org_id, id).await?;
        let client =
            require_intake_client(&mut ctx.tx, org_id, intake.client_id, &authed.principal).await?;
        let streamable = can_stream(&client);
        // The version is checked before the close writes, and the same lock
        // serializes a concurrent child write, so the detail below describes
        // the state this close produced.
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version,
            },
        )
        .await?;
        let prior = prior_detail(&mut ctx.tx, org_id, id, write_seq).await?;
        ewaste::close_intake(&mut ctx.tx, org_id, user_id, id).await?;
        let detail = load_detail(&mut ctx.tx, org_id, id).await?;
        emit_if_streamable(
            &mut ctx,
            streamable,
            "ewaste.intake_closed",
            id,
            &serde_json::json!({
                "intake_id": id,
                "total_in_g": detail.balance.total_in_g,
                "total_out_g": detail.balance.total_out_g,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "close",
                seq: write_seq,
                before: Some(serde_json::to_value(&prior).unwrap_or(Value::Null)),
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: detail.intake.updated_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(IntakeMutationResponse { detail, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// GET /api/ewaste/dispositions — the org-wide register, filterable for the
/// accountant handoff.
#[utoipa::path(operation_id = "ewaste_list_dispositions", get, path = "/api/ewaste/dispositions", tag = "ewaste", security(("bearer" = [])),
    params(ListDispositionsQuery),
    responses((status = 200, body = [EwasteDisposition])))]
pub async fn list_dispositions(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<ListDispositionsQuery>,
) -> ApiResult<Json<Vec<EwasteDisposition>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    if let Some(pathway) = q.pathway.as_deref() {
        if !ewaste::is_valid_pathway(pathway) {
            return Err(ApiError::Validation(format!(
                "pathway must be one of {}",
                ewaste::PATHWAYS.join(", ")
            )));
        }
    }
    if let (Some(from), Some(to)) = (q.from, q.to) {
        if from > to {
            return Err(ApiError::Validation("from is after to".into()));
        }
    }
    let cursor_parts = [
        q.cursor_disposed_at.is_some(),
        q.cursor_created_at.is_some(),
        q.cursor_id.is_some(),
    ];
    let cursor = if cursor_parts.iter().any(|present| *present) {
        match (q.cursor_disposed_at, q.cursor_created_at, q.cursor_id) {
            (Some(disposed_at), Some(created_at), Some(id)) => Some((disposed_at, created_at, id)),
            _ => {
                return Err(ApiError::Validation(
                    "a cursor needs cursor_disposed_at, cursor_created_at and cursor_id together"
                        .into(),
                ))
            }
        }
    } else {
        None
    };
    let org_id = authed.principal.user.org_id;
    let filter = DispositionFilter {
        client_id: q.client_id,
        pathway: q.pathway.as_deref().map(str::trim).map(str::to_string),
        from: q.from,
        to: q.to,
        limit: q.limit.unwrap_or(LIST_LIMIT).clamp(1, LIST_LIMIT),
        cursor,
        visibility: Some(visibility_of(&authed.principal)),
    };
    // One transaction for the scope check and the read (see `list`).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    if let Some(client_id) = q.client_id {
        require_intake_client_visible(&mut tx, org_id, client_id, &authed.principal).await?;
    }
    let rows = ewaste::list_dispositions_for_org(&mut tx, org_id, &filter).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}
