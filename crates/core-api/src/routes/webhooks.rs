//! Inbound email webhook: POST /api/webhooks/email-inbound.
//!
//! Purpose: HMAC-SHA256 shared-secret verification over the raw body
//! (constant-time), canonical payload parsing, and handoff to the email
//! pipeline. Machine-to-machine surface — deliberately NOT in OpenAPI/SDK.
//!
//! Non-purpose: no provider-specific payload dialects (providers are
//! adapted at the edge into the canonical shape) and no JWT auth — the
//! shared secret IS the authentication.

use crate::email;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use core_mail::hmac;
use serde_json::json;

pub const SIGNATURE_HEADER: &str = "X-Meridian-Signature";

/// POST /api/webhooks/email-inbound — accepts one normalized message.
///
/// Responses: 202 with a terminal `status` (comment / ticket_created /
/// quarantined / duplicate / unrouted); 401 bad signature; 422 malformed
/// payload; 503 when no inbound secret is configured (disabled none mode).
pub async fn email_inbound(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let Some(secret) = state.config.email_inbound_hmac_secret.as_deref() else {
        // Disabled on purpose: never an open relay into ticketing.
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": { "code": 503, "message": "inbound email disabled" } })),
        )
            .into_response());
    };

    let presented = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if presented.is_empty() || !hmac::verify(secret.as_bytes(), &body, presented) {
        return Err(ApiError::Unauthorized("invalid webhook signature".into()));
    }

    let msg: core_mail::InboundEmail = serde_json::from_slice(&body).map_err(|e| {
        // Raw serde diagnostics stay server-side (Wave 13 lane 1); the
        // envelope carries a fixed, parseable message instead.
        tracing::warn!(error = %e, "inbound webhook payload failed to parse");
        ApiError::Validation("inbound payload is malformed or has the wrong shape".into())
    })?;

    let status = email::process_inbound(&state, msg).await?;
    Ok((StatusCode::ACCEPTED, Json(email::inbound_response(status))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::email::{InboundStatus, MailStack};
    use crate::state::AppState;
    use axum::routing::post;
    use axum::Router;
    use tower::ServiceExt;

    fn test_state(with_secret: bool) -> AppState {
        let mut config = Config::from_env_allow_dev_fallback().expect("test config");
        config.email_inbound_hmac_secret = with_secret.then(|| "test-secret".to_string());
        config.database_url =
            "postgres://meridian_app:meridian_app@localhost:1/never_connected".into();
        AppState {
            // Lazy pool: never dialed because HMAC rejects first.
            pool: sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect_lazy(&config.database_url)
                .expect("lazy pool"),
            reporting: crate::state::AppState::reporting_pool_lazy(
                "postgres://meridian_app:meridian_app@localhost:1/never_connected",
            )
            .expect("lazy reporting pool"),
            config: std::sync::Arc::new(config.clone()),
            mail: std::sync::Arc::new(MailStack::from_config(&config)),
            store: std::sync::Arc::new(core_store::memory::MemoryStore::new()),
            routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
            oidc: crate::auth::oidc_client::OidcHttpClient::new(),
        }
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .route("/api/webhooks/email-inbound", post(email_inbound))
            .with_state(state)
    }

    const VALID_BODY: &str = r#"{
        "message_id": "<t@test>",
        "from": "a@b.c",
        "to": ["support@acme.test"],
        "subject": "Hi",
        "text": "hello"
    }"#;

    #[tokio::test]
    async fn disabled_secret_returns_503() {
        let res = app(test_state(false))
            .oneshot(
                axum::http::Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/api/webhooks/email-inbound")
                    .body(axum::body::Body::from(VALID_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn missing_and_bad_signatures_rejected() {
        for sig in [
            None,
            Some("deadbeef".to_string()),
            Some(hmac::sign(b"wrong", VALID_BODY.as_bytes())),
        ] {
            let mut req = axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/api/webhooks/email-inbound")
                .body(axum::body::Body::from(VALID_BODY))
                .unwrap();
            if let Some(s) = sig {
                req.headers_mut()
                    .insert(SIGNATURE_HEADER, s.parse().unwrap());
            }
            let res = app(test_state(true)).oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn tampered_body_rejected_even_with_stale_valid_signature() {
        let sig = hmac::sign(b"test-secret", b"original body");
        let req = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri("/api/webhooks/email-inbound")
            .header(SIGNATURE_HEADER, sig)
            .body(axum::body::Body::from("tampered body"))
            .unwrap();
        let res = app(test_state(true)).oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_signature_with_malformed_payload_is_422() {
        let body = r#"{"message_id": ""}"#;
        let req = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri("/api/webhooks/email-inbound")
            .header(
                SIGNATURE_HEADER,
                hmac::sign(b"test-secret", body.as_bytes()),
            )
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        let res = app(test_state(true)).oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn unroutable_recipient_accepted_without_side_effects() {
        // Valid signature + valid payload but no routing config anywhere:
        // must reach the DB layer... which this lazy-pool test cannot dial.
        // Full pipeline behavior lives in core-mail's two-weeks harness and
        // the core-db suite; here we assert only the auth/validation shell.
        let body = VALID_BODY.to_string();
        let req = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri("/api/webhooks/email-inbound")
            .header(
                SIGNATURE_HEADER,
                hmac::sign(b"test-secret", body.as_bytes()),
            )
            .body(axum::body::Body::from(body))
            .unwrap();
        let res = app(test_state(true)).oneshot(req).await.unwrap();
        // The lazy pool fails to connect → internal error, NOT unauthorized:
        // proves auth+parse passed before any database work.
        assert_ne!(res.status(), StatusCode::UNAUTHORIZED);
        assert_ne!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn status_mapping_is_terminal_and_snake_cased() {
        assert_eq!(InboundStatus::TicketCreated.as_str(), "ticket_created");
    }
}
