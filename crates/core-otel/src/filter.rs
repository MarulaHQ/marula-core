//! Log/span filter composition — one policy for every Marula process.
//!
//! Purpose: build the `tracing_subscriber::EnvFilter` identically in
//! core-api, realtime, worker, and marula-core-cli so telemetry levels never
//! drift between lanes.
//!
//! Precedence:
//! 1. `RUST_LOG` (standard env-filter syntax) wins outright when set.
//! 2. Otherwise the default directive set applies: `info,tower_http=debug`
//!    (request spans stay visible; everything else at info).
//!
//! sqlx query spans are OFF by default. sqlx 0.8 emits one DEBUG event per
//! executed statement under the `sqlx::query` target containing the
//! statement SUMMARY ONLY (normalized shape like
//! `SELECT … FROM tickets WHERE org_id = $1`) — bind VALUES are never part
//! of it. Opt in with `MARULA_SQLX_QUERIES=on|1|true`, which appends
//! `sqlx=debug` to the default set (or set `sqlx=debug` inside RUST_LOG).
//! WARN-level slow-query events from sqlx remain visible at info because
//! they are emitted above the default level.
//!
//! Redaction note: this module never adds attribute content itself; it
//! only gates existing targets. The redaction contract lives in
//! [`crate::redact`] and applies regardless of log level.

use tracing_subscriber::EnvFilter;

/// Environment variable enabling per-statement sqlx query events.
pub const SQLX_QUERIES_VAR: &str = "MARULA_SQLX_QUERIES";

/// The default directives when `RUST_LOG` is unset.
pub const DEFAULT_DIRECTIVES: &str = "info,tower_http=debug";

/// Pure directive-string composition (`RUST_LOG`, opt-in flag) so policy
/// is unit-testable without touching global subscriber state. `rust_log`
/// and `sqlx_queries` mirror the corresponding env vars.
#[must_use]
pub fn compose_directives(rust_log: Option<&str>, sqlx_queries: bool) -> String {
    match rust_log.map(str::trim).filter(|s| !s.is_empty()) {
        // RUST_LOG is authoritative; the opt-in var must not fight it.
        Some(rust_log) => rust_log.to_string(),
        None if sqlx_queries => format!("{DEFAULT_DIRECTIVES},sqlx=debug"),
        None => DEFAULT_DIRECTIVES.to_string(),
    }
}

fn sqlx_opt_in(raw: Option<String>) -> bool {
    raw.map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "on" | "1" | "true"))
        .unwrap_or(false)
}

/// Builds the process-wide env filter per the module rules.
#[must_use]
pub fn from_env() -> EnvFilter {
    let explicit = std::env::var("RUST_LOG").ok();
    let flag = sqlx_opt_in(std::env::var(SQLX_QUERIES_VAR).ok());
    EnvFilter::new(compose_directives(explicit.as_deref(), flag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keeps_sqlx_query_spans_off() {
        assert_eq!(compose_directives(None, false), DEFAULT_DIRECTIVES);
        assert!(!compose_directives(None, false).contains("sqlx"));
    }

    #[test]
    fn sqlx_opt_in_appends_debug_directive() {
        let out = compose_directives(None, true);
        assert_eq!(out, "info,tower_http=debug,sqlx=debug");
        assert!(out.ends_with("sqlx=debug"));
    }

    #[test]
    fn rust_log_overrides_everything() {
        assert_eq!(
            compose_directives(Some("warn,marula=trace"), true),
            "warn,marula=trace"
        );
    }

    #[test]
    fn empty_rust_log_falls_back_to_defaults() {
        // Empty/blank RUST_LOG means "not specified": defaults apply and
        // the opt-in flag stays in force.
        assert_eq!(compose_directives(Some("   "), false), DEFAULT_DIRECTIVES);
        assert_eq!(
            compose_directives(Some(""), true),
            "info,tower_http=debug,sqlx=debug"
        );
    }

    /// The env→flag mapping accepts exactly on/1/true (case-insensitive).
    #[test]
    fn opt_in_values_are_exact() {
        assert!(!sqlx_opt_in(None));
        assert!(!sqlx_opt_in(Some("off".into())));
        assert!(!sqlx_opt_in(Some("".into())));
        for v in ["on", "1", "TRUE", "True"] {
            assert!(sqlx_opt_in(Some(v.into())), "{v} must enable");
        }
    }

    /// The composed string parses into a working EnvFilter for both modes.
    #[test]
    fn composition_yields_valid_filters() {
        for (log, flag) in [(None, false), (None, true), (Some("warn"), false)] {
            let s = compose_directives(log, flag);
            let _filter = EnvFilter::try_new(&s)
                .unwrap_or_else(|error| panic!("directives must parse: {s}: {error}"));
        }
    }
}
