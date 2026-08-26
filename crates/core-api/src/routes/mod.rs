//! HTTP routes: router assembly and the OpenAPI document (utoipa derives
//! the spec from the handlers; Scalar serves interactive docs from it).

pub mod audit;
pub mod boards;
pub mod checklists;
pub mod clients;
pub mod email_admin;
pub mod internal;
pub mod notifications;
pub mod oidc;
pub mod openapi;
pub mod portal;
pub mod portal_admin;
pub mod settings;
pub mod sla_admin;
pub mod tickets;
pub mod timers;
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
        .route("/api/internal/effects", post(internal::effects));

    // Client-portal v1: public auth endpoints + a DEDICATED middleware for
    // the authenticated surface. Portal tokens (kind:"portal") never pass
    // require_auth, staff tokens never pass require_portal_auth.
    let portal_protected = Router::new()
        .route("/api/portal/me", get(portal::me))
        .route("/api/portal/tickets", get(portal::list_tickets))
        .route("/api/portal/tickets/{id}", get(portal::get_ticket))
        .route("/api/portal/tickets/{id}/reply", post(portal::reply))
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
        // boards + statuses + workflow customization
        .route("/api/boards", get(boards::list).post(boards::create))
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
        // audit (admin)
        .route("/api/audit-events", get(audit::list))
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
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let docs = openapi::scalar_router();

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(docs)
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}
