//! worker/retrieval_watch — the per-tenant embedding count behind the retrieval
//! mode decision (Wave 52 lane R15, `p52-retrieval-mode-config`).
//!
//! Purpose: the retrieval mode is a deployment lever, and the fact that decides
//! it is how many embeddings each tenant stores. The exact vector scan fits the
//! search leg until one tenant crosses roughly 20,000 embeddings; past that the
//! deployment wants `iterative` or `index`. Nobody can make that call from a
//! mean, because the mean hides the one tenant that crossed.
//!
//! So this sweep reads `search_embedding_distribution()` (migration 0153, a
//! `SECURITY DEFINER` count per org) every `MARULA_RETRIEVAL_WATCH_SECS` and
//! records `marula.embedding.count{org}`. It changes no mode: the gauge is
//! evidence for a person.
//!
//! Three decisions worth the ink:
//!
//! - **A count, never content.** The function returns `count(*)` per org. The
//!   gauge carries an org id and a number, so the report exposes no row data.
//! - **Per org, not a total.** A single total over every tenant would hide the
//!   one that crossed the threshold, which is the only number that matters.
//! - **Housekeeping only.** Two replicas running this would record the same
//!   series twice with no harm, but the sweep is registered with the other
//!   housekeeping loops so a scaled deployment runs it once.
//!
//! Non-purpose: no mode switching, no index creation (the migrate step owns
//! DDL), and no alert rule — the descriptor's distribution report and this
//! gauge are the same evidence read two ways.

use std::time::Duration;

use core_domain::retry::RETRIEVAL_WATCH_RETRY;
use sqlx::PgPool;
use tokio::sync::watch;

/// The environment lever for the cadence.
pub const WATCH_ENV: &str = "MARULA_RETRIEVAL_WATCH_SECS";

/// Default cadence. Slow on purpose: the count moves with the re-index job, not
/// with requests, and a stricter reading would be a stale number charted more
/// often.
pub const DEFAULT_INTERVAL_SECS: u64 = 300;

/// What one tick saw, for the log line and for tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchReport {
    /// Tenants with at least one embedding.
    pub tenants: usize,
    /// Tenants at or past the threshold where an index starts to pay.
    pub over_threshold: usize,
    /// Embeddings across every tenant, which is the install's total.
    pub total_embeddings: i64,
}

/// Read the distribution once and record the gauge.
///
/// # Errors
///
/// Returns a [`core_db::DbError`] when the function is missing (the database is
/// not migrated) or the read fails. A caller logs and backs off.
pub async fn sweep_once(pool: &PgPool) -> core_db::DbResult<WatchReport> {
    let distribution = core_db::retrieval::embedding_distribution(pool).await?;
    let mut report = WatchReport {
        tenants: distribution.len(),
        ..WatchReport::default()
    };
    for tenant in &distribution {
        if tenant.index_recommended {
            report.over_threshold += 1;
        }
        report.total_embeddings += tenant.embeddings;
        // `i64` is what SQL counts in; the gauge is unsigned, and a negative
        // count cannot happen. Saturating keeps a malformed row from panicking.
        core_otel::metrics::embedding_count(
            &tenant.org_id.to_string(),
            u64::try_from(tenant.embeddings).unwrap_or(0),
        );
    }
    tracing::debug!(
        tenants = report.tenants,
        over_threshold = report.over_threshold,
        total_embeddings = report.total_embeddings,
        "retrieval distribution recorded"
    );
    Ok(report)
}

/// The loop. Reads once, then sleeps; a failure backs off rather than spins.
pub async fn run(pool: PgPool, interval_secs: u64, mut shutdown: watch::Receiver<bool>) {
    let interval = Duration::from_secs(interval_secs.max(30));
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            result = sweep_once(&pool) => {
                match result {
                    Ok(report) => {
                        if report.over_threshold > 0 {
                            tracing::info!(
                                tenants = report.tenants,
                                over_threshold = report.over_threshold,
                                "{} tenant(s) are past the retrieval-mode threshold; \
                                 `exact` no longer fits the search leg for them",
                                report.over_threshold
                            );
                        }
                        tokio::time::sleep(interval).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "retrieval distribution read failed");
                        tokio::time::sleep(RETRIEVAL_WATCH_RETRY).await;
                    }
                }
            }
        }
    }
}

/// Resolve the cadence from an environment lookup, falling back to
/// [`DEFAULT_INTERVAL_SECS`]. A value below the floor is raised to it: a
/// one-second count of every tenant would be a query nobody asked for.
#[must_use]
pub fn interval_from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> u64 {
    lookup(WATCH_ENV)
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(30)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(map: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: std::collections::HashMap<String, String> = map
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key: &str| owned.get(key).cloned()
    }

    #[test]
    fn the_cadence_defaults_and_has_a_floor() {
        assert_eq!(
            interval_from_lookup(&lookup(&[])),
            DEFAULT_INTERVAL_SECS,
            "unset is the documented five minutes"
        );
        assert_eq!(interval_from_lookup(&lookup(&[(WATCH_ENV, "900")])), 900);
        assert_eq!(
            interval_from_lookup(&lookup(&[(WATCH_ENV, "1")])),
            30,
            "a one-second count of every tenant is not a cadence"
        );
        assert_eq!(
            interval_from_lookup(&lookup(&[(WATCH_ENV, "soon")])),
            DEFAULT_INTERVAL_SECS,
            "an unparseable value falls back rather than stopping the sweep"
        );
    }
}
