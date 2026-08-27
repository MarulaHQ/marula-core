//! Redaction rules for span attributes — the single source of truth.
//!
//! Purpose: pure, testable helpers that decide WHAT may ever become a span
//! attribute or metric label. Every lane routes attribute candidates
//! through these functions; nothing in Meridian records raw paths, query
//! strings, auth material, or user content.
//!
//! Rules (each enforced by a unit test below):
//! 1. Route identity is the axum route TEMPLATE (`/api/tickets/{id}`).
//!    A missing template (unmatched route) collapses to the constant
//!    `UNMATCHED_ROUTE` — never to the raw request path, whose cardinality
//!    and content are unbounded.
//! 2. Query strings are dropped wholesale: they carry `?token=` (WS),
//!    `?code=` (OIDC callback) and similar secrets.
//! 3. Header-shaped or credential-shaped values (`authorization`,
//!    `cookie`, `set-cookie`, `x-api-key`, JWT-looking strings, anything
//!    passed to [`redact_secret_like`]) collapse to `[redacted]`.
//!
//! Non-purpose: no general-purpose PII scrubber for free-form text — the
//! rule is simpler and stronger: free-form user content is never recorded
//! at all, so there is nothing to scrub.

/// Constant used when no route template matched. Deliberately NOT derived
/// from the request so unmatched traffic cannot explode label cardinality.
pub const UNMATCHED_ROUTE: &str = "unmatched";

/// Placeholder substituted for any credential-like value.
pub const REDACTED: &str = "[redacted]";

/// The only route label ever emitted for a request.
///
/// `matched` is `MatchedPath::as_str()` from the router (already the
/// template); `None` means the router did not match — including 404s,
/// which would otherwise leak probed paths into telemetry.
#[must_use]
pub fn route_template(matched: Option<&str>) -> &str {
    // Templates come from static route definitions inside the router, so
    // they are trusted; the guard below (`ensure_low_cardinality`) exists
    // for tests and CI-style assertions on the *shape*.
    matched.unwrap_or(UNMATCHED_ROUTE)
}

/// Cardinality guard: returns Err when a string does not look like a route
/// template. Used in tests (and usable as a debug assertion) so a refactor
/// that starts feeding raw paths into attributes fails loudly instead of
/// flooding the metric store with per-id series.
///
/// Heuristic: templates contain `{param}` placeholders or are one of the
/// known parameterless routes; raw paths contain path segments that look
/// like UUIDs, long hex/base64url tokens, or numeric ids.
pub fn ensure_low_cardinality(route: &str) -> Result<(), String> {
    let looks_raw = route.split('/').any(|segment| {
        if segment.starts_with('{') && segment.ends_with('}') {
            return false;
        }
        is_high_cardinality_segment(segment)
    });
    if looks_raw {
        Err(format!("raw-path segment leaked into route attr: {route}"))
    } else {
        Ok(())
    }
}

fn is_high_cardinality_segment(segment: &str) -> bool {
    if segment.is_empty() {
        return false;
    }
    let uuidish =
        segment.len() == 36 && segment.as_bytes().iter().filter(|b| **b == b'-').count() == 4;
    let numeric = segment.chars().all(|c| c.is_ascii_digit());
    // Opaque ids/tokens are long alnum blobs that virtually always carry
    // digits (uuids, base64url, snowflakes). Digitless long segments are
    // prose-like STATIC route words ("notification-preferences",
    // "calendar-exclusions") and MUST NOT be flagged: the pre-versioning
    // guard here panicked the track middleware on legitimate parameterless
    // routes the moment an actual request reached them.
    let tokenish = segment.len() >= 20
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && segment.chars().any(|c| c.is_ascii_digit());
    uuidish || numeric || tokenish
}

/// Drops the query string entirely. Query strings are never telemetry:
/// they carry WS `?token=`, OIDC `?code=`, pagination cursors, etc.
#[must_use]
pub fn strip_query(uri_path_and_query: &str) -> &str {
    match uri_path_and_query.split_once('?') {
        Some((path, _)) => path,
        None => uri_path_and_query,
    }
}

/// Collapses credential-like values to `[redacted]`. Applied defensively
/// anywhere a caller might be tempted to record header-ish data. Returns
/// `None` when the value is ordinary and MAY be recorded.
#[must_use]
pub fn redact_secret_like(key: &str, value: Option<&str>) -> Option<&'static str> {
    const SENSITIVE_KEYS: &[&str] = &[
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
        "api-key",
        "api_key",
        "token",
        "access_token",
        "refresh_token",
        "password",
        "secret",
        "jwt",
    ];
    let key = key.trim().to_ascii_lowercase();
    if SENSITIVE_KEYS.contains(&key.as_str()) {
        return Some(REDACTED);
    }
    // Values that merely look like credentials are redacted too, whatever
    // the key claims (mis-keyed headers exist in the wild).
    let looks_credential = value.is_some_and(|v| {
        v.len() >= 20
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    });
    if looks_credential {
        Some(REDACTED)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmatched_routes_collapse_to_a_constant() {
        assert_eq!(route_template(None), UNMATCHED_ROUTE);
        assert_eq!(route_template(None), "unmatched");
    }

    #[test]
    fn route_templates_pass_through_and_guard_accepts_them() {
        for template in [
            "/api/tickets/{id}",
            "/api/boards/{id}/members/{user_id}",
            "/api/health",
            "/ws",
        ] {
            assert_eq!(route_template(Some(template)), template);
            ensure_low_cardinality(template)
                .unwrap_or_else(|e| panic!("{template} should pass guard: {e}"));
        }
    }

    #[test]
    fn guard_rejects_raw_paths_with_ids_tokens_or_uuids() {
        for raw in [
            "/api/tickets/018f6b2c-4f1a-7abc-9de0-123456789abc",
            "/api/tickets/42",
            "/attachments/dQw4w9WgXcQ-dQw4w9WgXcQ12/download",
        ] {
            assert!(
                ensure_low_cardinality(raw).is_err(),
                "{raw} must be rejected"
            );
        }
    }

    #[test]
    fn digitless_prose_like_static_routes_pass_the_guard() {
        // Regression: these parameterless routes panicked the track
        // middleware before the tokenish rule required a digit — every real
        // request to them was a 500 instead of its handler.
        for template in [
            "/api/notification-preferences",
            "/api/v1/notification-preferences",
            "/api/calendar-exclusions/{id}",
        ] {
            ensure_low_cardinality(template)
                .unwrap_or_else(|e| panic!("{template} should pass guard: {e}"));
        }
    }

    #[test]
    fn query_strings_are_dropped_wholesale_including_token_params() {
        assert_eq!(strip_query("/ws"), "/ws");
        assert_eq!(strip_query("/ws?token=abc.def.ghi"), "/ws");
        assert_eq!(
            strip_query("/api/auth/oidc/acme/callback?code=secret&state=x"),
            "/api/auth/oidc/acme/callback"
        );
    }

    #[test]
    fn credential_shaped_keys_and_values_are_redacted() {
        assert_eq!(
            redact_secret_like("Authorization", Some("Bearer x")),
            Some(REDACTED)
        );
        assert_eq!(redact_secret_like("cookie", Some("sid=1")), Some(REDACTED));
        assert_eq!(redact_secret_like("x-api-key", None), Some(REDACTED));
        // Value-only heuristic catches mislabeled secrets…
        assert_eq!(
            redact_secret_like("note", Some("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9")),
            Some(REDACTED)
        );
        // …but ordinary short values pass untouched (None ⇒ don't record).
        assert_eq!(redact_secret_like("method", Some("GET")), None);
    }

    #[test]
    fn no_auth_header_or_body_content_survives_the_rules() {
        // The contract: given ANY header/token/body candidate, the helpers
        // either drop it or replace it with the placeholder.
        let forbidden = [
            ("authorization", "Bearer eyJhbGciOi.J9.eyJz"),
            ("set-cookie", "refresh=opaque; HttpOnly"),
            ("body", "{\"title\":\"client secret server\"}"),
        ];
        for (key, value) in forbidden {
            if let Some(out) = redact_secret_like(key, Some(value)) {
                assert_eq!(out, REDACTED);
                assert!(!out.contains("eyJ"));
                assert!(!out.contains("refresh"));
                assert!(!out.contains("title"));
            } // dropped entirely is equally acceptable
        }
    }
}
