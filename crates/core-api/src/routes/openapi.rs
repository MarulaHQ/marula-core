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
        crate::routes::boards::update_status,
        crate::routes::boards::delete_status,
        crate::routes::boards::list_transitions,
        crate::routes::boards::set_transitions,
        crate::routes::boards::toggle_favorite,
        crate::routes::checklists::get_template,
        crate::routes::checklists::put_template,
        crate::routes::checklists::delete_template,
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
        crate::routes::notifications::mark_read_bulk,
        crate::routes::notifications::mark_all_read,
        crate::routes::notifications::prefs_get,
        crate::routes::notifications::prefs_update,
        crate::routes::audit::list,
        crate::auth::handlers::list_sessions,
        crate::auth::handlers::revoke_session,
        crate::auth::mfa_handlers::totp_enroll_begin,
        crate::auth::mfa_handlers::totp_enroll_verify,
        crate::auth::mfa_handlers::totp_disable,
        crate::auth::mfa_handlers::regenerate_recovery_codes,
        crate::auth::mfa_handlers::mfa_verify,
        crate::auth::mfa_handlers::webauthn_register_begin,
        crate::auth::mfa_handlers::webauthn_register_finish,
        crate::auth::mfa_handlers::webauthn_credentials_list,
        crate::auth::mfa_handlers::webauthn_credential_remove,
        crate::auth::mfa_handlers::webauthn_login_begin,
        crate::auth::mfa_handlers::webauthn_login_finish,
        crate::routes::oidc::providers_list,
        crate::routes::oidc::providers_upsert,
        crate::routes::oidc::providers_delete,
        crate::routes::oidc::providers_public,
        crate::routes::oidc::login_redirect,
        crate::routes::oidc::callback,
        crate::routes::oidc::link_begin,
        crate::routes::oidc::sso_exchange,
        crate::routes::tickets::sla_status,
        crate::routes::sla_admin::list_calendars,
        crate::routes::sla_admin::create_calendar,
        crate::routes::sla_admin::update_calendar,
        crate::routes::sla_admin::delete_calendar,
        crate::routes::sla_admin::list_policies,
        crate::routes::sla_admin::get_ladder,
        crate::routes::sla_admin::replace_ladder,
        crate::routes::sla_admin::list_rules,
        crate::routes::sla_admin::create_rule,
        crate::routes::sla_admin::update_rule,
        crate::routes::sla_admin::delete_rule,
        crate::routes::internal::sla_verdict,
        crate::routes::internal::effects,
        crate::routes::settings::get_registry,
        crate::routes::settings::org_values_get,
        crate::routes::settings::org_value_put,
        crate::routes::settings::user_values_get,
        crate::routes::settings::user_value_put,
        crate::routes::settings::org_profile_get,
        crate::routes::settings::org_profile_patch,
        crate::routes::portal::login,
        crate::routes::portal::refresh,
        crate::routes::portal::me,
        crate::routes::portal::list_tickets,
        crate::routes::portal::get_ticket,
        crate::routes::portal::reply,
        crate::routes::portal_admin::list_portal_users,
        crate::routes::portal_admin::create_portal_user,
        crate::routes::portal_admin::patch_portal_user,
        crate::routes::portal_admin::delete_portal_user,
        crate::routes::portal_admin::list_board_exposures,
        crate::routes::portal_admin::put_board_exposure,
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
        crate::routes::boards::UpdateStatusRequest,
        crate::routes::boards::SetTransitionsRequest,
        crate::routes::boards::BoardTransitionsResponse,
        crate::routes::boards::FavoriteRequest,
        crate::routes::tickets::CreateTicketRequest,
        crate::routes::tickets::PatchTicketRequest,
        crate::routes::tickets::ChangeStatusRequest,
        crate::routes::tickets::AddCommentRequest,
        crate::routes::tickets::AddChecklistItemRequest,
        crate::routes::tickets::SetChecklistItemRequest,
        crate::routes::tickets::TicketListQuery,
        crate::routes::tickets::CommentListQuery,
        crate::routes::tickets::TicketChecklistView,
        crate::routes::checklists::ChecklistTemplateView,
        crate::routes::checklists::PutChecklistTemplateRequest,
        crate::routes::timers::StartTimerRequest,
        crate::routes::timers::PauseTimerRequest,
        crate::routes::timers::StopTimerRequest,
        crate::routes::timers::CreateTimeEntryRequest,
        crate::routes::timers::ListEntriesQuery,
        core_domain::entities::Ticket,
        core_domain::entities::TicketComment,
        core_domain::entities::ChecklistItem,
        core_domain::entities::ChecklistProgress,
        core_domain::entities::ChecklistTemplate,
        core_domain::entities::ChecklistTemplateItem,
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
        core_domain::entities::NotificationKind,
        crate::routes::notifications::NotificationPage,
        crate::routes::notifications::UnreadCount,
        crate::routes::notifications::MarkReadBulkRequest,
        crate::routes::notifications::MarkReadResult,
        crate::routes::notifications::PreferenceView,
        crate::routes::notifications::UpdatePreferenceRequest,
        core_domain::entities::AuditEvent,
        core_domain::entities::Actor,
        core_domain::entities::TicketSlaState,
        core_domain::entities::SlaCalendar,
        core_domain::entities::BusinessCalendar,
        core_domain::entities::MaintenanceWindow,
        core_domain::entities::EscalationStep,
        core_domain::status::TicketStatusCategory,
        core_domain::status::Priority,
        core_domain::status::NoteKind,
        core_domain::workflow::TransitionEdge,
        crate::auth::handlers::LoginResponse,
        crate::auth::handlers::SessionInfo,
        crate::auth::mfa_handlers::TotpEnrollBeginResponse,
        crate::auth::mfa_handlers::RecoveryCodesResponse,
        crate::auth::mfa_handlers::TotpEnrollVerifyRequest,
        crate::auth::mfa_handlers::TotpDisableRequest,
        crate::auth::mfa_handlers::RegenerateRecoveryRequest,
        crate::auth::mfa_handlers::MfaVerifyRequest,
        crate::auth::mfa_handlers::MfaTokenRequest,
        crate::auth::mfa_handlers::WebauthnBeginResponse,
        crate::auth::mfa_handlers::WebauthnCredentialInfo,
        crate::routes::oidc::OidcProviderInfo,
        crate::routes::oidc::UpsertProviderRequest,
        crate::routes::oidc::PublicProvider,
        crate::routes::oidc::LinkBeginResponse,
        crate::routes::oidc::SsoExchangeRequest,
        crate::routes::oidc::SsoExchangeResponse,
        crate::routes::audit::AuditPage,
        core_domain::automations::AutomationCondition,
        core_domain::automations::AutomationAction,
        crate::routes::tickets::TicketSlaStatus,
        crate::routes::sla_admin::CreateCalendarRequest,
        crate::routes::sla_admin::UpdateCalendarRequest,
        crate::routes::sla_admin::PolicyView,
        crate::routes::sla_admin::LadderStepSpec,
        crate::routes::sla_admin::ReplaceLadderRequest,
        crate::routes::sla_admin::CreateRuleRequest,
        crate::routes::sla_admin::UpdateRuleRequest,
        crate::routes::internal::SlaVerdictRequest,
        crate::routes::internal::VerdictResponse,
        crate::routes::internal::EffectsRequest,
        crate::routes::settings::RegistryResponse,
        crate::routes::settings::DeclarationView,
        crate::routes::settings::ValueView,
        crate::routes::settings::UpdateSettingRequest,
        crate::routes::settings::OrgProfileResponse,
        crate::routes::settings::PatchOrgRequest,
        core_domain::settings::SettingKind,
        core_domain::settings::SettingValue,
        crate::routes::portal::PortalLoginRequest,
        crate::routes::portal::PortalRefreshRequest,
        crate::routes::portal::PortalLoginResponse,
        crate::routes::portal::PortalProfile,
        crate::routes::portal::PortalTicketListQuery,
        crate::routes::portal::PortalTicketDetail,
        crate::routes::portal::PortalReplyRequest,
        crate::routes::portal_admin::CreatePortalUserRequest,
        crate::routes::portal_admin::PatchPortalUserRequest,
        crate::routes::portal_admin::BoardExposureView,
        crate::routes::portal_admin::PutExposureRequest,
        core_domain::entities::PortalUser,
        core_domain::entities::PortalRole,
        core_domain::entities::PortalUserStatus,
    ),
    ),
    tags(
        (name = "auth", description = "Registration, login, token lifecycle"),
        (name = "clients", description = "Client → site → contact → asset hierarchy"),
        (name = "boards", description = "Boards and semantic-category-bound statuses"),
        (name = "tickets", description = "Ticketing: lifecycle, notes, checklists"),
        (name = "timers", description = "Single-active-timer workflow + time entries"),
        (name = "notifications", description = "Notification center"),
        (name = "settings", description = "Typed settings registry: org + user scoped values"),
        (name = "audit", description = "Append-only audit log with filters + cursor pagination (admin)"),
        (name = "audit", description = "Append-only audit log (admin)"),
        (name = "sla", description = "SLA calendars, policies, escalation ladders (admin)"),
        (name = "automations", description = "Event-triggered automation rules (admin)"),
        (name = "portal", description = "Client portal v1: login, own-client tickets, replies"),
        (name = "portal-admin", description = "Staff administration of portal users + board exposure"),
    )
)]
pub struct ApiDoc;

pub fn openapi_json() -> serde_json::Value {
    ApiDoc::openapi()
        .to_pretty_json()
        .unwrap_or_else(|_| "{}".to_string())
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
    axum::response::Html(
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
            .to_string(),
    )
}

/// Token pair returned by all auth flows.
#[derive(Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
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
