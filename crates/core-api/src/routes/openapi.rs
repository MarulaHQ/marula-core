//! OpenAPI document assembly + Scalar UI. The spec is generated from the
//! handlers — never hand-written (§2 tooling doctrine). The generated TS
//! SDK (packages/sdk) is produced from this same document.

use crate::state::AppState;
use axum::{routing::get, Router};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "Meridian Core API", version = "0.1.0",
         description = "The single write surface for Project Meridian. \
                        All mutations flow through this API and emit domain events."),
    paths(
        crate::auth::handlers::register,
        crate::auth::handlers::login,
        crate::auth::handlers::refresh,
        crate::auth::handlers::logout,
        crate::auth::handlers::me,
        crate::routes::clients::list,
        crate::routes::clients::create,
        crate::routes::clients::get_one,
        crate::routes::clients::list_sites,
        crate::routes::clients::create_site,
        crate::routes::clients::list_contacts,
        crate::routes::clients::create_contact,
        crate::routes::clients::list_assets,
        crate::routes::clients::create_asset,
        crate::routes::boards::list,
        crate::routes::boards::create,
        crate::routes::boards::list_statuses,
        crate::routes::boards::add_status,
        crate::routes::boards::toggle_favorite,
        crate::routes::tickets::list,
        crate::routes::tickets::create,
        crate::routes::tickets::my_work,
        crate::routes::tickets::get_one,
        crate::routes::tickets::patch,
        crate::routes::tickets::change_status,
        crate::routes::tickets::list_comments,
        crate::routes::tickets::add_comment,
        crate::routes::tickets::get_checklist,
        crate::routes::tickets::add_checklist_item,
        crate::routes::tickets::set_checklist_item,
        crate::routes::tickets::ticket_time_entries,
        crate::routes::timers::active,
        crate::routes::timers::working_set,
        crate::routes::timers::start,
        crate::routes::timers::pause,
        crate::routes::timers::resume,
        crate::routes::timers::stop,
        crate::routes::timers::list_entries,
        crate::routes::timers::create_entry,
        crate::routes::notifications::list,
        crate::routes::notifications::unread_count,
        crate::routes::notifications::mark_read,
        crate::routes::audit::list,
    ),
    components(schemas(
        crate::auth::handlers::RegisterRequest,
        crate::auth::handlers::LoginRequest,
        crate::auth::handlers::RefreshRequest,
        crate::auth::handlers::MeResponse,
        crate::routes::openapi::TokenPair,
        crate::routes::openapi::HealthResponse,
        crate::routes::clients::CreateClientRequest,
        crate::routes::clients::CreateSiteRequest,
        crate::routes::clients::CreateContactRequest,
        crate::routes::clients::CreateAssetRequest,
        crate::routes::boards::CreateBoardRequest,
        crate::routes::boards::CreateStatusRequest,
        crate::routes::boards::FavoriteRequest,
        crate::routes::tickets::CreateTicketRequest,
        crate::routes::tickets::PatchTicketRequest,
        crate::routes::tickets::ChangeStatusRequest,
        crate::routes::tickets::AddCommentRequest,
        crate::routes::tickets::AddChecklistItemRequest,
        crate::routes::tickets::SetChecklistItemRequest,
        crate::routes::tickets::TicketListQuery,
        crate::routes::timers::StartTimerRequest,
        crate::routes::timers::PauseTimerRequest,
        crate::routes::timers::StopTimerRequest,
        crate::routes::timers::CreateTimeEntryRequest,
        crate::routes::timers::ListEntriesQuery,
        core_domain::entities::Ticket,
        core_domain::entities::TicketComment,
        core_domain::entities::ChecklistItem,
        core_domain::entities::Client,
        core_domain::entities::Site,
        core_domain::entities::Contact,
        core_domain::entities::DeviceAsset,
        core_domain::entities::Board,
        core_domain::entities::BoardStatus,
        core_domain::entities::TimerSession,
        core_domain::entities::TimerSegment,
        core_domain::entities::TimeEntry,
        core_domain::entities::Notification,
        core_domain::entities::AuditEvent,
        core_domain::entities::Actor,
        core_domain::entities::TicketSlaState,
        core_domain::status::TicketStatusCategory,
        core_domain::status::Priority,
        core_domain::status::NoteKind,
    ),
    ),
    tags(
        (name = "auth", description = "Registration, login, token lifecycle"),
        (name = "clients", description = "Client → site → contact → asset hierarchy"),
        (name = "boards", description = "Boards and semantic-category-bound statuses"),
        (name = "tickets", description = "Ticketing: lifecycle, notes, checklists"),
        (name = "timers", description = "Single-active-timer workflow + time entries"),
        (name = "notifications", description = "Notification center"),
        (name = "audit", description = "Append-only audit log (admin)"),
    )
)]
pub struct ApiDoc;

pub fn openapi_json() -> serde_json::Value {
    ApiDoc::openapi().to_pretty_json().unwrap_or_else(|_| "{}".to_string())
        .parse()
        .unwrap_or_else(|_| serde_json::json!({}))
}

pub fn scalar_router() -> Router<AppState> {
    Router::new()
        .route("/openapi.json", get(openapi_handler))
        .route("/docs", get(scalar_handler))
}

async fn openapi_handler() -> axum::Json<serde_json::Value> {
    axum::Json(openapi_json())
}

/// Scalar is served from its CDN against our local spec — no vendored UI.
async fn scalar_handler() -> axum::response::Html<String> {
    axum::response::Html(format!(
        r#"<!doctype html>
<html>
  <head>
    <title>Meridian API — Scalar</title>
    <meta charset="utf-8"/>
    <meta name="viewport" content="width=device-width, initial-scale=1"/>
  </head>
  <body>
    <script id="api-reference" type="application/json">[]</script>
    <script>
      document.getElementById('api-reference').dataset.url = '/openapi.json';
    </script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>"#
    ))
}

/// Token pair returned by all auth flows.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: uuid::Uuid,
    /// Access token TTL in seconds.
    pub expires_in: i64,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct HealthResponse {
    pub status: String,
}
