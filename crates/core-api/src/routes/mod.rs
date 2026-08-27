//! HTTP routes: router assembly and the OpenAPI document (utoipa derives
//! the spec from the handlers; Scalar serves interactive docs from it).

pub mod api_keys;
pub mod attachments;
pub mod audit;
pub mod availability;
pub mod board_access;
pub mod boards;
pub mod calendar;
pub mod checklists;
pub mod clients;
pub mod custom_fields;
pub mod dispatch;
pub mod email_admin;
pub mod imports;
pub mod internal;
pub mod notifications;
pub mod oidc;
pub mod openapi;
pub mod portal;
pub mod portal_admin;
pub mod scheduling;
pub mod search;
pub mod settings;
pub mod sla_admin;
pub mod tickets;
pub mod time;
pub mod timers;
pub mod webhook_admin;
pub mod webhooks;

use crate::auth::handlers as auth_handlers;
use crate::auth::mfa_handlers as mfa;
use crate::auth::middleware::require_auth;
use crate::state::AppState;
use axum::{
    middleware,
    routing::{get, post, put},
    Router,
};

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/register", post(auth_handlers::register))
        .route("/api/auth/login", post(auth_handlers::login))
        .route("/api/auth/refresh", post(auth_handlers::refresh))
        .route("/api/auth/logout", post(auth_handlers::logout))
        // MFA completion of a password login.
        .route("/api/auth/mfa/verify", post(mfa::mfa_verify))
        .route(
            "/api/auth/mfa/webauthn/begin",
            post(mfa::webauthn_login_begin),
        )
        .route(
            "/api/auth/mfa/webauthn/finish",
            post(mfa::webauthn_login_finish),
        )
        // OIDC SSO (config-driven; no providers configured ⇒ 404s).
        .route(
            "/api/auth/oidc/providers/public",
            get(oidc::providers_public),
        )
        .route("/api/auth/oidc/{name}/login", get(oidc::login_redirect))
        .route("/api/auth/oidc/{name}/callback", get(oidc::callback))
        .route("/api/sso/exchange", post(oidc::sso_exchange))
        // Machine-to-machine surfaces with their own auth (HMAC webhook,
        // internal worker token) — never behind JWT middleware.
        .route("/api/webhooks/email-inbound", post(webhooks::email_inbound))
        .route(
            "/api/internal/email/outbound/claim",
            post(internal::claim_outbound),
        )
        .route(
            "/api/internal/email/outbound/results",
            post(internal::report_results),
        )
        // Machine surface for the worker (token-gated inside the handlers,
        // NOT behind the JWT middleware — §4.1: worker effects enter
        // through core-api, never direct DB writes).
        .route("/api/internal/sla/verdict", post(internal::sla_verdict))
        .route("/api/internal/effects", post(internal::effects))
        .route(
            "/api/internal/time/gap-audit/run",
            post(internal::run_gap_audit),
        )
        .route(
            "/api/internal/webhook/deliveries",
            post(internal::webhook_deliveries),
        );

    // Client-portal v1: public auth endpoints + a DEDICATED middleware for
    // the authenticated surface. Portal tokens (kind:"portal") never pass
    // require_auth, staff tokens never pass require_portal_auth.
    let portal_protected = Router::new()
        .route("/api/portal/me", get(portal::me))
        .route("/api/portal/tickets", get(portal::list_tickets))
        .route("/api/portal/tickets/{id}", get(portal::get_ticket))
        .route("/api/portal/tickets/{id}/reply", post(portal::reply))
        // Portal-visible attachments (TASKLIST 2.14): metadata + streamed
        // download only; hidden/internal files are indistinguishable 404s.
        .route(
            "/api/portal/tickets/{id}/attachments",
            get(portal::list_attachments),
        )
        .route(
            "/api/portal/tickets/{id}/attachments/{attachment_id}/download",
            get(portal::download_attachment),
        )
        // Stamps org_id onto the current request span (telemetry) once the
        // portal auth middleware below has resolved it.
        .layer(middleware::from_fn(crate::otel_http::record_org))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            portal::require_portal_auth,
        ));
    let public = public
        .route("/api/portal/login", post(portal::login))
        .route("/api/portal/refresh", post(portal::refresh))
        .merge(portal_protected);

    let protected = Router::new()
        .route("/api/me", get(auth_handlers::me))
        // session management (security settings)
        .route("/api/auth/sessions", get(auth_handlers::list_sessions))
        .route(
            "/api/auth/sessions/{id}/revoke",
            post(auth_handlers::revoke_session),
        )
        // TOTP enrollment + recovery codes
        .route(
            "/api/auth/mfa/totp/enroll-begin",
            post(mfa::totp_enroll_begin),
        )
        .route(
            "/api/auth/mfa/totp/enroll-verify",
            post(mfa::totp_enroll_verify),
        )
        .route("/api/auth/mfa/totp/disable", post(mfa::totp_disable))
        .route(
            "/api/auth/mfa/recovery-codes/regenerate",
            post(mfa::regenerate_recovery_codes),
        )
        // WebAuthn credential management
        .route(
            "/api/auth/mfa/webauthn/register/begin",
            post(mfa::webauthn_register_begin),
        )
        .route(
            "/api/auth/mfa/webauthn/register/finish",
            post(mfa::webauthn_register_finish),
        )
        .route(
            "/api/auth/mfa/webauthn/credentials",
            get(mfa::webauthn_credentials_list),
        )
        .route(
            "/api/auth/mfa/webauthn/credentials/{id}/remove",
            post(mfa::webauthn_credential_remove),
        )
        // OIDC provider admin + explicit linking
        .route("/api/auth/oidc/providers", get(oidc::providers_list))
        .route(
            "/api/auth/oidc/providers/{name}",
            put(oidc::providers_upsert).delete(oidc::providers_delete),
        )
        .route("/api/auth/oidc/{name}/link", post(oidc::link_begin))
        // clients
        .route("/api/clients", get(clients::list).post(clients::create))
        .route("/api/clients/{id}", get(clients::get_one))
        .route(
            "/api/clients/{id}/sites",
            get(clients::list_sites).post(clients::create_site),
        )
        .route(
            "/api/clients/{id}/contacts",
            get(clients::list_contacts).post(clients::create_contact),
        )
        .route(
            "/api/clients/{id}/assets",
            get(clients::list_assets).post(clients::create_asset),
        )
        // boards + statuses + workflow customization + organization/RBAC
        .route("/api/boards", get(boards::list).post(boards::create))
        .route("/api/boards/nav", get(board_access::nav))
        .route(
            "/api/board-groups",
            get(board_access::list_groups).post(board_access::create_group),
        )
        .route(
            "/api/board-groups/{id}",
            axum::routing::patch(board_access::update_group).delete(board_access::delete_group),
        )
        .route("/api/boards/{id}/group", put(board_access::assign_group))
        .route(
            "/api/boards/{id}/members",
            get(board_access::list_members).put(board_access::upsert_member),
        )
        .route(
            "/api/boards/{id}/members/{user_id}",
            axum::routing::delete(board_access::remove_member),
        )
        .route(
            "/api/boards/{id}/statuses",
            get(boards::list_statuses).post(boards::add_status),
        )
        .route(
            "/api/boards/{id}/statuses/{status_id}",
            axum::routing::patch(boards::update_status).delete(boards::delete_status),
        )
        .route(
            "/api/boards/{id}/transitions",
            get(boards::list_transitions).put(boards::set_transitions),
        )
        .route("/api/boards/{id}/favorite", post(boards::toggle_favorite))
        .route(
            "/api/boards/{id}/checklist-template",
            get(checklists::get_template)
                .put(checklists::put_template)
                .delete(checklists::delete_template),
        )
        // custom fields / dynamic forms (TASKLIST 2.17)
        .route(
            "/api/boards/{id}/fields",
            get(custom_fields::list_defs).post(custom_fields::create_def),
        )
        .route(
            "/api/boards/{id}/fields/{field_id}",
            axum::routing::patch(custom_fields::update_def).delete(custom_fields::delete_def),
        )
        .route(
            "/api/tickets/{id}/fields/{field_id}",
            put(custom_fields::set_ticket_value).delete(custom_fields::clear_ticket_value),
        )
        // tickets
        .route("/api/tickets", get(tickets::list).post(tickets::create))
        .route("/api/tickets/my-work", get(tickets::my_work))
        .route(
            "/api/tickets/{id}",
            get(tickets::get_one).patch(tickets::patch),
        )
        .route("/api/tickets/{id}/status", post(tickets::change_status))
        .route("/api/tickets/{id}/sla", get(tickets::sla_status))
        // sla configuration (admin)
        .route(
            "/api/sla/calendars",
            get(sla_admin::list_calendars).post(sla_admin::create_calendar),
        )
        .route(
            "/api/sla/calendars/{id}",
            axum::routing::patch(sla_admin::update_calendar).delete(sla_admin::delete_calendar),
        )
        .route("/api/sla/policies", get(sla_admin::list_policies))
        .route(
            "/api/sla/policies/{id}/ladder",
            get(sla_admin::get_ladder).put(sla_admin::replace_ladder),
        )
        // automation rules (admin)
        .route(
            "/api/automation-rules",
            get(sla_admin::list_rules).post(sla_admin::create_rule),
        )
        .route(
            "/api/automation-rules/{id}",
            axum::routing::patch(sla_admin::update_rule).delete(sla_admin::delete_rule),
        )
        .route(
            "/api/tickets/{id}/comments",
            get(tickets::list_comments).post(tickets::add_comment),
        )
        // attachments (TASKLIST 2.14): raw-body upload needs a per-route
        // limit override because axum's default caps bodies at 2 MB.
        .route(
            "/api/tickets/{id}/attachments",
            get(attachments::list_for_ticket)
                .post(attachments::upload)
                .layer(axum::extract::DefaultBodyLimit::max(
                    state.config.max_upload_bytes.max(1024 * 1024) as usize,
                )),
        )
        .route(
            "/api/tickets/{id}/comments/{comment_id}/attachments",
            get(attachments::list_for_comment),
        )
        .route("/api/attachments/{id}/download", get(attachments::download))
        .route(
            "/api/attachments/{id}",
            axum::routing::delete(attachments::remove),
        )
        .route(
            "/api/tickets/{id}/checklist",
            get(tickets::get_checklist).post(tickets::add_checklist_item),
        )
        .route(
            "/api/tickets/{id}/checklist/{item_id}",
            post(tickets::set_checklist_item),
        )
        .route(
            "/api/tickets/{id}/time-entries",
            get(tickets::ticket_time_entries),
        )
        // timers (single-active-timer workflow)
        .route("/api/timers/active", get(timers::active))
        .route("/api/timers/working-set", get(timers::working_set))
        .route("/api/timers/start", post(timers::start))
        .route("/api/timers/pause", post(timers::pause))
        .route("/api/timers/resume", post(timers::resume))
        .route("/api/timers/stop", post(timers::stop))
        // time entries
        .route(
            "/api/time-entries",
            get(timers::list_entries).post(timers::create_entry),
        )
        // time discipline (settings + missing-time gap reports)
        .route(
            "/api/time-settings",
            get(time::get_settings).put(time::put_settings),
        )
        .route("/api/time-gaps/me", get(time::my_gaps))
        .route("/api/time-gaps", get(time::user_gaps))
        // notifications
        .route("/api/notifications", get(notifications::list))
        .route(
            "/api/notifications/unread-count",
            get(notifications::unread_count),
        )
        .route(
            "/api/notifications/mark-read",
            post(notifications::mark_read_bulk),
        )
        .route(
            "/api/notifications/read-all",
            post(notifications::mark_all_read),
        )
        .route(
            "/api/notifications/{id}/read",
            post(notifications::mark_read),
        )
        .route(
            "/api/notification-preferences",
            get(notifications::prefs_get).put(notifications::prefs_update),
        )
        // unified search (TASKLIST 2.16)
        .route("/api/search", get(search::search))
        // import framework (TASKLIST D2): org-admin gated CSV/JSON mapper.
        // The two payload-heavy posts raise axum's default body cap like
        // attachments do; chunk/batch ids flow through the path.
        .route(
            "/api/imports/dry-run",
            post(imports::dry_run).layer(axum::extract::DefaultBodyLimit::max(
                state
                    .config
                    .max_upload_bytes
                    .max(core_domain::import_core::MAX_CHUNK_BYTES as i64) as usize,
            )),
        )
        .route(
            "/api/imports/batches",
            get(imports::list_batches).post(imports::create_batch),
        )
        .route("/api/imports/batches/{batch_id}", get(imports::get_batch))
        .route(
            "/api/imports/batches/{batch_id}/chunks",
            post(imports::post_chunk).layer(axum::extract::DefaultBodyLimit::max(
                state
                    .config
                    .max_upload_bytes
                    .max(core_domain::import_core::MAX_CHUNK_BYTES as i64) as usize,
            )),
        )
        .route(
            "/api/imports/batches/{batch_id}/finish",
            post(imports::finish_batch),
        )
        // audit (admin)
        .route("/api/audit-events", get(audit::list))
        // plugin runtime v1: signed webhooks OUT + API keys (admin;
        // x-api-key machine auth is handled inside require_auth)
        .route(
            "/api/webhooks",
            get(webhook_admin::list).post(webhook_admin::create),
        )
        .route(
            "/api/webhooks/{id}",
            axum::routing::patch(webhook_admin::update).delete(webhook_admin::remove),
        )
        .route(
            "/api/webhooks/{id}/deliveries",
            get(webhook_admin::deliveries),
        )
        .route("/api/webhooks/{id}/test", post(webhook_admin::test_fire))
        .route("/api/api-keys", get(api_keys::list).post(api_keys::create))
        .route(
            "/api/api-keys/{id}",
            axum::routing::delete(api_keys::revoke),
        )
        // settings hub: typed registry + org/user scoped values + org profile
        .route("/api/settings/registry", get(settings::get_registry))
        .route(
            "/api/settings/org",
            get(settings::org_values_get).put(settings::org_value_put),
        )
        .route(
            "/api/settings/user",
            get(settings::user_values_get).put(settings::user_value_put),
        )
        .route(
            "/api/org",
            get(settings::org_profile_get).patch(settings::org_profile_patch),
        )
        // email inbound routing settings (admin)
        .route(
            "/api/email/inbound-settings",
            get(email_admin::get_inbound_settings).put(email_admin::put_inbound_settings),
        )
        .route(
            "/api/boards/{id}/inbound-address",
            put(email_admin::put_board_inbound_address),
        )
        // portal administration (staff, PortalAdmin grant)
        .route(
            "/api/clients/{id}/portal-users",
            get(portal_admin::list_portal_users).post(portal_admin::create_portal_user),
        )
        .route(
            "/api/portal-users/{id}",
            axum::routing::patch(portal_admin::patch_portal_user)
                .delete(portal_admin::delete_portal_user),
        )
        .route(
            "/api/clients/{id}/board-exposures",
            get(portal_admin::list_board_exposures).put(portal_admin::put_board_exposure),
        )
        // scheduling/calendars/availability (TASKLIST 2.12)
        .route("/api/appointments", post(scheduling::create_appointment))
        .route(
            "/api/appointments/{id}",
            get(scheduling::get_appointment).patch(scheduling::patch_appointment),
        )
        .route(
            "/api/appointments/{id}/move",
            post(scheduling::move_appointment),
        )
        .route(
            "/api/appointments/{id}/cancel",
            post(scheduling::cancel_appointment),
        )
        .route(
            "/api/scheduling/conflict-check",
            post(scheduling::conflict_check),
        )
        .route("/api/calendar/day", get(calendar::calendar_day))
        .route("/api/calendar/week", get(calendar::calendar_week))
        .route("/api/calendar/team", get(calendar::calendar_team))
        .route(
            "/api/availability",
            get(availability::list_availability).post(availability::create_availability),
        )
        .route(
            "/api/availability/{id}",
            axum::routing::delete(availability::delete_availability),
        )
        .route(
            "/api/on-call-blocks",
            get(availability::list_on_call).post(availability::create_on_call),
        )
        .route(
            "/api/on-call-blocks/{id}",
            axum::routing::delete(availability::delete_on_call),
        )
        .route(
            "/api/calendar-exclusions",
            get(availability::list_exclusions).post(availability::create_exclusion),
        )
        .route(
            "/api/calendar-exclusions/{id}",
            axum::routing::delete(availability::delete_exclusion),
        )
        // Stamps org_id onto the current request span (telemetry) once the
        // auth middleware below has resolved it.
        .layer(middleware::from_fn(crate::otel_http::record_org))
        // dispatch board (TASKLIST 2.19)
        .route("/api/dispatch/day", get(dispatch::dispatch_day))
        .route("/api/dispatch/bulk-move", post(dispatch::bulk_move))
        .route("/api/dispatch/assign", post(dispatch::assign))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    // Machine-to-machine effect surface (§4.1): token-guarded, NOT behind
    // the JWT middleware, and deliberately absent from OpenAPI/the SDK.
    let internal = Router::new()
        .route("/internal/search/upsert", post(internal::upsert))
        .route("/internal/search/delete", post(internal::delete));

    let docs = openapi::scalar_router();

    // Request telemetry (TASKLIST 2.18): outermost so the span covers the
    // entire pipeline; the route TEMPLATE + status + duration become span
    // attrs and `meridian.http.*` metrics. Unmatched routes collapse to
    // the constant "unmatched" — never raw paths (redact module docs).
    Router::new()
        .merge(public)
        .merge(protected)
        .merge(internal)
        .merge(docs)
        .layer(middleware::from_fn(crate::otel_http::track))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}
