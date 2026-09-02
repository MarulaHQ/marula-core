//! HTTP routes: router assembly and the OpenAPI document (utoipa derives
//! the spec from the handlers; Scalar serves interactive docs from it).
//!
//! ## API path versioning (TASKLIST U1, policy §2/§3)
//!
//! Every `/api` handler surface is mounted TWICE by [`build_router`]:
//!
//! - `/api/v1/...` — the CANONICAL, documented contract. The OpenAPI
//!   document (and therefore `@meridian/sdk`) describes v1 paths only.
//! - `/api/...` — the unversioned LEGACY ALIAS from before versioning was
//!   introduced. It hits the very same handlers through the same middleware
//!   sequence with identical payloads, status codes, auth behavior and error
//!   shapes; nothing extra is added on top of it (deprecation/sunset warning
//!   headers are U4 territory, deliberately not here).
//!
//! Compatibility stance (docs/upgrade-compatibility-policy.md §2/§3): the
//! alias lives for at least one generation — removal happens NO EARLIER
//! than the next major version after its deprecation is announced in the
//! release notes' "Breaking & deprecated" section. Do not add new routes to
//! only one mount, and never change one mount's behavior without the other.
//! When new endpoints land on later lanes they follow the same two-mount
//! rule automatically, because registration goes through [`api_surface`].
//!
//! Non-purpose: no per-generation behavior forking, no response-shaping
//! differences, and no runtime deprecation headers here (that is the
//! deprecation-metadata work item, TASKLIST U4).
//!
//! Implementation note: both mounts are produced by running ONE builder
//! ([`api_surface`]) twice over an [`ApiGeneration`] tag rather than via
//! `Router::nest`. Nesting would strip full route templates from telemetry
//! (`MatchedPath` would see only the inner segment), give unmatched nested
//! requests a different fallback shape than today's top-level 404, and
//! cannot re-prefix an already-pathed router anyway. Building twice means
//! middleware ordering is identical across mounts *by construction* (the
//! exact same statement sequence runs per generation) and legacy byte
//! parity holds because the legacy generation reconstructs the historical
//! literal paths.

pub mod alerts;
pub mod api_keys;
pub mod attachments;
pub mod audit;
pub mod availability;
pub mod billing;
pub mod board_access;
pub mod boards;
pub mod calendar;
pub mod checklists;
pub mod clients;
pub mod custom_fields;
pub mod dispatch;
pub mod documents;
pub mod email_admin;
pub mod export;
pub mod imports;
pub mod internal;
pub mod invoices;
pub mod kb;
pub mod manifest;
pub mod notifications;
pub mod oidc;
pub mod openapi;
pub mod portal;
pub mod portal_admin;
pub mod projects;
pub mod report_defs;
pub mod reports;
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

/// Which URL base a router build targets.
///
/// Mounts built for [`ApiGeneration::LegacyAlias`] and
/// [`ApiGeneration::Canonical`] run the same handlers through the same
/// layer sequence; they differ ONLY in their URL base.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ApiGeneration {
    /// `/api/v1` — canonical documented contract (OpenAPI/SDK source).
    Canonical,
    /// `/api` — unversioned compatibility alias; removed no earlier than
    /// the next major per upgrade-compatibility-policy.md §2/§3. Not part
    /// of the documented contract: the spec never lists these paths.
    LegacyAlias,
}

impl ApiGeneration {
    /// The URL base this generation mounts under.
    pub const fn base(self) -> &'static str {
        match self {
            Self::LegacyAlias => "/api",
            Self::Canonical => "/api/v1",
        }
    }
}

/// Joins a generation's base onto a handler-local tail. Tails start at the
/// namespace segment (`"/health"`, `"/tickets/{id}"`) — historically that
/// was embedded after a bare `"/api"` string inside each `.route()` call.
///
/// Plain concatenation instead of `format!` so wildcard braces like `{id}`
/// stay inert literals rather than format escapes.
fn route_path(gen: ApiGeneration, tail: &str) -> String {
    let mut out = String::with_capacity(gen.base().len() + tail.len());
    out.push_str(gen.base());
    out.push_str(tail);
    out
}

pub fn build_router(state: AppState) -> Router {
    // Canonical + alias share one builder; prefixes are disjoint so merge
    // order cannot shadow either mount.
    let api = Router::new()
        .merge(api_surface(ApiGeneration::Canonical, state.clone()))
        .merge(api_surface(ApiGeneration::LegacyAlias, state.clone()));

    // Machine-to-machine effect surface (§4.1): token-guarded, NOT behind
    // the JWT middleware, and deliberately absent from OpenAPI/the SDK.
    // It sits outside the /api namespace entirely, so it stays UNVERSIONED:
    // worker↔core wiring is an internal seam, not a public boundary (policy
    // §2 lists exactly four stable boundaries; this is none of them).
    let internal = Router::new()
        .route("/internal/search/upsert", post(internal::upsert))
        .route("/internal/search/delete", post(internal::delete));

    let docs = openapi::scalar_router();

    // Request telemetry (TASKLIST 2.18): outermost so the span covers the
    // entire pipeline; the route TEMPLATE + status + duration become span
    // attrs and `meridian.http.*` metrics. Unmatched routes collapse to
    // the constant "unmatched" — never raw paths (redact module docs).
    Router::new()
        .merge(api)
        .merge(internal)
        .merge(docs)
        .layer(middleware::from_fn(crate::otel_http::track))
        .with_state(state)
}

/// Builds ALL `/api`-namespaced surfaces (public auth/machine endpoints,
/// client portal, staff protected routes) under ONE generation's base.
///
/// Called once per [`ApiGeneration`] so every mount gets the identical
/// route set and identical middleware stack ordering — keep statement order
/// below exactly as-is when touching routes: axum layers apply to routes
/// added before them on this sub-router, so reordering changes semantics
/// for BOTH generations simultaneously.
fn api_surface(gen: ApiGeneration, state: AppState) -> Router<AppState> {
    // Local shorthand keeps call sites close to their previous shape.
    let rp = |tail: &str| route_path(gen, tail);

    let public = Router::new()
        .route(&rp("/health"), get(health))
        .route(&rp("/auth/register"), post(auth_handlers::register))
        .route(&rp("/auth/login"), post(auth_handlers::login))
        .route(&rp("/auth/refresh"), post(auth_handlers::refresh))
        .route(&rp("/auth/logout"), post(auth_handlers::logout))
        // MFA completion of a password login.
        .route(&rp("/auth/mfa/verify"), post(mfa::mfa_verify))
        .route(
            &rp("/auth/mfa/webauthn/begin"),
            post(mfa::webauthn_login_begin),
        )
        .route(
            &rp("/auth/mfa/webauthn/finish"),
            post(mfa::webauthn_login_finish),
        )
        // OIDC SSO (config-driven; no providers configured ⇒ 404s).
        .route(
            &rp("/auth/oidc/providers/public"),
            get(oidc::providers_public),
        )
        .route(&rp("/auth/oidc/{name}/login"), get(oidc::login_redirect))
        .route(&rp("/auth/oidc/{name}/callback"), get(oidc::callback))
        .route(&rp("/sso/exchange"), post(oidc::sso_exchange))
        // Machine-to-machine surfaces with their own auth (HMAC webhook,
        // internal worker token) — never behind JWT middleware.
        .route(
            &rp("/webhooks/email-inbound"),
            post(webhooks::email_inbound),
        )
        .route(
            &rp("/internal/email/outbound/claim"),
            post(internal::claim_outbound),
        )
        .route(
            &rp("/internal/email/outbound/results"),
            post(internal::report_results),
        )
        // Machine surface for the worker (token-gated inside the handlers,
        // NOT behind the JWT middleware — §4.1: worker effects enter
        // through core-api, never direct DB writes).
        .route(&rp("/internal/sla/verdict"), post(internal::sla_verdict))
        .route(&rp("/internal/effects"), post(internal::effects))
        .route(
            &rp("/internal/time/gap-audit/run"),
            post(internal::run_gap_audit),
        )
        .route(
            &rp("/internal/webhook/deliveries"),
            post(internal::webhook_deliveries),
        )
        // Async import batches (TASKLIST D2 remainder): the worker replays
        // staged ObjectStore chunks through the same commit path as
        // synchronous pushes. Bearer-token gated inside the handler.
        .route(
            &rp("/internal/imports/batch/run"),
            post(internal::run_import_batch),
        );

    // Client-portal v1: public auth endpoints + a DEDICATED middleware for
    // the authenticated surface. Portal tokens (kind:"portal") never pass
    // require_auth, staff tokens never pass require_portal_auth.
    let portal_protected = Router::new()
        .route(&rp("/portal/me"), get(portal::me))
        .route(&rp("/portal/tickets"), get(portal::list_tickets))
        .route(&rp("/portal/tickets/{id}"), get(portal::get_ticket))
        .route(&rp("/portal/tickets/{id}/reply"), post(portal::reply))
        // Portal-visible attachments (TASKLIST 2.14): metadata + streamed
        // download only; hidden/internal files are indistinguishable 404s.
        .route(
            &rp("/portal/tickets/{id}/attachments"),
            get(portal::list_attachments),
        )
        .route(
            &rp("/portal/tickets/{id}/attachments/{attachment_id}/download"),
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
        .route(&rp("/portal/login"), post(portal::login))
        .route(&rp("/portal/refresh"), post(portal::refresh))
        .merge(portal_protected)
        // Org-data export (TASKLIST D1): the download endpoint authenticates
        // purely by its short-lived HMAC signature — no JWT middleware — so
        // DR scripts can pull bundles without an interactive session.
        .route(
            &rp("/org/exports/{org_id}/{export_id}/download"),
            get(export::download),
        )
        // Alert ingestor (Wave 9): HMAC-signed machine surface; the source
        // ref resolves org + secret inside the handler (no JWT middleware).
        .merge(alerts::public_routes(gen));

    let protected = Router::new()
        .route(&rp("/me"), get(auth_handlers::me))
        // session management (security settings)
        .route(&rp("/auth/sessions"), get(auth_handlers::list_sessions))
        .route(
            &rp("/auth/sessions/{id}/revoke"),
            post(auth_handlers::revoke_session),
        )
        // TOTP enrollment + recovery codes
        .route(
            &rp("/auth/mfa/totp/enroll-begin"),
            post(mfa::totp_enroll_begin),
        )
        .route(
            &rp("/auth/mfa/totp/enroll-verify"),
            post(mfa::totp_enroll_verify),
        )
        .route(&rp("/auth/mfa/totp/disable"), post(mfa::totp_disable))
        .route(
            &rp("/auth/mfa/recovery-codes/regenerate"),
            post(mfa::regenerate_recovery_codes),
        )
        // WebAuthn credential management
        .route(
            &rp("/auth/mfa/webauthn/register/begin"),
            post(mfa::webauthn_register_begin),
        )
        .route(
            &rp("/auth/mfa/webauthn/register/finish"),
            post(mfa::webauthn_register_finish),
        )
        .route(
            &rp("/auth/mfa/webauthn/credentials"),
            get(mfa::webauthn_credentials_list),
        )
        .route(
            &rp("/auth/mfa/webauthn/credentials/{id}/remove"),
            post(mfa::webauthn_credential_remove),
        )
        // OIDC provider admin + explicit linking
        .route(&rp("/auth/oidc/providers"), get(oidc::providers_list))
        .route(
            &rp("/auth/oidc/providers/{name}"),
            put(oidc::providers_upsert).delete(oidc::providers_delete),
        )
        .route(&rp("/auth/oidc/{name}/link"), post(oidc::link_begin))
        // clients
        .route(&rp("/clients"), get(clients::list).post(clients::create))
        .route(&rp("/clients/{id}"), get(clients::get_one))
        .route(
            &rp("/clients/{id}/sites"),
            get(clients::list_sites).post(clients::create_site),
        )
        .route(
            &rp("/clients/{id}/contacts"),
            get(clients::list_contacts).post(clients::create_contact),
        )
        .route(
            &rp("/clients/{id}/assets"),
            get(clients::list_assets).post(clients::create_asset),
        )
        // boards + statuses + workflow customization + organization/RBAC
        .route(&rp("/boards"), get(boards::list).post(boards::create))
        .route(&rp("/boards/nav"), get(board_access::nav))
        .route(
            &rp("/board-groups"),
            get(board_access::list_groups).post(board_access::create_group),
        )
        .route(
            &rp("/board-groups/{id}"),
            axum::routing::patch(board_access::update_group).delete(board_access::delete_group),
        )
        .route(&rp("/boards/{id}/group"), put(board_access::assign_group))
        .route(
            &rp("/boards/{id}/members"),
            get(board_access::list_members).put(board_access::upsert_member),
        )
        .route(
            &rp("/boards/{id}/members/{user_id}"),
            axum::routing::delete(board_access::remove_member),
        )
        .route(
            &rp("/boards/{id}/statuses"),
            get(boards::list_statuses).post(boards::add_status),
        )
        .route(
            &rp("/boards/{id}/statuses/{status_id}"),
            axum::routing::patch(boards::update_status).delete(boards::delete_status),
        )
        .route(
            &rp("/boards/{id}/transitions"),
            get(boards::list_transitions).put(boards::set_transitions),
        )
        // Favorite toggle: the handler is BODY-driven (board_id rides in
        // FavoriteRequest), so the documented v1 contract is
        // /boards/favorite. The {id} shape below is a compat shim from
        // v0.1.x which accepted an ignored path id; it stays mounted (both
        // generations) for this alias lifetime and is removed no earlier
        // than the next major — ONE handler, ONE documented path, never
        // list the shim in the spec.
        .route(&rp("/boards/favorite"), post(boards::toggle_favorite))
        .route(&rp("/boards/{id}/favorite"), post(boards::toggle_favorite))
        .route(
            &rp("/boards/{id}/checklist-template"),
            get(checklists::get_template)
                .put(checklists::put_template)
                .delete(checklists::delete_template),
        )
        // custom fields / dynamic forms (TASKLIST 2.17)
        .route(
            &rp("/boards/{id}/fields"),
            get(custom_fields::list_defs).post(custom_fields::create_def),
        )
        .route(
            &rp("/boards/{id}/fields/{field_id}"),
            axum::routing::patch(custom_fields::update_def).delete(custom_fields::delete_def),
        )
        .route(
            &rp("/tickets/{id}/fields/{field_id}"),
            put(custom_fields::set_ticket_value).delete(custom_fields::clear_ticket_value),
        )
        // tickets
        .route(&rp("/tickets"), get(tickets::list).post(tickets::create))
        .route(&rp("/tickets/my-work"), get(tickets::my_work))
        .route(
            &rp("/tickets/{id}"),
            get(tickets::get_one).patch(tickets::patch),
        )
        .route(&rp("/tickets/{id}/status"), post(tickets::change_status))
        .route(&rp("/tickets/{id}/sla"), get(tickets::sla_status))
        // sla configuration (admin)
        .route(
            &rp("/sla/calendars"),
            get(sla_admin::list_calendars).post(sla_admin::create_calendar),
        )
        .route(
            &rp("/sla/calendars/{id}"),
            axum::routing::patch(sla_admin::update_calendar).delete(sla_admin::delete_calendar),
        )
        .route(&rp("/sla/policies"), get(sla_admin::list_policies))
        .route(
            &rp("/sla/policies/{id}/ladder"),
            get(sla_admin::get_ladder).put(sla_admin::replace_ladder),
        )
        // automation rules (admin)
        .route(
            &rp("/automation-rules"),
            get(sla_admin::list_rules).post(sla_admin::create_rule),
        )
        .route(
            &rp("/automation-rules/{id}"),
            axum::routing::patch(sla_admin::update_rule).delete(sla_admin::delete_rule),
        )
        .route(
            &rp("/tickets/{id}/comments"),
            get(tickets::list_comments).post(tickets::add_comment),
        )
        // attachments (TASKLIST 2.14): raw-body upload needs a per-route
        // limit override because axum's default caps bodies at 2 MB.
        .route(
            &rp("/tickets/{id}/attachments"),
            get(attachments::list_for_ticket)
                .post(attachments::upload)
                .layer(axum::extract::DefaultBodyLimit::max(
                    state.config.max_upload_bytes.max(1024 * 1024) as usize,
                )),
        )
        .route(
            &rp("/tickets/{id}/comments/{comment_id}/attachments"),
            get(attachments::list_for_comment),
        )
        .route(
            &rp("/attachments/{id}/download"),
            get(attachments::download),
        )
        .route(
            &rp("/attachments/{id}"),
            axum::routing::delete(attachments::remove),
        )
        .route(
            &rp("/tickets/{id}/checklist"),
            get(tickets::get_checklist).post(tickets::add_checklist_item),
        )
        .route(
            &rp("/tickets/{id}/checklist/{item_id}"),
            post(tickets::set_checklist_item),
        )
        .route(
            &rp("/tickets/{id}/time-entries"),
            get(tickets::ticket_time_entries),
        )
        // timers (single-active-timer workflow)
        .route(&rp("/timers/active"), get(timers::active))
        .route(&rp("/timers/working-set"), get(timers::working_set))
        .route(&rp("/timers/start"), post(timers::start))
        .route(&rp("/timers/pause"), post(timers::pause))
        .route(&rp("/timers/resume"), post(timers::resume))
        .route(&rp("/timers/stop"), post(timers::stop))
        // time entries
        .route(
            &rp("/time-entries"),
            get(timers::list_entries).post(timers::create_entry),
        )
        // time discipline (settings + missing-time gap reports)
        .route(
            &rp("/time-settings"),
            get(time::get_settings).put(time::put_settings),
        )
        .route(&rp("/time-gaps/me"), get(time::my_gaps))
        .route(&rp("/time-gaps"), get(time::user_gaps))
        // notifications
        .route(&rp("/notifications"), get(notifications::list))
        .route(
            &rp("/notifications/unread-count"),
            get(notifications::unread_count),
        )
        .route(
            &rp("/notifications/mark-read"),
            post(notifications::mark_read_bulk),
        )
        .route(
            &rp("/notifications/read-all"),
            post(notifications::mark_all_read),
        )
        .route(
            &rp("/notifications/{id}/read"),
            post(notifications::mark_read),
        )
        .route(
            &rp("/notification-preferences"),
            get(notifications::prefs_get).put(notifications::prefs_update),
        )
        // unified search (TASKLIST 2.16)
        .route(&rp("/search"), get(search::search))
        // knowledge base (TASKLIST Phase 3 native KB v1): categories +
        // articles with client-facing flags; mutations instant-first.
        .route(
            &rp("/kb/categories"),
            get(kb::list_categories).post(kb::create_category),
        )
        .route(
            &rp("/kb/categories/{id}"),
            axum::routing::patch(kb::update_category).delete(kb::delete_category),
        )
        .route(
            &rp("/kb/articles"),
            get(kb::list_articles).post(kb::create_article),
        )
        .route(
            &rp("/kb/articles/{id}"),
            get(kb::get_article)
                .patch(kb::update_article)
                .delete(kb::delete_article),
        )
        // documents with immutable versions (TASKLIST D4): edits and
        // restores APPEND versions; instant-first mutations.
        .route(
            &rp("/documents"),
            get(documents::list).post(documents::create),
        )
        .route(
            &rp("/documents/{id}"),
            get(documents::get_one).patch(documents::update),
        )
        .route(
            &rp("/documents/{id}/versions"),
            get(documents::list_versions),
        )
        .route(
            &rp("/documents/{id}/versions/{version}"),
            get(documents::get_version),
        )
        .route(&rp("/documents/{id}/restore"), post(documents::restore))
        // contracts + recurring billing rules (Wave 8b billing lane):
        // contract CRUD plus READ-ONLY rule-evaluation previews — no
        // invoice persistence (Wave 9).
        .route(&rp("/contracts"), get(billing::list).post(billing::create))
        .route(
            &rp("/contracts/{id}"),
            get(billing::get_one)
                .patch(billing::patch)
                .delete(billing::remove),
        )
        .route(
            &rp("/contracts/{id}/billing-preview"),
            get(billing::preview_contract),
        )
        .route(&rp("/billing/preview"), get(billing::preview_org))
        // native invoicing v1 (TASKLIST Phase 3, Wave 9 invoicing lane):
        // invoice runs, invoices + lines, print documents, payments via
        // the PaymentProvider seam, and forward-only dunning states.
        // Mutations instant-first.
        .route(
            &rp("/billing/invoice-runs"),
            get(invoices::list_runs).post(invoices::create_run),
        )
        .route(&rp("/billing/invoice-runs/{id}"), get(invoices::get_run))
        .route(&rp("/invoices"), get(invoices::list_invoices))
        .route(&rp("/invoices/{id}"), get(invoices::get_invoice))
        .route(&rp("/invoices/{id}/document"), get(invoices::get_document))
        .route(&rp("/invoices/{id}/void"), post(invoices::void_invoice))
        .route(
            &rp("/invoices/{id}/payments"),
            get(invoices::list_payments).post(invoices::record_payment),
        )
        .route(
            &rp("/invoices/{id}/dunning"),
            post(invoices::transition_dunning),
        )
        // import framework (TASKLIST D2): org-admin gated CSV/JSON mapper.
        // The two payload-heavy posts raise axum's default body cap like
        // attachments do; chunk/batch ids flow through the path.
        .route(
            &rp("/imports/dry-run"),
            post(imports::dry_run).layer(axum::extract::DefaultBodyLimit::max(
                state
                    .config
                    .max_upload_bytes
                    .max(core_domain::import_core::MAX_CHUNK_BYTES as i64) as usize,
            )),
        )
        .route(
            &rp("/imports/batches"),
            get(imports::list_batches).post(imports::create_batch),
        )
        .route(&rp("/imports/batches/{batch_id}"), get(imports::get_batch))
        .route(
            &rp("/imports/batches/{batch_id}/chunks"),
            post(imports::post_chunk).layer(axum::extract::DefaultBodyLimit::max(
                state
                    .config
                    .max_upload_bytes
                    .max(core_domain::import_core::MAX_CHUNK_BYTES as i64) as usize,
            )),
        )
        .route(
            &rp("/imports/batches/{batch_id}/finish"),
            post(imports::finish_batch),
        )
        // D2 remainder: saved field-mapping templates per source system.
        .route(
            &rp("/imports/templates"),
            get(imports::list_templates).post(imports::create_template),
        )
        .route(
            &rp("/imports/templates/{template_id}"),
            axum::routing::put(imports::update_template)
                .get(imports::get_template)
                .delete(imports::delete_template),
        )
        // D2 remainder: async giant batches — stage chunks to the
        // ObjectStore, then hand the batch to the worker queue.
        .route(
            &rp("/imports/batches/{batch_id}/stage"),
            post(imports::stage_chunk).layer(axum::extract::DefaultBodyLimit::max(
                state
                    .config
                    .max_upload_bytes
                    .max(core_domain::import_core::MAX_CHUNK_BYTES as i64) as usize,
            )),
        )
        .route(
            &rp("/imports/batches/{batch_id}/run"),
            post(imports::run_batch),
        )
        // org-data export (TASKLIST D1): signed-download bundle creation.
        .route(&rp("/org/exports"), post(export::create))
        // audit (admin)
        .route(&rp("/audit-events"), get(audit::list))
        // operational reports (TASKLIST R1): read-model views only
        .route(&rp("/reports/{kind}"), get(reports::report))
        // plugin runtime v1: signed webhooks OUT + API keys (admin;
        // x-api-key machine auth is handled inside require_auth)
        .route(
            &rp("/webhooks"),
            get(webhook_admin::list).post(webhook_admin::create),
        )
        .route(
            &rp("/webhooks/{id}"),
            axum::routing::patch(webhook_admin::update).delete(webhook_admin::remove),
        )
        .route(
            &rp("/webhooks/{id}/deliveries"),
            get(webhook_admin::deliveries),
        )
        .route(&rp("/webhooks/{id}/test"), post(webhook_admin::test_fire))
        .route(&rp("/api-keys"), get(api_keys::list).post(api_keys::create))
        .route(
            &rp("/api-keys/{id}"),
            axum::routing::delete(api_keys::revoke),
        )
        // settings hub: typed registry + org/user scoped values + org profile
        .route(&rp("/settings/registry"), get(settings::get_registry))
        .route(
            &rp("/settings/org"),
            get(settings::org_values_get).put(settings::org_value_put),
        )
        .route(
            &rp("/settings/user"),
            get(settings::user_values_get).put(settings::user_value_put),
        )
        .route(
            &rp("/org"),
            get(settings::org_profile_get).patch(settings::org_profile_patch),
        )
        // customization manifest (TASKLIST U5): admin-gated canonical
        // export; owner/admin checked inside the handler (policy §5).
        .route(&rp("/org/manifest"), get(manifest::get))
        // email inbound routing settings (admin)
        .route(
            &rp("/email/inbound-settings"),
            get(email_admin::get_inbound_settings).put(email_admin::put_inbound_settings),
        )
        .route(
            &rp("/boards/{id}/inbound-address"),
            put(email_admin::put_board_inbound_address),
        )
        // portal administration (staff, PortalAdmin grant)
        .route(
            &rp("/clients/{id}/portal-users"),
            get(portal_admin::list_portal_users).post(portal_admin::create_portal_user),
        )
        .route(
            &rp("/portal-users/{id}"),
            axum::routing::patch(portal_admin::patch_portal_user)
                .delete(portal_admin::delete_portal_user),
        )
        .route(
            &rp("/clients/{id}/board-exposures"),
            get(portal_admin::list_board_exposures).put(portal_admin::put_board_exposure),
        )
        // scheduling/calendars/availability (TASKLIST 2.12)
        .route(&rp("/appointments"), post(scheduling::create_appointment))
        .route(
            &rp("/appointments/{id}"),
            get(scheduling::get_appointment).patch(scheduling::patch_appointment),
        )
        .route(
            &rp("/appointments/{id}/move"),
            post(scheduling::move_appointment),
        )
        .route(
            &rp("/appointments/{id}/cancel"),
            post(scheduling::cancel_appointment),
        )
        .route(
            &rp("/scheduling/conflict-check"),
            post(scheduling::conflict_check),
        )
        .route(&rp("/calendar/day"), get(calendar::calendar_day))
        .route(&rp("/calendar/week"), get(calendar::calendar_week))
        .route(&rp("/calendar/team"), get(calendar::calendar_team))
        .route(
            &rp("/availability"),
            get(availability::list_availability).post(availability::create_availability),
        )
        .route(
            &rp("/availability/{id}"),
            axum::routing::delete(availability::delete_availability),
        )
        .route(
            &rp("/on-call-blocks"),
            get(availability::list_on_call).post(availability::create_on_call),
        )
        .route(
            &rp("/on-call-blocks/{id}"),
            axum::routing::delete(availability::delete_on_call),
        )
        .route(
            &rp("/calendar-exclusions"),
            get(availability::list_exclusions).post(availability::create_exclusion),
        )
        .route(
            &rp("/calendar-exclusions/{id}"),
            axum::routing::delete(availability::delete_exclusion),
        )
        // projects line (TASKLIST Phase 3): projects → phases → tasks,
        // dependencies, saved views, templates, and the milestone
        // billing seam (completions). Instant-gated mutations + typed
        // errors live inside the module.
        .route(&rp("/projects"), get(projects::list).post(projects::create))
        .route(
            &rp("/projects/{id}"),
            get(projects::get_one).patch(projects::patch),
        )
        .route(
            &rp("/projects/{id}/phases"),
            get(projects::list_phases).post(projects::create_phase),
        )
        .route(
            &rp("/projects/{id}/phases/{phase_id}"),
            axum::routing::patch(projects::patch_phase),
        )
        .route(
            &rp("/projects/{id}/phases/{phase_id}/complete"),
            post(projects::complete_phase),
        )
        .route(
            &rp("/projects/{id}/tasks"),
            get(projects::list_tasks).post(projects::create_task),
        )
        .route(
            &rp("/projects/{id}/tasks/{task_id}"),
            axum::routing::patch(projects::patch_task),
        )
        .route(
            &rp("/projects/{id}/tasks/{task_id}/complete"),
            post(projects::complete_task),
        )
        .route(
            &rp("/projects/{id}/tasks/{task_id}/dependencies"),
            get(projects::get_deps).put(projects::set_deps),
        )
        .route(
            &rp("/project-views"),
            get(projects::list_views).post(projects::create_view),
        )
        .route(
            &rp("/project-views/{id}"),
            axum::routing::patch(projects::patch_view).delete(projects::delete_view),
        )
        .route(
            &rp("/project-templates"),
            get(projects::list_templates).post(projects::create_template),
        )
        .route(
            &rp("/project-templates/{id}"),
            axum::routing::patch(projects::patch_template).delete(projects::delete_template),
        )
        .route(
            &rp("/project-templates/{id}/instantiate"),
            post(projects::instantiate),
        )
        // report definitions (TASKLIST R3): stored specs as data, RBAC-
        // scoped, instant-first write path; module owns its own router.
        .merge(report_defs::routes(gen))
        // alert ingestor (TASKLIST Phase 3): source admin + alert ledger;
        // module owns its own router (ingest mounts via public_routes).
        .merge(alerts::admin_routes(gen))
        // Stamps org_id onto the current request span (telemetry) once the
        // auth middleware below has resolved it.
        .layer(middleware::from_fn(crate::otel_http::record_org))
        // dispatch board (TASKLIST 2.19)
        .route(&rp("/dispatch/day"), get(dispatch::dispatch_day))
        .route(&rp("/dispatch/bulk-move"), post(dispatch::bulk_move))
        .route(&rp("/dispatch/assign"), post(dispatch::assign))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    public.merge(protected)
}

async fn health() -> &'static str {
    "ok"
}
