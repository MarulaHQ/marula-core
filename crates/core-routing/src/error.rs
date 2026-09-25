//! Typed routing failures. Every provider maps its vendor errors onto
//! this vocabulary; no vendor error string is ever surfaced verbatim
//! (excerpts are truncated and credential-free by construction — the
//! adapters never put a token in the URL they log or echo).

/// The closed failure vocabulary of the routing seam.
#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    /// The operator named a provider in config that the registry does not
    /// hold — every measured path degrades to the heuristic, visibly.
    #[error("routing provider `{provider}` is configured but not registered")]
    NotConfigured { provider: String },
    /// No provider is configured at all (the honest default).
    #[error("no routing provider is configured")]
    NoProvider,
    /// The provider exists but does not implement the requested capability.
    #[error("provider `{provider}` does not support {capability}")]
    Unsupported {
        provider: String,
        capability: &'static str,
    },
    /// The request itself is malformed (empty matrix, both time controls,
    /// over-cap batch, …) — retrying identically is futile.
    #[error("invalid routing request: {0}")]
    Invalid(String),
    /// The provider rejected the operator's credential.
    #[error("routing provider rejected the configured credential")]
    Unauthorized,
    /// The provider asked the caller to slow down.
    #[error("routing provider rate-limited the request")]
    RateLimited { retry_after_secs: Option<i64> },
    /// Transport-level failure: DNS, TLS, timeout. Retry with backoff.
    #[error("routing provider unavailable: {0}")]
    Unavailable(String),
    /// The provider answered, but not with a shape the seam understands.
    #[error("routing provider protocol error: {0}")]
    Protocol(String),
    /// Non-success HTTP status; `excerpt` is a short, credential-free
    /// slice of the body for diagnostics.
    #[error("routing provider returned {status}: {excerpt}")]
    Upstream { status: u16, excerpt: String },
}

impl RoutingError {
    /// Stable machine label for typed API surfaces (warnings, status
    /// views). Never localized copy.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            RoutingError::NotConfigured { .. } => "provider_not_configured",
            RoutingError::NoProvider => "no_provider",
            RoutingError::Unsupported { .. } => "capability_unsupported",
            RoutingError::Invalid(_) => "invalid_request",
            RoutingError::Unauthorized => "provider_unauthorized",
            RoutingError::RateLimited { .. } => "provider_rate_limited",
            RoutingError::Unavailable(_) => "provider_unavailable",
            RoutingError::Protocol(_) => "provider_protocol_error",
            RoutingError::Upstream { .. } => "provider_upstream_error",
        }
    }

    /// Whether a bounded retry with backoff may succeed. Used by callers
    /// that choose to retry; the seam itself never retries.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            RoutingError::RateLimited { .. } | RoutingError::Unavailable(_) => true,
            RoutingError::Upstream { status, .. } => *status >= 500,
            _ => false,
        }
    }

    /// Builds an `Upstream` error from a response body with a hard,
    /// single-line excerpt cap. Adapters call this INSTEAD of embedding
    /// raw bodies (bodies can echo the request; requests must never carry
    /// credentials in URLs).
    #[must_use]
    pub fn upstream(status: u16, body: &str) -> Self {
        let mut excerpt: String = body
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        if excerpt.is_empty() {
            excerpt = "<empty body>".into();
        }
        RoutingError::Upstream { status, excerpt }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_excerpt_is_single_line_and_capped() {
        let long = format!("{}\nsecond line", "x".repeat(500));
        let err = RoutingError::upstream(500, &long);
        match err {
            RoutingError::Upstream { excerpt, .. } => {
                assert_eq!(excerpt.len(), 200);
                assert!(!excerpt.contains('\n'));
            }
            other => panic!("expected upstream, got {other:?}"),
        }
    }

    #[test]
    fn transient_classification_is_stable() {
        assert!(RoutingError::RateLimited {
            retry_after_secs: Some(3)
        }
        .is_transient());
        assert!(RoutingError::Unavailable("timeout".into()).is_transient());
        assert!(RoutingError::Upstream {
            status: 503,
            excerpt: "x".into()
        }
        .is_transient());
        assert!(!RoutingError::Upstream {
            status: 400,
            excerpt: "x".into()
        }
        .is_transient());
        assert!(!RoutingError::Unauthorized.is_transient());
    }
}
