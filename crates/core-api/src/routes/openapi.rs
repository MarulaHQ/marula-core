//! OpenAPI document assembly + Scalar UI. The spec is generated from the
//! handlers — never hand-written (§2 tooling doctrine). The generated TS
//! SDK (packages/sdk) is produced from this same document.
//!
//! ## Versioning stance (TASKLIST U1, policy §2/§3)
//!
//! `/api/v1` is the CONTRACT: [`canonicalize_paths`] rewrites every
//! documented `/api/*` key onto its `/api/v1` twin at assembly time so the
//! served document and the committed SDK snapshot describe v1 exclusively.
//! Legacy aliases (see routes::mod) remain reachable at runtime but are
//! deliberately ABSENT from the spec — an alias is removed no earlier than
//! the next major version once its deprecation ships in release notes.
//!
//! Non-purpose: no multi-version documentation yet. When a second API
//! generation eventually exists, its paths must be listed alongside v1's
//! (never swapped); if any future handler ever annotates itself with a
//! versioned literal (`/api/vN/...`, N ≠ unset), it MUST be excluded here —
//! the rewrite therefore skips keys that already start with `/api/v`.

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
        crate::routes::board_access::nav,
        crate::routes::board_access::list_groups,
        crate::routes::board_access::create_group,
        crate::routes::board_access::update_group,
        crate::routes::board_access::delete_group,
        crate::routes::board_access::assign_group,
        crate::routes::board_access::list_members,
        crate::routes::board_access::upsert_member,
        crate::routes::board_access::remove_member,
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
        crate::routes::custom_fields::list_defs,
        crate::routes::custom_fields::create_def,
        crate::routes::custom_fields::update_def,
        crate::routes::custom_fields::delete_def,
        crate::routes::custom_fields::set_ticket_value,
        crate::routes::custom_fields::clear_ticket_value,
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
        crate::routes::time::get_settings,
        crate::routes::time::put_settings,
        crate::routes::time::my_gaps,
        crate::routes::time::user_gaps,
        crate::routes::notifications::list,
        crate::routes::notifications::unread_count,
        crate::routes::notifications::mark_read,
        crate::routes::notifications::mark_read_bulk,
        crate::routes::notifications::mark_all_read,
        crate::routes::notifications::prefs_get,
        crate::routes::notifications::prefs_update,
        crate::routes::audit::list,
        crate::routes::reports::report,
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
        crate::routes::manifest::get,
        crate::routes::internal::sla_verdict,
        crate::routes::internal::effects,
        crate::routes::settings::get_registry,
        crate::routes::settings::org_values_get,
        // Restored: the PUT half of /api/settings/org was missing from this
        // list while the route stayed registered, so snapshots silently
        // lost it and generated SDK types rejected the settings-hub writes.
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
        crate::routes::webhook_admin::list,
        crate::routes::webhook_admin::create,
        crate::routes::webhook_admin::update,
        crate::routes::webhook_admin::remove,
        crate::routes::webhook_admin::deliveries,
        crate::routes::webhook_admin::test_fire,
        crate::routes::api_keys::list,
        crate::routes::api_keys::create,
        crate::routes::api_keys::revoke,
        crate::routes::scheduling::create_appointment,
        crate::routes::scheduling::get_appointment,
        crate::routes::scheduling::patch_appointment,
        crate::routes::scheduling::move_appointment,
        crate::routes::scheduling::cancel_appointment,
        crate::routes::scheduling::conflict_check,
        crate::routes::calendar::calendar_day,
        crate::routes::calendar::calendar_week,
        crate::routes::calendar::calendar_team,
        crate::routes::dispatch::dispatch_day,
        crate::routes::dispatch::bulk_move,
        crate::routes::dispatch::assign,
        crate::routes::availability::list_availability,
        crate::routes::availability::create_availability,
        crate::routes::availability::delete_availability,
        crate::routes::availability::list_on_call,
        crate::routes::availability::create_on_call,
        crate::routes::availability::delete_on_call,
        crate::routes::availability::list_exclusions,
        crate::routes::availability::create_exclusion,
        crate::routes::availability::delete_exclusion,
        crate::routes::search::search,
        crate::routes::imports::dry_run,
        crate::routes::imports::create_batch,
        crate::routes::imports::post_chunk,
        crate::routes::imports::finish_batch,
        crate::routes::imports::get_batch,
        crate::routes::imports::list_batches,
        crate::routes::export::create,
        crate::routes::export::download,
        crate::routes::attachments::upload,
        crate::routes::attachments::list_for_ticket,
        crate::routes::attachments::list_for_comment,
        crate::routes::attachments::download,
        crate::routes::attachments::remove,
        crate::routes::portal::list_attachments,
        crate::routes::portal::download_attachment,
    ),
    components(schemas(
        // import framework (TASKLIST D2)
        crate::routes::imports::DryRunRequest,
        crate::routes::imports::CreateBatchRequest,
        crate::routes::imports::ChunkRequest,
        crate::routes::imports::RowsPayload,
        crate::routes::imports::RunReportResponse,
        crate::routes::imports::ChunkSummaryWire,
        crate::routes::imports::BatchCreatedResponse,
        crate::routes::imports::BatchViewResponse,
        core_domain::import_core::RowReport,
        core_domain::import_core::RowOutcome,
        core_domain::import_core::EntityKind,
        core_domain::import_core::ConflictPolicy,
        core_domain::import_core::RowFormat,
        // org-data export (TASKLIST D1, generalizes the U5 manifest)
        crate::routes::export::ExportCreatedResponse,
        core_domain::entities::export::ExportBundle,
        core_domain::entities::export::ExportSections,
        core_domain::entities::export::ExportOrg,
        core_domain::entities::export::ExportClient,
        core_domain::entities::export::ExportSite,
        core_domain::entities::export::ExportContact,
        core_domain::entities::export::ExportTicket,
        core_domain::entities::export::ExportComment,
        core_domain::entities::export::ExportTimeEntry,
        core_domain::entities::export::ExportFieldValue,
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
        crate::routes::board_access::BoardNavEntry,
        crate::routes::board_access::BoardMemberView,
        crate::routes::board_access::CreateGroupRequest,
        crate::routes::board_access::UpdateGroupRequest,
        crate::routes::board_access::AssignGroupRequest,
        crate::routes::board_access::UpsertMemberRequest,
        crate::routes::tickets::CreateTicketRequest,
        crate::routes::tickets::PatchTicketRequest,
        crate::routes::tickets::ChangeStatusRequest,
        crate::routes::tickets::AddCommentRequest,
        crate::routes::tickets::AddChecklistItemRequest,
        crate::routes::tickets::SetChecklistItemRequest,
        crate::routes::tickets::TicketListQuery,
        crate::routes::tickets::CommentListQuery,
        crate::routes::tickets::TicketChecklistView,
        crate::routes::tickets::MyWorkItem,
        crate::routes::custom_fields::FieldDefView,
        crate::routes::custom_fields::CreateFieldDefRequest,
        crate::routes::custom_fields::UpdateFieldDefRequest,
        crate::routes::custom_fields::FieldKindWire,
        crate::routes::custom_fields::SetTicketFieldValueRequest,
        crate::routes::custom_fields::TicketFieldInput,
        crate::routes::custom_fields::TicketCustomFieldValue,
        crate::routes::custom_fields::TicketWithFields,
        crate::routes::checklists::ChecklistTemplateView,
        crate::routes::checklists::PutChecklistTemplateRequest,
        crate::routes::timers::StartTimerRequest,
        crate::routes::timers::PauseTimerRequest,
        crate::routes::timers::StopTimerRequest,
        crate::routes::timers::CreateTimeEntryRequest,
        crate::routes::timers::ListEntriesQuery,
        crate::routes::time::TimeSettingsResponse,
        crate::routes::time::UpdateTimeSettingsRequest,
        crate::routes::time::GapReportResponse,
        crate::routes::time::GapDayView,
        crate::routes::time::GapReportQuery,
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
        core_domain::entities::BoardGroup,
        core_domain::entities::BoardMember,
        core_domain::policy::BoardRole,
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
        // instant-first write-path contract (TASKLIST IF1–IF3)
        crate::instant::MutationParams,
        crate::instant::MutationMeta,
        crate::routes::tickets::TicketMutationResponse,
        crate::routes::tickets::TicketStatusMutationResponse,
        crate::routes::tickets::TicketCommentMutationResponse,
        crate::routes::clients::ClientMutationResponse,
        crate::routes::timers::TimerMutationResponse,
        crate::routes::timers::TimeEntryMutationResponse,
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
        // customization manifest (TASKLIST U5, policy §5)
        core_domain::entities::manifest::CustomizationManifest,
        core_domain::entities::manifest::ManifestSections,
        core_domain::entities::manifest::ManifestSla,
        core_domain::entities::manifest::ManifestCustomField,
        core_domain::entities::manifest::ManifestBoard,
        core_domain::entities::manifest::ManifestBoardStatus,
        core_domain::entities::manifest::ManifestTransition,
        core_domain::entities::manifest::ManifestAutomationRule,
        core_domain::entities::manifest::ManifestWebhook,
        core_domain::entities::manifest::ManifestApiKey,
        core_domain::entities::manifest::ManifestSetting,
        core_domain::entities::manifest::ManifestSlaCalendar,
        core_domain::entities::manifest::ManifestSlaPolicy,
        core_domain::entities::manifest::ManifestEscalationStep,
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
        crate::routes::webhook_admin::WebhookView,
        crate::routes::webhook_admin::WebhookList,
        crate::routes::webhook_admin::WebhookSecretResponse,
        crate::routes::webhook_admin::CreateWebhookRequest,
        crate::routes::webhook_admin::UpdateWebhookRequest,
        crate::routes::webhook_admin::DeliveryAttemptView,
        crate::routes::webhook_admin::DeliveryPage,
        crate::routes::webhook_admin::DeliveryQuery,
        crate::routes::webhook_admin::TestFireResponse,
        crate::routes::api_keys::ApiKeyView,
        crate::routes::api_keys::ApiKeyList,
        crate::routes::api_keys::CreateApiKeyRequest,
        crate::routes::api_keys::ApiKeyCreatedResponse,
        // scheduling (TASKLIST 2.12)
        core_domain::entities::Appointment,
        core_domain::entities::AppointmentKind,
        core_domain::entities::AppointmentStatus,
        core_domain::entities::AvailabilityWindow,
        core_domain::entities::OnCallBlock,
        core_domain::entities::CalendarExclusion,
        crate::routes::scheduling::CreateAppointmentRequest,
        crate::routes::scheduling::PatchAppointmentRequest,
        crate::routes::scheduling::MoveAppointmentRequest,
        crate::routes::scheduling::ConflictCheckRequest,
        crate::routes::scheduling::ConflictCheckResponse,
        crate::routes::scheduling::ConflictView,
        // dispatch board (TASKLIST 2.19)
        crate::routes::dispatch::DispatchDayQuery,
        crate::routes::dispatch::DispatchDayView,
        crate::routes::dispatch::DispatchAppointment,
        crate::routes::dispatch::TechnicianView,
        crate::routes::dispatch::BulkMoveRequest,
        crate::routes::dispatch::MoveSpec,
        crate::routes::dispatch::BulkMoveResponse,
        crate::routes::dispatch::AssignRequest,
        crate::routes::calendar::CalendarViewResponse,
        crate::routes::calendar::CalendarItem,
        crate::routes::calendar::DayWeekQuery,
        crate::routes::calendar::TeamRangeQuery,
        crate::routes::availability::CreateAvailabilityRequest,
        crate::routes::availability::AvailabilityListResponse,
        crate::routes::availability::AvailabilityQuery,
        crate::routes::availability::CreateOnCallRequest,
        crate::routes::availability::CreateExclusionRequest,
        crate::routes::availability::RangeParams,
        crate::routes::availability::OwnedDeleteQuery,
        crate::routes::search::SearchQuery,
        crate::routes::search::SearchResponse,
        crate::routes::search::SearchResult,
        // attachments (TASKLIST 2.14)
        core_domain::entities::Attachment,
        core_domain::entities::AttachmentKind,
        crate::routes::portal::PortalAttachmentView,
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
        (name = "search", description = "Unified FTS search across tickets, comments and clients"),
        (name = "audit", description = "Append-only audit log (admin)"),
        (name = "sla", description = "SLA calendars, policies, escalation ladders (admin)"),
        (name = "automations", description = "Event-triggered automation rules (admin)"),
        (name = "portal", description = "Client portal v1: login, own-client tickets, replies"),
        (name = "portal-admin", description = "Staff administration of portal users + board exposure"),
        (name = "integrations", description = "Signed webhooks OUT + scoped API keys (plugin runtime v1)"),
        (name = "scheduling", description = "Appointments, availability, on-call, calendar views"),
        (name = "attachments", description = "File attachments on tickets/comments (ObjectStore-backed)"),
    )
)]
pub struct ApiDoc;

/// Rewrites documented path keys onto the canonical `/api/v1` namespace.
///
/// Keys starting with `/api/` (the unversioned literals carried by the
/// `#[utoipa::path]` annotations) become their `/api/v1` twins; keys
/// already versioned (`/api/v…`) pass through untouched so future handler
/// generations cannot be silently double-prefixed. Non-`/api` keys — the
/// top-level `/openapi.json`, `/docs`, and internal machine surfaces
/// outside the API namespace if any were ever listed — are ignored.
///
/// The key COUNT is preserved exactly: this is a rename, not a filter, so
/// a drift between documented operations and either mount cannot hide in
/// an off-by-one.
fn canonicalize_paths(spec: &mut serde_json::Value) {
    let Some(paths) = spec.get_mut("paths").and_then(|p| p.as_object_mut()) else {
        return;
    };
    let renamed: Vec<String> = paths
        .keys()
        .filter(|k| k.starts_with("/api/") && !k.starts_with("/api/v"))
        .cloned()
        .collect();
    for old in renamed {
        let new = format!("/api/v1{}", &old["/api".len()..]);
        let entry = paths.remove(&old).expect("key observed above");
        paths.insert(new, entry);
    }
}

pub fn openapi_json() -> serde_json::Value {
    let mut value = ApiDoc::openapi()
        .to_pretty_json()
        .unwrap_or_else(|_| "{}".to_string())
        .parse()
        .unwrap_or_else(|_| serde_json::json!({}));
    canonicalize_paths(&mut value);
    value
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
