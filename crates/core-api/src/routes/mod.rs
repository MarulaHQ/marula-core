//! HTTP routes: router assembly and the OpenAPI document (utoipa derives
//! the spec from the handlers; Scalar serves interactive docs from it).
//!
//! ## API path versioning (TASKLIST U1, policy §2/§3)
//!
//! Every `/api` handler surface is mounted TWICE by [`build_router`]:
//!
//! - `/api/v1/...` — the CANONICAL, documented contract. The OpenAPI
//!   document (and therefore `@marula/sdk`) describes v1 paths only.
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

pub mod agent_platform;
pub mod ai_policy;
pub mod alerts;
pub mod api_keys;
pub mod appointment_requests;
pub mod approvals;
pub mod assets;
pub mod attachments;
pub mod audit;
pub mod availability;
pub mod billing;
pub mod billing_provider;
pub mod board_access;
pub mod boards;
pub mod byok;
pub mod calendar;
pub mod calendar_sync;
pub mod catalog;
pub mod chat_runner;
pub mod checklists;
pub mod clients;
pub mod config_items;
pub mod connector;
pub mod costs;
pub mod credit_notes;
pub mod custom_fields;
pub mod day_pack;
pub mod decision_config;
pub mod decision_edge;
pub mod decision_suggestions;
pub mod dedupe;
pub mod dev;
pub mod dispatch;
pub mod dispatch_connector;
pub mod document_attachments;
pub mod documents;
pub mod edge;
pub mod email_admin;
pub mod entity_dedupe;
pub mod events;
pub mod export;
pub mod fx;
pub mod imports;
pub mod integrations;
pub mod internal;
pub mod invoices;
pub mod kb;
pub mod kb_rich;
pub mod labels;
pub mod loans;
pub mod loop_termination;
pub mod manifest;
pub mod notifications;
pub mod oidc;
pub mod onboarding;
pub mod openapi;
pub mod org_config;
pub mod org_users;
pub mod passwords;
pub mod pipeline;
pub mod plugin_distribution;
pub mod plugin_surface;
pub mod plugins;
pub mod portal;
pub mod portal_admin;
pub mod portal_v2;
pub mod presence;
pub mod projects;
pub mod provenance;
pub mod purchasing;
pub mod quotes;
pub mod recurring;
pub mod reply_snippets;
pub mod report_defs;
pub mod report_schedules;
pub mod reports;
pub mod roles;
pub mod routing;
pub mod scan;
pub mod scheduling;
pub mod scheduling_links;
pub mod scorecards;
pub mod search;
pub mod secret_share;
pub mod secrets;
pub mod self_host;
pub mod settings;
pub mod sign_offs;
pub mod similar_tickets;
pub mod sla_admin;
pub mod stock;
pub mod stripe_webhook;
pub mod tags;
pub mod taxonomy;
pub mod templates;
pub mod ticket_emails;
pub mod ticket_links;
pub mod tickets;
pub mod time;
pub mod time_categories;
pub mod timers;
pub mod tools;
pub mod transcription;
pub mod undo;
pub mod verification;
pub mod webhook_admin;
pub mod webhook_receiver;
pub mod webhooks;
pub mod workflow_overrides;

use crate::auth::handlers as auth_handlers;
use crate::auth::mfa_handlers as mfa;
use crate::auth::middleware::require_auth;
use crate::auth::recovery;
use crate::state::AppState;
use axum::{
    middleware,
    routing::{delete, get, post, put},
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
        .route("/internal/search/delete", post(internal::delete))
        .route("/internal/search/backfill", post(internal::search_backfill))
        .route(
            "/internal/decision/fan-out",
            post(internal::decision_fan_out),
        )
        .route(
            "/internal/alerts/decision-ack",
            post(internal::alert_decision_ack),
        )
        .route("/internal/dedupe/run", post(dedupe::internal_run));

    let docs = openapi::scalar_router();

    // Dev-tools surface (TASKLIST DX2): mounted ONLY when the process runs
    // with MARULA_DEV_TOOLS=1. Unset, the routes DO NOT EXIST — no
    // registration at all, not a 403 — and they stay outside the /api/v1
    // contract, the OpenAPI document, and the two-mount rule (a dev surface
    // is not an API generation).
    let dev = if state.config.dev_tools {
        Router::new().merge(dev::router(state.clone()))
    } else {
        Router::new()
    };

    // Request telemetry (TASKLIST 2.18): outermost so the span covers the
    // entire pipeline; the route TEMPLATE + status + duration become span
    // attrs and `marula.http.*` metrics. Unmatched routes collapse to
    // the constant "unmatched" — never raw paths (redact module docs).
    //
    // Typed rejections (Wave 13 lane 1) sit just inside telemetry: every
    // text/plain framework rejection (raw serde/query text) is rewritten
    // onto the IF3 envelope before it leaves the process.
    //
    // Correlation ids (Wave 13 lane 2) sit just inside those: every request
    // gets a `correlation_id` span field + an echoed `x-correlation-id`
    // response header, so a client-reported failure pairs with exactly one
    // grep-able server log line.
    let app = Router::new()
        .merge(api)
        .merge(internal)
        .merge(docs)
        .merge(dev)
        .layer(middleware::from_fn(crate::otel_http::correlate))
        .layer(middleware::from_fn(crate::error::typed_rejections))
        .layer(middleware::from_fn(crate::otel_http::track))
        .with_state(state);
    // ChatRunner (Wave 18 lane 2) stamps the composed router on every
    // build so its wardrobe tool self-calls ride the REAL middleware
    // stack (auth/RBAC/RLS) with the caller's own bearer.
    chat_runner::set_runner_router(app.clone());
    app
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
        // Wave 19 lane 5 recovery flows (Scope B): public, uniform-shaped
        // request answers + single-use hashed confirm tokens.
        .route(
            &rp("/auth/password-reset/request"),
            post(recovery::password_reset_request),
        )
        .route(
            &rp("/auth/password-reset/confirm"),
            post(recovery::password_reset_confirm),
        )
        .route(
            &rp("/auth/email-verification/request"),
            post(recovery::email_verification_request),
        )
        .route(
            &rp("/auth/email-verification/confirm"),
            post(recovery::email_verification_confirm),
        )
        .route(
            &rp("/auth/magic-link/request"),
            post(recovery::magic_link_request),
        )
        .route(
            &rp("/auth/magic-link/confirm"),
            post(recovery::magic_link_confirm),
        )
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
        // Edge-assertion federation (Wave 13 lane 7): unauthenticated
        // exchange of a proxy-signed JWT for tokens — deny-by-default
        // (only ENABLED sources resolve), typed 401 reasons.
        .route(&rp("/auth/edge/{name}/exchange"), post(edge::exchange))
        // Org user invite redemption (Wave 11 lane 2): public — the
        // one-time setup token is the credential; no JWT middleware.
        .route(
            &rp("/org/users/accept-invite"),
            post(org_users::accept_invite),
        )
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
        // Wave 34 lane 8: notification + digest email enqueue (the worker
        // decides WHO gets mail; core-api renders + writes the ledger).
        .route(
            &rp("/internal/email/notification"),
            post(internal::notification_email),
        )
        .route(&rp("/internal/email/digest"), post(internal::digest_email))
        .route(&rp("/internal/email/csat"), post(internal::csat_email))
        // Machine surface for the worker (token-gated inside the handlers,
        // NOT behind the JWT middleware — §4.1: worker effects enter
        // through core-api, never direct DB writes).
        .route(&rp("/internal/sla/verdict"), post(internal::sla_verdict))
        .route(&rp("/internal/pm/generate"), post(internal::pm_generate))
        .route(
            &rp("/internal/secrets/rotation"),
            post(internal::rotation_notify),
        )
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
        // CSAT surveys on resolution (Wave 34 lane 2): the in-app response
        // route rides portal auth; the emailed token is scoped to the
        // caller's client on top.
        .route(
            &rp("/portal/csat/{token}"),
            get(portal::csat_view).post(portal::csat_submit),
        )
        // Portal v2 (TASKLIST Phase 3): invoice view + pay, KB
        // self-service, appointment requests. Same middleware stack, same
        // client-scoping doctrine; mutations instant-first.
        .route(&rp("/portal/invoices"), get(portal_v2::list_invoices))
        .route(&rp("/portal/invoices/{id}"), get(portal_v2::get_invoice))
        .route(
            &rp("/portal/invoices/{id}/document"),
            get(portal_v2::invoice_document),
        )
        .route(
            &rp("/portal/invoices/{id}/pay"),
            post(portal_v2::pay_invoice),
        )
        .route(&rp("/portal/kb/categories"), get(portal_v2::kb_categories))
        .route(&rp("/portal/kb/articles"), get(portal_v2::kb_articles))
        .route(&rp("/portal/kb/articles/{id}"), get(portal_v2::kb_article))
        .route(&rp("/portal/kb/search"), get(portal_v2::kb_search))
        .route(
            &rp("/portal/schedule/slots"),
            get(portal_v2::schedule_slots),
        )
        .route(
            &rp("/portal/appointment-requests"),
            get(portal_v2::list_appointment_requests).post(portal_v2::create_appointment_request),
        )
        .route(
            &rp("/portal/appointment-requests/{id}/cancel"),
            post(portal_v2::cancel_appointment_request),
        )
        // Portal quotes (Wave 34 lane 1): scoped list/detail + the
        // client's approve/decline legs, bound to the SAME quote
        // lifecycle the staff flow runs; conversion stays staff-side.
        .route(&rp("/portal/quotes"), get(portal_v2::list_quotes))
        .route(&rp("/portal/quotes/{id}"), get(portal_v2::get_quote))
        .route(
            &rp("/portal/quotes/{id}/approve"),
            post(portal_v2::approve_quote),
        )
        .route(
            &rp("/portal/quotes/{id}/decline"),
            post(portal_v2::decline_quote),
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
        // Portal magic-link sign-in (Wave 41 lane 13): the client's own way
        // in, on its own throttle kind and its own one-use token family. Both
        // halves answer identically for every input, so neither reveals whether
        // an address is a client of this service desk.
        .route(
            &rp("/portal/magic-link/request"),
            post(portal::magic_link_request),
        )
        .route(
            &rp("/portal/magic-link/confirm"),
            post(portal::magic_link_confirm),
        )
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
        .merge(alerts::public_routes(gen))
        .merge(webhook_receiver::public_routes(gen))
        // Scheduling links (Wave 11 lane 4): unauthenticated, token-
        // scoped public booking surface; the handler resolves the org
        // scope itself (alerts pattern).
        .merge(scheduling_links::public_routes(gen))
        // Calendar sync (Wave 14 lane 2): the unauthenticated, token-
        // scoped OUTBOUND feed — the external calendar subscribes to it.
        .merge(calendar_sync::public_routes(gen))
        // Stripe webhook receiver (Wave 22): unauthenticated by design —
        // the Stripe-Signature HMAC is the auth; the handler rejects
        // typed 403 without a valid signature or configured secret.
        .route(&rp("/webhooks/stripe"), post(stripe_webhook::receive))
        // Ecosystem connectors (Wave 22): Slack/Teams/Rewst/VoIP over the
        // connector_installations consent seam; installs + tests are
        // owner/admin.
        .route(&rp("/connectors"), get(integrations::index))
        .route(
            &rp("/connectors/{kind}"),
            post(integrations::install).delete(integrations::remove),
        )
        .route(&rp("/connectors/{kind}/test"), post(integrations::test))
        .route(
            &rp("/connectors/voip/call-logs"),
            post(integrations::voip_ingest),
        )
        // Distributor pricing feeds (Wave 22): immutable batches, typed
        // overlay publisher.
        .route(
            &rp("/catalog/pricing-feeds"),
            post(integrations::create_feed),
        )
        .route(
            &rp("/catalog/pricing-feeds/{batch_id}/apply"),
            post(integrations::apply_feed),
        );

    let protected = Router::new()
        .route(&rp("/me"), get(auth_handlers::me))
        .route(
            &rp("/users/me"),
            axum::routing::patch(auth_handlers::patch_me),
        )
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
        .route(&rp("/auth/oidc/presets"), get(oidc::presets_list))
        .route(&rp("/auth/oidc/{name}/link"), post(oidc::link_begin))
        // Edge-assertion source admin + explicit links (Wave 13 lane 7)
        .route(&rp("/auth/edge/sources"), get(edge::sources_list))
        .route(
            &rp("/auth/edge/sources/{name}"),
            put(edge::sources_upsert).delete(edge::sources_delete),
        )
        .route(
            &rp("/auth/edge/sources/{name}/links"),
            get(edge::links_list).post(edge::links_create),
        )
        .route(
            &rp("/auth/edge/sources/{name}/links/{link_id}"),
            axum::routing::delete(edge::links_delete),
        )
        // org user management (Wave 11 lane 2): directory is manager+,
        // mutations owner/admin, instant-first envelope, audited.
        .route(
            &rp("/org/users"),
            get(org_users::list).post(org_users::invite),
        )
        .route(
            &rp("/org/users/{id}/status"),
            axum::routing::patch(org_users::set_status),
        )
        .route(
            &rp("/org/users/{id}/role"),
            axum::routing::patch(org_users::set_role),
        )
        // Admin email-verification override (Wave 19 lane 5, 0088).
        .route(
            &rp("/org/users/{id}/email-verified"),
            post(org_users::set_email_verified),
        )
        // role directory + preset lifecycle (Wave 13 lane 6, RB1): the
        // shipped presets are data rows; fork/restore ride the envelope.
        .route(&rp("/roles"), get(roles::list))
        .route(&rp("/roles/{id}"), put(roles::update))
        .route(&rp("/roles/{id}/fork"), post(roles::fork))
        .route(&rp("/roles/{id}/restore"), post(roles::restore))
        .route(
            &rp("/templates"),
            get(templates::list).post(templates::create),
        )
        .route(
            &rp("/templates/{id}"),
            get(templates::get)
                .patch(templates::update)
                .delete(templates::delete),
        )
        .route(&rp("/templates/{id}/fork"), post(templates::fork))
        .route(&rp("/templates/{id}/restore"), post(templates::restore))
        .route(&rp("/templates/{id}/render"), post(templates::render))
        // canned responses/snippets close-out (Wave 34 lane 3): typed
        // library in the settings domain, staff-composed, fork-on-edit.
        .route(
            &rp("/reply-snippets"),
            get(reply_snippets::list).post(reply_snippets::create),
        )
        .route(
            &rp("/reply-snippets/{id}"),
            get(reply_snippets::get_by_id)
                .patch(reply_snippets::update)
                .delete(reply_snippets::delete),
        )
        .route(&rp("/reply-snippets/{id}/fork"), post(reply_snippets::fork))
        .route(
            &rp("/reply-snippets/{id}/render"),
            post(reply_snippets::render),
        )
        // ticket taxonomy (Wave 11 lane 3): classification-only trees,
        // full org CRUD (soft retire), instant-first envelope, audited.
        .route(
            &rp("/taxonomy/types"),
            get(taxonomy::list_types).post(taxonomy::create_type),
        )
        .route(
            &rp("/taxonomy/types/{id}"),
            axum::routing::patch(taxonomy::update_type),
        )
        .route(
            &rp("/taxonomy/types/{id}/retire"),
            post(taxonomy::retire_type),
        )
        .route(
            &rp("/taxonomy/categories"),
            get(taxonomy::list_categories).post(taxonomy::create_category),
        )
        .route(
            &rp("/taxonomy/categories/{id}"),
            axum::routing::patch(taxonomy::update_category),
        )
        .route(
            &rp("/taxonomy/categories/{id}/reparent"),
            post(taxonomy::reparent_category),
        )
        .route(
            &rp("/taxonomy/categories/{id}/retire"),
            post(taxonomy::retire_category),
        )
        // board moves: status remapped through the semantic categories,
        // SLA preserved by default, audited, ticket.board_moved.
        .route(&rp("/tickets/{id}/move-board"), post(taxonomy::move_board))
        // clients
        .route(&rp("/clients"), get(clients::list).post(clients::create))
        .route(&rp("/clients/{id}"), get(clients::get_one))
        // backend documentation-completeness scoring (Wave 17 lane 13,
        // migration 0089): the weights live server-side; the client card
        // renders the facts.
        .route(
            &rp("/clients/{id}/completeness"),
            get(clients::completeness),
        )
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
            &rp("/boards/{id}/statuses/usage"),
            get(boards::status_usage),
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
        .route(
            &rp("/tickets/{id}/decision-suggestions"),
            get(decision_suggestions::list),
        )
        .route(
            &rp("/tickets/{id}/similar-tickets"),
            get(similar_tickets::similar_tickets),
        )
        .route(&rp("/tickets/{id}/status"), post(tickets::change_status))
        .route(&rp("/tickets/{id}/sla"), get(tickets::sla_status))
        // ephemeral presence (Wave 12 lane 12): who is viewing/editing
        // what; in-memory TTL store, transitions ride the event stream.
        .route(
            &rp("/tickets/{id}/presence"),
            get(presence::ticket_presence),
        )
        .route(&rp("/presence/heartbeat"), post(presence::heartbeat))
        // live delivery (Wave 12 lane 12): SSE push of contract events +
        // presence frames; dedicated pg LISTEN fan-out (migration 0068).
        .route(&rp("/events/stream"), get(events::stream))
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
        // rule test-run (Wave 17 lane 13): dry-run against sampled
        // tickets, proposal-shaped — the editor renders the result, never
        // a fake run.
        .route(
            &rp("/automation-rules/{id}/test-run"),
            post(sla_admin::test_run_rule),
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
        // Customer sign-offs (Wave 43 track 2 lane 12, migration 0127):
        // one JSON route carries the whole capture (base64 PNG + signer
        // name + consent version), so an offline queue replay is a single
        // op rather than bytes without their evidence row.
        .route(
            &rp("/tickets/{id}/sign-offs"),
            get(sign_offs::list).post(sign_offs::create),
        )
        .route(
            &rp("/tickets/{id}/sign-offs/{sign_off_id}/revoke"),
            post(sign_offs::revoke),
        )
        .route(
            &rp("/sign-offs/{id}/payload"),
            get(sign_offs::payload),
        )
        .route(
            &rp("/org/sign-off-consents/current"),
            get(sign_offs::current_consent),
        )
        .route(
            &rp("/org/sign-off-consents"),
            post(sign_offs::create_consent),
        )
        // Transcription edge (Wave 22): the ADR 0008 registered
        // `transcription` plugin surface — audio attachments only, the
        // shared AI admission pipeline gates every call.
        .route(
            &rp("/attachments/{id}/transcribe"),
            post(transcription::transcribe_attachment),
        )
        .route(
            &rp("/tickets/{id}/checklist"),
            get(tickets::get_checklist).post(tickets::add_checklist_item),
        )
        .route(
            &rp("/tickets/{id}/checklist/complete-all"),
            post(tickets::complete_all_checklist),
        )
        .route(
            &rp("/tickets/{id}/checklist/{item_id}"),
            post(tickets::set_checklist_item),
        )
        // Workbar notes/tasks lane (Wave 17 lane 11): IF2 draft autosave +
        // Tier-2 refine proposal seam (docs/notes-tasks-workbar.md).
        .route(
            &rp("/tickets/{id}/note-draft"),
            get(tickets::get_note_draft)
                .put(tickets::put_note_draft)
                .delete(tickets::delete_note_draft),
        )
        .route(
            &rp("/tickets/{id}/notes/refine"),
            post(tickets::refine_note),
        )
        // per-ticket email read (Wave 12 lane 6): the timeline's real
        // in/out rows via the 2.4 email path.
        .route(
            &rp("/tickets/{id}/emails"),
            get(ticket_emails::list_for_ticket),
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
        // time-entry approval workflows (Wave 14 lane 1)
        .route(&rp("/time-entries/{id}/submit"), post(approvals::submit))
        .route(&rp("/time-entries/{id}/approvals"), get(approvals::history))
        // Wave 41 lane 8: the one-click verdict fix, on the same permission
        // as the decision it sits beside.
        .route(&rp("/time-entries/{id}/fix"), post(approvals::fix_entry))
        .route(
            &rp("/time-entries/{id}/reopen"),
            post(approvals::reopen_entry),
        )
        .route(
            &rp("/time-entries/approvals/decide"),
            post(approvals::decide),
        )
        .route(&rp("/time-entries/approvals/queue"), get(approvals::queue))
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
        // Wave 26 onboarding flows (org wizard + per-user first-run)
        .route(&rp("/onboarding"), get(onboarding::get_summary))
        .route(
            &rp("/onboarding/org/steps/{step}/complete"),
            post(onboarding::org_step_complete),
        )
        .route(
            &rp("/onboarding/org/steps/{step}/skip"),
            post(onboarding::org_step_skip),
        )
        .route(
            &rp("/onboarding/user/steps/{step}/complete"),
            post(onboarding::user_step_complete),
        )
        .route(
            &rp("/onboarding/user/steps/{step}/skip"),
            post(onboarding::user_step_skip),
        )
        .route(&rp("/onboarding/reset/org"), post(onboarding::org_reset))
        .route(&rp("/onboarding/reset/user"), post(onboarding::user_reset))
        .route(&rp("/onboarding/disable"), post(onboarding::disable))
        .route(
            &rp("/notification-preferences"),
            get(notifications::prefs_get).put(notifications::prefs_update),
        )
        // Wave 34 lane 8: org-level notification defaults (admin policy over
        // the per-user preference chain).
        .route(
            &rp("/notification-org-defaults"),
            get(notifications::org_defaults_get).put(notifications::org_defaults_put),
        )
        // unified search (TASKLIST 2.16)
        .route(&rp("/search"), get(search::search))
        // search pins (Wave 17 lane 13, migration 0089): operator-curated
        // persistent results; the local recent chips are untouched.
        .route(
            &rp("/search/pins"),
            get(search::pins_list).post(search::pins_create),
        )
        .route(
            &rp("/search/pins/{id}"),
            axum::routing::delete(search::pins_delete),
        )
        // org-wide ticket-deadline read model for /calendar team mode
        // (Wave 17 lane 13, migration 0089).
        .route(&rp("/calendar/deadlines"), get(calendar::deadlines))
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
        // KB rich content (Wave 13 lane 4, KB1/KB2): oEmbed allowlist
        // registry + per-article embeds + linked external documents.
        // Mutations instant-first; resolution goes through the registry
        // only — raw arbitrary iframes are never allowed (ADR 0004).
        .route(
            &rp("/kb/embed-providers"),
            get(kb_rich::list_providers).post(kb_rich::create_provider),
        )
        .route(
            &rp("/kb/embed-providers/{id}"),
            axum::routing::patch(kb_rich::update_provider).delete(kb_rich::delete_provider),
        )
        .route(&rp("/kb/embeds/resolve"), post(kb_rich::resolve_embed))
        .route(
            &rp("/kb/articles/{article_id}/embeds"),
            get(kb_rich::list_embeds).post(kb_rich::create_embed),
        )
        .route(
            &rp("/kb/embeds/{id}"),
            axum::routing::delete(kb_rich::delete_embed),
        )
        .route(
            &rp("/kb/external-documents"),
            get(kb_rich::list_external_documents).post(kb_rich::create_external_document),
        )
        .route(
            &rp("/kb/external-documents/{id}"),
            get(kb_rich::get_external_document)
                .patch(kb_rich::update_external_document)
                .delete(kb_rich::delete_external_document),
        )
        .route(
            &rp("/kb/external-documents/{id}/link-check"),
            post(kb_rich::link_check_external_document),
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
        // documents attachments (Wave 17 lane 13, migration 0089): the
        // documents surface over the core-store seam (2.14); content-
        // addressed dedup shared with the ticket attachments surface.
        .route(
            &rp("/documents/{id}/attachments"),
            get(document_attachments::list).post(document_attachments::upload),
        )
        .route(
            &rp("/documents/{id}/attachments/{attachment_id}"),
            axum::routing::delete(document_attachments::delete),
        )
        .route(
            &rp("/documents/{id}/attachments/{attachment_id}/download"),
            get(document_attachments::download),
        )
        // docs-platform v1 (Wave 11 lane 7, 0053): typed document
        // layouts, cross-entity links, and the password vault.
        .route(
            &rp("/documents/layouts"),
            get(documents::list_layouts).post(documents::create_layout),
        )
        .route(
            &rp("/documents/layouts/{id}"),
            axum::routing::patch(documents::update_layout).delete(documents::delete_layout),
        )
        .route(
            &rp("/documents/{id}/links"),
            get(documents::list_links).post(documents::add_link),
        )
        .route(
            &rp("/documents/links/{id}"),
            axum::routing::delete(documents::delete_link),
        )
        .route(
            &rp("/passwords"),
            get(passwords::list).post(passwords::create),
        )
        .route(
            &rp("/passwords/{id}"),
            get(passwords::get_one)
                .patch(passwords::update)
                .delete(passwords::delete),
        )
        .route(&rp("/passwords/{id}/reveal"), post(passwords::reveal))
        .route(&rp("/passwords/{id}/history"), get(passwords::history))
        .route(
            &rp("/passwords/{id}/links"),
            get(passwords::list_links).post(passwords::add_link),
        )
        // Native secrets vault (ADR 0006, Wave 16 lane 3, migration
        // 0078): secret category trees (client AND org scope,
        // fork-the-seed per ADR 0004), SecretRecord generalization
        // beside the shipped PasswordEntry surface, audited reveal,
        // record links on the RecordLink seam, and the role × folder
        // category grants (financial:read gate shape).
        .route(
            &rp("/secrets/folders"),
            get(secrets::list_folders).post(secrets::create_folder),
        )
        .route(
            &rp("/secrets/folders/{id}"),
            axum::routing::patch(secrets::update_folder).delete(secrets::delete_folder),
        )
        .route(
            &rp("/secrets/folders/{id}/fork"),
            post(secrets::fork_folder),
        )
        .route(
            &rp("/secrets/records"),
            get(secrets::list_records).post(secrets::create_record),
        )
        .route(
            &rp("/secrets/records/{id}"),
            get(secrets::get_record)
                .patch(secrets::update_record)
                .delete(secrets::delete_record),
        )
        .route(
            &rp("/secrets/records/{id}/reveal"),
            post(secrets::reveal_record),
        )
        .route(
            &rp("/secrets/records/{id}/file"),
            axum::routing::put(secrets::put_record_file).get(secrets::get_record_file),
        )
        .route(&rp("/secrets/records/bulk"), post(secrets::bulk_ingest))
        .route(
            &rp("/secrets/records/{id}/links"),
            get(secrets::list_record_links).post(secrets::add_record_link),
        )
        // one-time share links (Wave 19 lane 3)
        .route(
            &rp("/secrets/records/{id}/share-links"),
            get(secret_share::list_record_share_links).post(secret_share::create_record_share_link),
        )
        .route(
            &rp("/secrets/records/{id}/share-links/{share_id}"),
            axum::routing::delete(secret_share::delete_record_share_link),
        )
        .route(
            &rp("/passwords/{id}/share-links"),
            get(secret_share::list_password_share_links)
                .post(secret_share::create_password_share_link),
        )
        .route(
            &rp("/passwords/{id}/share-links/{share_id}"),
            axum::routing::delete(secret_share::delete_password_share_link),
        )
        .route(
            &rp("/secrets/share-links/claim"),
            post(secret_share::claim_share_link),
        )
        .route(
            &rp("/secrets/grants"),
            get(secrets::list_grants)
                .put(secrets::set_grant)
                .delete(secrets::delete_grant),
        )
        // asset taxonomy on the docs-platform (Wave 11 lane 5, 0054):
        // typed asset instances (device_assets + document_layouts),
        // resource REST for external-DMS parity, instant-first
        // mutations, `asset.*` contract events.
        .route(&rp("/assets"), get(assets::list).post(assets::create))
        .route(
            &rp("/assets/integration-links"),
            get(assets::integration_links),
        )
        .route(
            &rp("/assets/{id}"),
            get(assets::get_one)
                .patch(assets::update)
                .delete(assets::remove),
        )
        .route(
            &rp("/assets/{id}/links"),
            get(assets::list_links).post(assets::add_link),
        )
        .route(&rp("/assets/{id}/vault"), get(assets::list_vault))
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
        // Wave 39 lane 3 contract breadth: billable amendment lifecycle
        // + SLA service credits (applied as negative lines at run time).
        .route(
            &rp("/contracts/{id}/change-orders"),
            post(billing::create_change_order),
        )
        .route(
            &rp("/contracts/{id}/change-orders/{order_id}/transition"),
            post(billing::transition_change_order),
        )
        .route(
            &rp("/contracts/{id}/sla-credits"),
            post(billing::create_sla_credit),
        )
        .route(
            &rp("/contracts/{id}/sla-credits/{credit_id}/cancel"),
            post(billing::cancel_sla_credit),
        )
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
        .route(
            &rp("/invoices/{id}/dunning/statement"),
            post(invoices::send_statement),
        )
        // credit notes (TASKLIST Money+docs backend gap): issue, apply
        // against invoices, refund, void. Mutations instant-first.
        .route(
            &rp("/credit-notes"),
            get(credit_notes::list).post(credit_notes::issue),
        )
        .route(&rp("/credit-notes/{id}"), get(credit_notes::get_one))
        .route(&rp("/credit-notes/{id}/apply"), post(credit_notes::apply))
        .route(&rp("/credit-notes/{id}/refund"), post(credit_notes::refund))
        .route(&rp("/credit-notes/{id}/void"), post(credit_notes::void))
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
        // History ledger (Wave 17 lane 13, migration 0089): list/archive
        // read model + one fresh signed link per row.
        .route(&rp("/org/exports"), get(export::list).post(export::create))
        .route(
            &rp("/org/exports/{export_id}"),
            axum::routing::patch(export::archive),
        )
        .route(
            &rp("/org/exports/{export_id}/download-link"),
            post(export::download_link),
        )
        // audit (admin)
        .route(&rp("/audit-events"), get(audit::list))
        // provenance read model (TASKLIST IF9): the 2.3 audit query API
        // extended with per-field write-trail provenance (doctrine #9);
        // RBAC tiers resolve inside the handler (manager+ org-wide,
        // others own-actions only)
        .route(&rp("/provenance"), get(provenance::list))
        // operational reports (TASKLIST R1): read-model views only
        .route(&rp("/reports/{kind}"), get(reports::report))
        // coaching scorecards (Wave 14 lane 3, R5): aggregation over the
        // read-model views; private-by-default (self only unless owner/admin)
        .route(&rp("/scorecards"), get(scorecards::team))
        .route(&rp("/scorecards/clients"), get(scorecards::clients))
        .route(&rp("/scorecards/me"), get(scorecards::me))
        .route(
            &rp("/scorecards/me/settings"),
            get(scorecards::get_settings).patch(scorecards::update_settings),
        )
        .route(&rp("/scorecards/{user_id}"), get(scorecards::get_user))
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
        // self-hoster visibility (Wave 36 lane 3): one owner/admin read of
        // the ops stack — health sweep, ClamAV detections, external links.
        .route(&rp("/settings/self-host"), get(self_host::get))
        .route(
            &rp("/settings/org"),
            get(settings::org_values_get).put(settings::org_value_put),
        )
        .route(
            &rp("/settings/user"),
            get(settings::user_values_get).put(settings::user_value_put),
        )
        // Wave 42: the caller's resolved greeting/signature envelope — the
        // live preview and the composer prefill, through the same chain the
        // send path uses.
        .route(
            &rp("/settings/user/email-envelope"),
            get(settings::user_email_envelope_get),
        )
        .route(
            &rp("/org"),
            get(settings::org_profile_get).patch(settings::org_profile_patch),
        )
        // org-scoped pod-visibility matrix (Wave 12 lane 6): the admin
        // per-RBAC config moved off device-local panel state.
        .route(
            &rp("/org/pod-visibility"),
            get(org_config::pod_visibility_get).put(org_config::pod_visibility_put),
        )
        // AI/BYOK settings (TASKLIST Phase 3: CompletionProvider seam)
        .route(&rp("/settings/ai/status"), get(byok::get_ai_status))
        .route(&rp("/settings/ai/byok"), put(byok::put_byok_secret))
        .route(
            &rp("/settings/ai/byok/{provider}"),
            axum::routing::delete(byok::delete_byok_secret),
        )
        .route(&rp("/settings/ai/test"), post(byok::test_ai_connection))
        // AI availability matrix + usage ledger (ADR 0008, Wave 18 lane 1)
        .route(&rp("/settings/ai/matrix"), get(ai_policy::get_matrix))
        .route(&rp("/settings/ai/catalog"), put(ai_policy::put_catalog))
        .route(
            &rp("/settings/ai/catalog/{entry_id}"),
            axum::routing::delete(ai_policy::delete_catalog),
        )
        .route(
            &rp("/settings/ai/credentials"),
            put(ai_policy::put_org_credential),
        )
        .route(
            &rp("/settings/ai/credentials"),
            get(ai_policy::get_org_credentials),
        )
        .route(
            &rp("/settings/ai/credentials/{credential_id}"),
            axum::routing::delete(ai_policy::delete_org_credential),
        )
        .route(&rp("/settings/ai/grants"), put(ai_policy::put_grant))
        .route(
            &rp("/settings/ai/grants/{grant_id}"),
            axum::routing::delete(ai_policy::delete_grant),
        )
        .route(&rp("/settings/ai/caps"), put(ai_policy::put_cap))
        .route(
            &rp("/settings/ai/caps/{cap_id}"),
            axum::routing::delete(ai_policy::delete_cap),
        )
        .route(&rp("/settings/ai/usage/org"), get(ai_policy::get_org_usage))
        // Decision-harness admin config (this lane): the per-surface
        // config + master switch + shadow-preview seam the settings/ai
        // decision-harness leg rides (ADR 0010 §4 + ADR 0011 §3).
        .route(
            &rp("/settings/ai/decision"),
            get(decision_config::get_decision_overview),
        )
        .route(
            &rp("/settings/ai/decision/features"),
            put(decision_config::put_decision_features),
        )
        .route(
            &rp("/settings/ai/decision/{surface_key}/config"),
            put(decision_config::put_decision_config),
        )
        .route(
            &rp("/settings/ai/decision/{surface_key}/preview"),
            post(decision_config::post_decision_preview),
        )
        .route(
            &rp("/settings/ai/decision/{surface_key}/runs"),
            get(decision_config::get_decision_runs),
        )
        .route(&rp("/ai/me/surfaces"), get(ai_policy::get_my_surfaces))
        .route(&rp("/ai/me/authorize"), post(ai_policy::authorize))
        .route(&rp("/ai/me/credentials"), put(ai_policy::put_my_credential))
        .route(
            &rp("/ai/me/credentials"),
            get(ai_policy::get_my_credentials),
        )
        .route(
            &rp("/ai/me/credentials/{credential_id}"),
            axum::routing::delete(ai_policy::delete_my_credential),
        )
        .route(
            &rp("/ai/me/model-picks/{surface_key}"),
            put(ai_policy::put_my_pick).delete(ai_policy::delete_my_pick),
        )
        .route(
            &rp("/settings/ai/model-picks/{surface_key}"),
            put(ai_policy::put_org_pick),
        )
        .route(&rp("/ai/me/usage"), get(ai_policy::get_my_usage))
        // Plugin-surface HTTP registration (Wave 18 lane 2b): admin
        // routes over upsert_org_plugin_surface — list/create/enable/
        // disable, audited, instant-first. The grant flow renders the
        // registered plugin surfaces.
        .route(
            &rp("/settings/ai/plugin-surfaces"),
            get(ai_policy::get_plugin_surfaces).put(ai_policy::put_plugin_surface),
        )
        .route(
            &rp("/settings/ai/plugin-surfaces/{surface_id}/enable"),
            post(ai_policy::enable_plugin_surface),
        )
        .route(
            &rp("/settings/ai/plugin-surfaces/{surface_id}/disable"),
            post(ai_policy::disable_plugin_surface),
        )
        // ChatRunner (ADR 0008 §3, Wave 18 lane 2): the bounded model
        // loop over SSE. Authorization refusals ride lane 1's edge.
        .route(
            &rp("/ai/chat/sessions/{session_id}/turns"),
            post(chat_runner::post_turn),
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
        // Wave 34 lane 8: outbound identity + transport + delivery ledger.
        .route(&rp("/email/settings"), get(email_admin::get_mail_settings))
        .route(
            &rp("/email/outbound-settings"),
            put(email_admin::put_outbound_settings),
        )
        .route(
            &rp("/email/transport-credentials"),
            put(email_admin::put_transport_credentials)
                .delete(email_admin::delete_transport_credentials),
        )
        .route(
            &rp("/email/delivery-ledger"),
            get(email_admin::get_delivery_ledger),
        )
        .route(&rp("/email/test-send"), post(email_admin::test_send))
        .route(
            &rp("/boards/{id}/email-identity"),
            put(email_admin::put_board_identity),
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
        // Portal v2 staff half: the client's appointment requests are
        // decided here (approve books the calendar, decline refuses).
        .route(
            &rp("/appointment-requests"),
            get(appointment_requests::list),
        )
        .route(
            &rp("/appointment-requests/{id}/approve"),
            post(appointment_requests::approve),
        )
        .route(
            &rp("/appointment-requests/{id}/decline"),
            post(appointment_requests::decline),
        )
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
        // Two-way calendar sync (Wave 14 lane 2)
        .route(
            &rp("/calendar-sync"),
            get(calendar_sync::list_connections).post(calendar_sync::create_connection),
        )
        .route(
            &rp("/calendar-sync/{id}"),
            axum::routing::patch(calendar_sync::update_connection)
                .delete(calendar_sync::delete_connection),
        )
        .route(
            &rp("/calendar-sync/{id}/rotate-token"),
            post(calendar_sync::rotate_token),
        )
        .route(
            &rp("/calendar-sync/{id}/sync"),
            post(calendar_sync::run_sync),
        )
        .route(
            &rp("/calendar-sync/{id}/events"),
            get(calendar_sync::list_events),
        )
        // Delegated OAuth for the Google + Microsoft calendar providers
        // (Wave 34 claims close-out, claim 5).
        //
        // The callback is deliberately OUTSIDE the authenticated set: it is
        // the vendor's browser redirect and carries no bearer token. Its
        // safety comes from the server-generated `state` nonce, which is
        // matched against the connection row and consumed atomically — see
        // `oauth_callback`.
        .route(
            &rp("/calendar-sync/{connection_id}/authorize"),
            post(calendar_sync::authorize_start),
        )
        .route(
            &rp("/calendar-sync/{provider}/callback"),
            get(calendar_sync::oauth_callback),
        )
        .route(&rp("/calendar/day"), get(calendar::calendar_day))
        // Plugin runtime v2 + marketplace registry (Wave 14 lane 5):
        // catalog reads are plugins:view; every write is an explicit
        // admin action (special:manage_plugins + owner/admin).
        .route(
            &rp("/plugins/catalog"),
            get(plugins::list_catalog).post(plugins::publish_catalog_entry),
        )
        .route(
            &rp("/plugins/catalog/{slug}"),
            get(plugins::get_catalog_entry),
        )
        .route(
            &rp("/plugins/catalog/{slug}/versions"),
            post(plugins::append_catalog_version),
        )
        .route(
            &rp("/plugins/catalog/{slug}/revoke"),
            post(plugins::revoke_catalog_entry),
        )
        .route(
            &rp("/plugins/installed"),
            get(plugins::list_installs).post(plugins::create_install),
        )
        .route(
            &rp("/plugins/installed/{id}"),
            axum::routing::patch(plugins::update_install).delete(plugins::delete_install),
        )
        // §4.6 plugin surface registry (Wave 34 lane 4): reads are any
        // authed org member (declarations carry no org data; each
        // consuming surface keeps its own data gates).
        .route(
            &rp("/plugins/surface/widgets"),
            get(plugin_surface::list_widgets),
        )
        .route(
            &rp("/plugins/surface/automation-actions"),
            get(plugin_surface::list_automation_actions),
        )
        .route(
            &rp("/plugins/surface/report-renders"),
            get(plugin_surface::list_report_renders),
        )
        .route(
            &rp("/plugins/surface/field-validators"),
            get(plugin_surface::list_field_validators),
        )
        // ADR 0009 distribution lifecycle (Wave 16 lane 7): sources,
        // staged imports, update re-consent, settings-from-manifest,
        // provenance. Reads are plugins:view; lifecycle writes are
        // owner/admin (special:manage_plugins) as in the seam above.
        .route(
            &rp("/plugins/distribution/staged"),
            get(plugin_distribution::list_staged),
        )
        .route(
            &rp("/plugins/distribution/staged/registry"),
            post(plugin_distribution::stage_from_registry),
        )
        .route(
            &rp("/plugins/distribution/staged/github"),
            post(plugin_distribution::stage_from_github),
        )
        .route(
            &rp("/plugins/distribution/staged/zip"),
            post(plugin_distribution::stage_from_zip),
        )
        .route(
            &rp("/plugins/distribution/staged/skill"),
            post(plugin_distribution::stage_from_skill),
        )
        .route(
            &rp("/plugins/distribution/staged/{id}/consent"),
            get(plugin_distribution::get_staged_consent),
        )
        .route(
            &rp("/plugins/distribution/staged/{id}/approve"),
            post(plugin_distribution::approve_staged),
        )
        .route(
            &rp("/plugins/distribution/staged/{id}/refuse"),
            post(plugin_distribution::refuse_staged),
        )
        .route(
            &rp("/plugins/distribution/theme-packs"),
            get(plugin_distribution::list_theme_packs),
        )
        .route(
            &rp("/plugins/distribution/theme-packs/{slug}"),
            delete(plugin_distribution::delete_theme_pack),
        )
        .route(
            &rp("/plugins/installed/{id}/update"),
            get(plugin_distribution::get_update_proposal).post(plugin_distribution::propose_update),
        )
        .route(
            &rp("/plugins/installed/{id}/consent"),
            post(plugin_distribution::consent_update),
        )
        .route(
            &rp("/plugins/installed/{id}/discard-update"),
            post(plugin_distribution::discard_update),
        )
        .route(
            &rp("/plugins/installed/{id}/settings"),
            get(plugin_distribution::get_install_settings)
                .patch(plugin_distribution::put_install_settings),
        )
        .route(
            &rp("/plugins/installed/{id}/provenance"),
            get(plugin_distribution::list_install_provenance),
        )
        .route(
            &rp("/plugins/distribution/agent-skills"),
            get(plugin_distribution::list_org_skills),
        )
        .route(
            &rp("/plugins/distribution/agent-skills/{id}/enabled"),
            post(plugin_distribution::set_skill_enabled),
        )
        // Marketplace v2 (Wave 22): verified publishers + ratings —
        // strictly ADDITIVE over the v1 surfaces above (reads plugins:view;
        // publisher writes owner/admin; ratings any staff viewer).
        .route(
            &rp("/plugins/distribution/publishers"),
            get(plugin_distribution::list_publishers).post(plugin_distribution::verify_publisher),
        )
        .route(
            &rp("/plugins/distribution/publishers/{id}"),
            delete(plugin_distribution::revoke_publisher),
        )
        .route(
            &rp("/plugins/distribution/ratings"),
            get(plugin_distribution::list_ratings).put(plugin_distribution::put_rating),
        )
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
        // scheduling links (Wave 11 lane 4): free-form groups, meeting
        // types, and shareable booking links (public token surface above)
        .route(
            &rp("/scheduling-groups"),
            get(scheduling_links::list_groups).post(scheduling_links::create_group),
        )
        .route(
            &rp("/scheduling-groups/{id}"),
            get(scheduling_links::get_group)
                .patch(scheduling_links::update_group)
                .delete(scheduling_links::delete_group),
        )
        .route(
            &rp("/scheduling-groups/{id}/members"),
            post(scheduling_links::add_group_member),
        )
        .route(
            &rp("/scheduling-groups/{id}/members/{user_id}"),
            axum::routing::delete(scheduling_links::remove_group_member),
        )
        .route(
            &rp("/meeting-types"),
            get(scheduling_links::list_meeting_types).post(scheduling_links::create_meeting_type),
        )
        .route(
            &rp("/meeting-types/{id}"),
            axum::routing::patch(scheduling_links::update_meeting_type)
                .delete(scheduling_links::delete_meeting_type),
        )
        .route(
            &rp("/scheduling-links"),
            get(scheduling_links::list_links).post(scheduling_links::create_link),
        )
        .route(
            &rp("/scheduling-links/{id}/revoke"),
            post(scheduling_links::revoke_link),
        )
        .route(
            &rp("/scheduling-links/{id}/bookings"),
            get(scheduling_links::list_link_bookings),
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
        // The dated Gantt + phase budget/burn read model (Wave 34 claims
        // close-out, claim 6). Separate from the phase list because it
        // partitions dated from undated, and a client drawing a timeline
        // must not have to re-derive that distinction to avoid placing a
        // phase nobody scheduled.
        .route(&rp("/projects/{id}/gantt"), get(projects::gantt))
        .route(
            &rp("/projects/{id}/phases/{phase_id}"),
            axum::routing::patch(projects::patch_phase),
        )
        .route(
            &rp("/projects/{id}/phases/{phase_id}/complete"),
            post(projects::complete_phase),
        )
        .route(&rp("/projects/{id}/phase-burn"), get(projects::phase_burn))
        .route(
            &rp("/projects/{id}/billing-releases"),
            get(projects::list_billing_releases),
        )
        .route(
            &rp("/projects/{id}/billing-releases/{release_id}/cancel"),
            post(projects::cancel_billing_release),
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
        // scheduled report delivery (TASKLIST R4): admin-gated schedule
        // CRUD + cursor-paged listing with next-run preview; module owns
        // its own router.
        .merge(report_schedules::routes(gen))
        // agent platform (TASKLIST Phase 3, IF8): capability registry +
        // agent skills + proposal pipeline; module owns its own router.
        .merge(agent_platform::routes(gen))
        // undo as a core capability (TASKLIST IF7): per-field/per-record
        // applied-state history + revert-as-normal-IF1-write; module owns
        // its own router.
        .merge(undo::routes(gen))
        // migration connector (TASKLIST D3): authenticated bulk push
        // (rides the D2 engine, checkpoint cursors) + cursor-paged pull
        // exports; module owns its own router.
        .merge(connector::routes(gen))
        // workflow constructs (Wave 11 lane 9, migration 0056): tags,
        // merge/linking, ticket costs/expenses, and PM schedules — each
        // module owns its own router.
        .merge(tags::routes(gen))
        // time categories (Wave 39 lane 1, migration 0110): org time
        // taxonomy, capture defaults, billing-audit suggestion reads —
        // module-owned router; mutations ride the IF1-IF3 envelope.
        .merge(time_categories::routes(gen))
        // billing verification workspace (Wave 39 lane 2, migration
        // 0111): the pre-invoice review surface, its in-context entry
        // actions, and the close sign-off — module-owned router.
        .merge(verification::routes(gen))
        .merge(ticket_links::routes(gen))
        .merge(costs::routes(gen))
        .merge(recurring::routes(gen))
        // quote builder + product/service catalog + sales pipeline
        // (Wave 12 lane 2, migration 0060) — each module owns its own
        // router; mutations ride the IF1–IF3 envelope.
        .merge(catalog::routes(gen))
        .merge(stock::routes(gen))
        // purchasing + vendors (Wave 32 lane 2, migration 0103): vendors,
        // purchase orders with typed receipt states, receipts posting
        // into the lane-1 stock ledger, external deeplinks, and the bill
        // → project cost rollup hook.
        .merge(purchasing::routes(gen))
        // tools checkout + toolboxes + client loans (Wave 32 lane 3,
        // migration 0104) — module-owned routers; mutations ride the
        // IF1-IF3 envelope.
        .merge(tools::routes(gen))
        .merge(day_pack::routes(gen))
        .merge(loans::routes(gen))
        // label templates + label rendering + scan resolution (Wave 32
        // lane 5, migration 0105): one shared print/scan capability —
        // template CRUD, the deterministic draft preview + entity render,
        // and the typed scanned-payload resolver every scanner surface
        // (web keyboard-wedge now, mobile camera in Wave 33) consumes.
        .merge(labels::routes(gen))
        .merge(scan::routes(gen))
        .merge(quotes::routes(gen))
        .merge(pipeline::routes(gen))
        // currency & FX (ADR 0002, Wave 12 lane 10, migration 0066): the
        // append-only rate book (manual-entry built-in provider) + the
        // reconciliation read model; mutations ride the IF1–IF3 envelope.
        .merge(fx::routes(gen))
        // configuration identity (ADR 0003, Wave 12 lane 3, migration
        // 0061): CI facets of the asset surface — source links (the only
        // feed write path), interfaces, curation, and the audited/undoable
        // merge queue; module owns its own router.
        .merge(config_items::routes(gen))
        .merge(entity_dedupe::routes(gen))
        .merge(dedupe::routes(gen))
        .merge(workflow_overrides::routes(gen))
        // dispatch connector (TASKLIST Phase 3, Wave 9 lane 9): scoped
        // machine surface — schedule reads + field-service status pushes;
        // module owns its own router.
        .merge(dispatch_connector::routes(gen))
        // Stamps org_id onto the current request span (telemetry) once the
        // auth middleware below has resolved it.
        .layer(middleware::from_fn(crate::otel_http::record_org))
        // dispatch board (TASKLIST 2.19)
        .route(&rp("/dispatch/day"), get(dispatch::dispatch_day))
        .route(&rp("/dispatch/bulk-move"), post(dispatch::bulk_move))
        .route(&rp("/dispatch/assign"), post(dispatch::assign))
        // dispatch v2 (TASKLIST Phase 3, Wave 9 lane 9): resource
        // planning (capacity horizon, queue prioritization, skills
        // matching) + travel/mileage entry drafts.
        .route(&rp("/dispatch/capacity"), get(dispatch::capacity))
        // curated per-technician work-next queues (Wave 34 claims
        // close-out, claim 2). One path, three verbs: list your own (or, as
        // a dispatcher, someone's), add-or-move, and remove.
        .route(
            &rp("/dispatch/work-next"),
            get(dispatch::work_next_list)
                .put(dispatch::work_next_add)
                .delete(dispatch::work_next_remove),
        )
        .route(
            &rp("/dispatch/work-next/reorder"),
            post(dispatch::work_next_reorder),
        )
        // resource planning across project phases (claim 1): the stated
        // commitments the appointment-only capacity horizon could not see,
        // and the forecast that adds them up.
        .route(
            &rp("/dispatch/planning/allocations"),
            get(dispatch::phase_allocations_list).put(dispatch::phase_allocation_upsert),
        )
        .route(
            &rp("/dispatch/planning/allocations/{allocation_id}"),
            delete(dispatch::phase_allocation_delete),
        )
        .route(
            &rp("/dispatch/planning/forecast"),
            get(dispatch::capacity_forecast),
        )
        .route(&rp("/dispatch/queue"), get(dispatch::queue_list))
        .route(
            &rp("/dispatch/queue/{ticket_id}"),
            axum::routing::put(dispatch::queue_set).delete(dispatch::queue_clear),
        )
        .route(&rp("/dispatch/skills"), get(dispatch::skills_catalog))
        .route(
            &rp("/dispatch/users/{user_id}/skills"),
            axum::routing::put(dispatch::user_skills_replace),
        )
        .route(&rp("/dispatch/match"), get(dispatch::match_queue))
        .route(
            &rp("/dispatch/tickets/{ticket_id}/required-skills"),
            axum::routing::put(dispatch::ticket_skills_replace),
        )
        .route(
            &rp("/dispatch/travel-drafts"),
            get(dispatch::travel_draft_list).post(dispatch::travel_draft_create),
        )
        .route(
            &rp("/dispatch/travel-drafts/{id}"),
            axum::routing::patch(dispatch::travel_draft_patch)
                .delete(dispatch::travel_draft_delete),
        )
        .route(
            &rp("/dispatch/travel-drafts/{id}/submit"),
            post(dispatch::travel_draft_submit),
        )
        // dispatch v3 — routing (Wave 32 lane 7): day journey plans +
        // warnings, suggestion-grade improvements, journey start points,
        // mileage reconciliation, expense-from-draft.
        .route(&rp("/dispatch/routing"), get(routing::routing_day))
        .route(
            &rp("/dispatch/journey-start"),
            get(routing::journey_start_get).put(routing::journey_start_set),
        )
        .route(
            &rp("/dispatch/reconciliation"),
            get(routing::reconciliation),
        )
        .route(
            &rp("/dispatch/travel-drafts/{id}/expense"),
            post(routing::travel_draft_expense),
        )
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    public.merge(protected)
}

async fn health() -> &'static str {
    "ok"
}
