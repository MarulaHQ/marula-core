//! core-api/otel_http — HTTP request spans + request metrics (TASKLIST 2.18).
//!
//! Purpose: one outermost middleware producing a span per request with
//! LOW-CARDINALITY attributes only — method, route TEMPLATE
//! (`/api/tickets/{id}`, never the raw path), status code, duration — plus
//! `org_id` stamped by [`record_org`] once an auth middleware has resolved
//! it. The same middleware counts `marula.http.requests` and samples
//! `marula.http.request.duration` via [`core_otel::metrics`].
//!
//! Redaction (see core_otel::redact): no headers, no query strings (WS
//! `?token=` / OIDC `?code=` never reach telemetry), no bodies. Unmatched
//! routes collapse to the constant `"unmatched"` so 404 probing cannot
//! explode label cardinality; a debug assertion enforces template shape.
//!
//! Non-purpose: no per-handler spans (handlers already log domain events),
//! no sampling policy, no body capture — this module is deliberately the
//! only place request-level telemetry is defined.

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use std::time::Instant;
use tracing::Instrument as _;

/// The request-correlation middleware (Wave 13 lane 2): every request
/// carries a `correlation_id` — an inbound `x-correlation-id` header when
/// one is sane, else a fresh UUIDv7 — stamped onto the current request
/// span and echoed back on the response. Client-reported failures and
/// server log lines then share one id: an error envelope shown in the
/// console pairs with exactly one grep-able server log.
///
/// Redaction stance: an inbound id is only echoed when it is a short
/// ASCII alphanumeric/dash token (bounded length, no header injection);
/// anything else is replaced by a server-generated id. The span attribute
/// is the ONLY log surface — ids never touch route labels or metrics.
pub async fn correlate(request: Request, next: Next) -> Response {
    const MAX_ID_LEN: usize = 128;
    let inbound = request
        .headers()
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| {
            !v.is_empty()
                && v.len() <= MAX_ID_LEN
                && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .map(str::to_owned);
    let correlation_id = inbound.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

    let span = tracing::Span::current();
    span.record("correlation_id", correlation_id.as_str());

    let mut response = next.run(request).await;
    if let Ok(value) = axum::http::HeaderValue::from_str(&correlation_id) {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static("x-correlation-id"),
            value,
        );
    }
    response
}

/// The outermost request-telemetry middleware. Applied LAST on the final
/// router so it wraps everything (the span covers auth + handlers).
pub async fn track(request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_string();
    let matched = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str());
    // Own the route label: it must outlive `request` (moved into `next`).
    let route = core_otel::redact::route_template(matched).to_string();
    // Cheap shape guard: raw paths must never become labels/spans attrs.
    if cfg!(debug_assertions) {
        debug_assert!(
            core_otel::redact::ensure_low_cardinality(&route).is_ok(),
            "raw path leaked into route attribute: {route}"
        );
    }

    let span = tracing::info_span!(
        "http_request",
        http.method = %method,
        http.route = %route,
        // Stamped post-auth by `record_org`; empty until then.
        org_id = tracing::field::Empty,
        // Stamped by `correlate` for every request (Wave 13 lane 2).
        correlation_id = tracing::field::Empty,
    );

    let started = Instant::now();
    // The span is CURRENT for the whole inner pipeline (auth → handlers),
    // so nested spans parent onto it and record_org can stamp org_id.
    let response = next.run(request).instrument(span.clone()).await;
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;

    let status = response.status().as_u16();
    span.record("http.status_code", status);
    span.record("latency_ms", latency_ms as u64);
    core_otel::metrics::http_request(&route, &method, status, latency_ms);
    response
}

/// Post-auth middleware: stamps `org_id` onto the CURRENT request span.
///
/// Layered INSIDE `require_auth` / portal auth so the extension is present
/// when it runs. Machine-to-machine routes keep their payload-borne org in
/// handler spans instead — this middleware simply records nothing there.
pub async fn record_org(request: Request, next: Next) -> Response {
    let span = tracing::Span::current();
    if let Some(user) = request.extensions().get::<crate::auth::AuthedUser>() {
        span.record("org_id", tracing::field::display(user.user.org_id));
    } else if let Some(portal) = request.extensions().get::<crate::auth::PortalAuthed>() {
        span.record("org_id", tracing::field::display(portal.user.org_id));
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    /// The redaction contract the middleware routes every label through:
    /// templates pass, raw paths (ids/UUIDs) are rejected, unmatched
    /// collapses to a constant.
    #[test]
    fn route_label_is_always_a_template() {
        assert_eq!(
            core_otel::redact::route_template(Some("/api/tickets/{id}")),
            "/api/tickets/{id}"
        );
        assert_eq!(
            core_otel::redact::route_template(None),
            core_otel::redact::UNMATCHED_ROUTE
        );
        assert!(core_otel::redact::ensure_low_cardinality("/api/tickets/42").is_err());
        assert!(core_otel::redact::ensure_low_cardinality("/api/tickets/{id}").is_ok());
    }

    /// The middleware composes into a Router without changing responses
    /// (status and body pass through untouched).
    #[tokio::test]
    async fn track_passes_responses_through() {
        async fn handler() -> &'static str {
            "ok"
        }
        let app = Router::new()
            .route("/api/health", get(handler))
            .layer(axum::middleware::from_fn(track));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(axum::body::Body::empty())
                    .expect("static request"),
            )
            .await
            .expect("in-process call");
        assert_eq!(response.status(), 200);
    }

    /// Correlation ids: a request with no inbound header gets a fresh
    /// UUIDv7 echoed on the response; a sane inbound id is honored
    /// verbatim; a junk inbound id is replaced (never echoed).
    #[tokio::test]
    async fn correlate_generates_echoes_and_scrubs_ids() {
        async fn handler() -> &'static str {
            "ok"
        }
        let app = Router::new()
            .route("/api/health", get(handler))
            .layer(axum::middleware::from_fn(correlate));

        let fresh = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let echoed = fresh
            .headers()
            .get("x-correlation-id")
            .and_then(|v| v.to_str().ok())
            .expect("fresh request still gets an id");
        assert!(uuid::Uuid::parse_str(echoed).is_ok(), "{echoed} not a UUID");

        let honor = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("x-correlation-id", "abc-123-XYZ")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            honor.headers().get("x-correlation-id").unwrap(),
            "abc-123-XYZ"
        );

        let scrubbed = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    // Space is not in the allowed set (valid as a header
                    // value, but not a correlation id) — it must be
                    // replaced by a server-generated id.
                    .header("x-correlation-id", "bad id value")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let id = scrubbed
            .headers()
            .get("x-correlation-id")
            .and_then(|v| v.to_str().ok())
            .expect("scrubbed request still gets an id");
        assert!(uuid::Uuid::parse_str(id).is_ok(), "{id} not a fresh UUID");
    }
}
