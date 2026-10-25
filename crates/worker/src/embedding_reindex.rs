//! worker/embedding_reindex — the clock behind the embedding model change
//! (Wave 52 lane R14, `p52-embedding-model-catalogue`).
//!
//! Purpose: when the instance's embedding model changes, every stored vector in
//! the old space has to be rewritten in the new one. A migration cannot call a
//! model, so the work is a job; this loop is its clock. Per tick it:
//!
//! 1. reads the instance's recorded space (core-api records it at boot);
//! 2. queues one run per org that still holds rows outside that space —
//!    idempotent on `(switch_id, org_id)`, so a restart re-queues nothing;
//! 3. claims a run, asks core-api for one bounded pass, and paces itself.
//!
//! Three decisions worth the ink:
//!
//! - **The worker queues, core-api embeds.** The insert spans orgs, and only the
//!   worker's owner connection may do that; every embedding write stays in
//!   core-api (`/internal/search/backfill`), so there is still exactly one
//!   writer for the vectors.
//! - **Resumability is the predicate, not a cursor.** A pass selects rows with
//!   no fresh vector in the target space, so a crash repeats at most one batch
//!   and cannot skip a row. Nothing here tracks a position.
//! - **The rate is the pace, not a burst.** `batch x 60000 / rows_per_min`
//!   milliseconds after every pass keeps a re-index from eating the capacity
//!   live search needs; the default leaves about a third of R13's measured
//!   960 rows/min.
//!
//! Non-purpose: no window arithmetic beyond [`ReindexPolicy::allows_hour`], no
//! retrieval changes (fusing the two readable spaces is core-db's), and no
//! deletion of old rows — core-api drops an org's previous-space rows in the
//! same transaction that marks its run complete.

use std::time::Duration;

use chrono::Timelike;
use core_domain::embedding::ReindexPolicy;
use core_domain::retry::EMBEDDING_REINDEX_RETRY;
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

/// Runs worked per tick. Bounds how long one tick can hold the loop, so a
/// shutdown is honoured between runs rather than after a whole install drains.
pub const RUNS_PER_TICK: usize = 8;

/// What one pass of the sweep did, for the log line and for tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Runs this tick worked (a run may span several ticks).
    pub runs: usize,
    /// Runs created by the enqueue step.
    pub queued: u64,
    /// Rows core-api embedded across those runs.
    pub embedded: usize,
    /// Runs whose leg is dark (no credential, AI off, provider down).
    pub dark_runs: usize,
    /// True when the tick had nothing to do at all.
    pub idle: bool,
}

/// The pass core-api runs for one org: the shape of `BackfillReport` the worker
/// depends on, and nothing else. A mirror rather than a shared type because the
/// worker must not link core-api.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BackfillPass {
    #[serde(default)]
    pub embedded: usize,
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub dark: Option<String>,
    #[serde(default)]
    pub remaining: i64,
    #[serde(default)]
    pub total_rows: i64,
}

#[async_trait::async_trait]
pub trait BackfillClient: Send + Sync {
    /// Runs one bounded pass for one org, advancing `run_id` if given.
    async fn backfill(
        &self,
        org_id: Uuid,
        run_id: Option<Uuid>,
        max_batches: i64,
    ) -> anyhow::Result<BackfillPass>;
}

/// The HTTP client for core-api's internal surface. The endpoint is idempotent,
/// so a retried pass is safe; the timeout is generous because a local batch of
/// 128 inputs is measured in seconds, not milliseconds.
pub struct HttpBackfillClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl HttpBackfillClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl BackfillClient for HttpBackfillClient {
    async fn backfill(
        &self,
        org_id: Uuid,
        run_id: Option<Uuid>,
        max_batches: i64,
    ) -> anyhow::Result<BackfillPass> {
        let response = self
            .http
            .post(format!("{}/internal/search/backfill", self.base_url))
            .header("x-internal-token", &self.token)
            .json(&serde_json::json!({
                "org_id": org_id,
                "max_batches": max_batches,
                "run_id": run_id,
            }))
            .timeout(Duration::from_secs(300))
            .send()
            .await?;
        let response = response.error_for_status()?;
        Ok(response.json().await?)
    }
}

/// Batches per pass: the policy's row budget in `EMBEDDING_BATCH` units, at
/// least one.
fn batches_per_pass(policy: &ReindexPolicy) -> i64 {
    (policy.batch / core_db::search::EMBEDDING_BATCH).max(1)
}

/// One tick: queue what is missing, then work what is due.
///
/// `now_hour` is the UTC hour, injected so a test can place the sweep inside or
/// outside the window without waiting for the clock.
pub async fn sweep_once(
    pool: &PgPool,
    policy: &ReindexPolicy,
    client: &dyn BackfillClient,
    now_hour: u8,
) -> anyhow::Result<SweepReport> {
    let mut report = SweepReport::default();
    if !policy.allows_hour(now_hour) {
        report.idle = true;
        return Ok(report);
    }
    let state = core_db::embedding_space::read_state(pool).await?;
    let Some(target) = state.target_space.clone() else {
        // core-api has not recorded a space yet (first boot after 0152, or a
        // worker started before the API). Nothing to queue and nothing readable.
        report.idle = true;
        return Ok(report);
    };
    if let Some(switch_id) = state.switch_id {
        report.queued =
            core_db::embedding_space::enqueue_missing_runs(pool, switch_id, &target).await?;
        if report.queued > 0 {
            tracing::info!(
                switch_id = %switch_id,
                to_space = %target,
                queued = report.queued,
                "queued per-org re-index runs"
            );
        }
    }
    core_otel::metrics::embedding_reindex_runs_pending(
        u64::try_from(core_db::embedding_space::runs_pending(pool).await?).unwrap_or(u64::MAX),
    );

    for _ in 0..RUNS_PER_TICK {
        let Some(run) = core_db::embedding_space::next_run(pool).await? else {
            break;
        };
        if run.to_space != target {
            // A second model change landed before this org was reached. Rewriting
            // rows into the superseded space would be read by nothing.
            let mut conn = pool.acquire().await?;
            core_db::embedding_space::fail_run(
                &mut conn,
                run.id,
                &format!(
                    "superseded: this run targets `{}`, the instance now writes `{target}`",
                    run.to_space
                ),
            )
            .await?;
            continue;
        }
        let mut conn = pool.acquire().await?;
        if !core_db::embedding_space::lease(&mut conn, run.id).await? {
            // Another sweep claimed it between `next_run` and here.
            continue;
        }
        drop(conn);

        let pass = match client
            .backfill(run.org_id, Some(run.id), batches_per_pass(policy))
            .await
        {
            Ok(pass) => pass,
            Err(error) => {
                // Transport or 5xx: keep the run, say why, retry next tick.
                let mut conn = pool.acquire().await?;
                core_db::embedding_space::note_error(&mut conn, run.id, &error.to_string()).await?;
                tracing::warn!(org_id = %run.org_id, %error, "re-index pass failed; will retry");
                report.runs += 1;
                report.idle = false;
                continue;
            }
        };

        report.runs += 1;
        report.idle = false;
        report.embedded += pass.embedded;
        if pass.embedded > 0 {
            core_otel::metrics::embedding_reindex_embedded(
                &run.org_id.to_string(),
                u64::try_from(pass.embedded).unwrap_or(u64::MAX),
            );
        }
        core_otel::metrics::embedding_reindex_remaining(
            &run.org_id.to_string(),
            u64::try_from(pass.remaining.max(0)).unwrap_or(u64::MAX),
        );

        let mut conn = pool.acquire().await?;
        if pass.complete {
            // core-api already closed the run and dropped the previous space's
            // rows in the same transaction; releasing a closed run is a no-op.
            core_db::embedding_space::release(&mut conn, run.id).await?;
            tracing::info!(
                org_id = %run.org_id,
                embedded = pass.embedded,
                rows = pass.total_rows,
                "re-index complete for org"
            );
        } else if pass.dark.is_some() || pass.embedded == 0 {
            // Dark leg or a pass that could not move: the message is the
            // operator's to act on, and a retry costs one request per tick.
            report.dark_runs += 1;
            let reason = pass
                .reason
                .clone()
                .unwrap_or_else(|| "the pass embedded nothing".to_string());
            core_db::embedding_space::note_error(&mut conn, run.id, &reason).await?;
            tracing::warn!(
                org_id = %run.org_id,
                remaining = pass.remaining,
                reason = %reason,
                "re-index pass made no progress"
            );
        } else {
            core_db::embedding_space::release(&mut conn, run.id).await?;
        }
        drop(conn);

        // The rate limit. Every pass pays it, including the last of a tick: the
        // next tick starts immediately, so the pause is what keeps the whole job
        // under `rows_per_min` rather than only the call itself.
        tokio::time::sleep(policy.pace()).await;
    }
    Ok(report)
}

/// The loop: work while there is work, look again later when there is not.
///
/// The pacing lives inside [`sweep_once`] (one pause per pass), so a tick with
/// work is followed immediately by the next tick and the row ceiling is the
/// pace, nothing else. `interval_secs` is the idle cadence — how often a
/// caught-up install, or one outside its window, looks again.
pub async fn run(
    pool: PgPool,
    policy: ReindexPolicy,
    client: std::sync::Arc<dyn BackfillClient>,
    mut shutdown: watch::Receiver<bool>,
) {
    tracing::info!(
        batch = policy.batch,
        rows_per_min = policy.rows_per_min,
        idle_secs = policy.interval_secs,
        window = ?policy.window,
        "embedding re-index sweep enabled"
    );
    loop {
        if *shutdown.borrow() {
            return;
        }
        let hour = u8::try_from(chrono::Utc::now().hour()).unwrap_or(0);
        let pause = tokio::select! {
            _ = shutdown.changed() => return,
            result = sweep_once(&pool, &policy, client.as_ref(), hour) => match result {
                Ok(report) if report.idle => Duration::from_secs(policy.interval_secs),
                Ok(_report) => Duration::ZERO,
                Err(error) => {
                    tracing::error!(%error, "embedding re-index tick failed");
                    EMBEDDING_REINDEX_RETRY
                }
            },
        };
        if pause.is_zero() {
            continue;
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            _ = tokio::time::sleep(pause) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn policy(vars: &[(&str, &str)]) -> ReindexPolicy {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        ReindexPolicy::from_lookup(&|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let policy = policy(&[]);
        assert_eq!(policy.rows_per_min, 600);
        assert_eq!(policy.interval_secs, 60);
        assert_eq!(policy.window, None);
        assert_eq!(batches_per_pass(&policy), 1);
    }

    #[test]
    fn a_zero_knob_falls_back_instead_of_stopping_the_job() {
        let policy = policy(&[
            ("MARULA_EMBEDDING_REINDEX_ROWS_PER_MIN", "0"),
            ("MARULA_EMBEDDING_REINDEX_SECS", "0"),
        ]);
        assert_eq!(policy.rows_per_min, 600);
        assert_eq!(policy.interval_secs, 60);
    }

    #[test]
    fn the_pace_keeps_a_pass_under_the_row_ceiling() {
        let policy = policy(&[
            ("MARULA_EMBEDDING_REINDEX_BATCH", "128"),
            ("MARULA_EMBEDDING_REINDEX_ROWS_PER_MIN", "600"),
        ]);
        // 128 rows at 600/min is 12.8 s of budget.
        assert_eq!(policy.pace(), Duration::from_millis(12_800));
    }

    #[test]
    fn batches_per_pass_scales_with_the_row_budget() {
        assert_eq!(
            batches_per_pass(&ReindexPolicy {
                batch: 512,
                ..ReindexPolicy::default()
            }),
            4
        );
        assert_eq!(
            batches_per_pass(&ReindexPolicy {
                batch: 1,
                ..ReindexPolicy::default()
            }),
            1,
            "a pass always asks for at least one batch"
        );
    }

    #[test]
    fn the_window_gates_the_tick_before_any_database_work() {
        let policy = policy(&[("MARULA_EMBEDDING_REINDEX_WINDOW", "22-06")]);
        assert!(policy.allows_hour(23));
        assert!(policy.allows_hour(5));
        assert!(!policy.allows_hour(12));
    }

    /// A pass that reports a dark leg must leave the run alive, so the operator
    /// who fixes the credential sees it finish rather than a dead run.
    #[tokio::test]
    async fn a_dark_pass_keeps_the_run_and_a_superseded_one_does_not() {
        // The client contract the sweep depends on.
        struct Dark;
        #[async_trait::async_trait]
        impl BackfillClient for Dark {
            async fn backfill(
                &self,
                _org_id: Uuid,
                _run_id: Option<Uuid>,
                _max_batches: i64,
            ) -> anyhow::Result<BackfillPass> {
                Ok(BackfillPass {
                    embedded: 0,
                    complete: false,
                    reason: Some("no credential".to_string()),
                    dark: Some("missing_credential".to_string()),
                    remaining: 10,
                    total_rows: 10,
                })
            }
        }
        let calls = Mutex::new(0_usize);
        struct Counter<'a>(&'a Mutex<usize>);
        #[async_trait::async_trait]
        impl BackfillClient for Counter<'_> {
            async fn backfill(
                &self,
                _org_id: Uuid,
                _run_id: Option<Uuid>,
                _max_batches: i64,
            ) -> anyhow::Result<BackfillPass> {
                *self.0.lock().expect("lock") += 1;
                Ok(BackfillPass {
                    embedded: 0,
                    complete: false,
                    reason: None,
                    dark: None,
                    remaining: 5,
                    total_rows: 10,
                })
            }
        }
        // The fake is used only to pin the shape: `dark` is a stable id, not
        // prose, so a caller can branch on it.
        let pass = Dark.backfill(Uuid::nil(), None, 1).await.expect("pass");
        assert_eq!(pass.dark.as_deref(), Some("missing_credential"));
        let counter = Counter(&calls);
        counter.backfill(Uuid::nil(), None, 1).await.expect("pass");
        assert_eq!(*calls.lock().expect("lock"), 1);
    }
}
