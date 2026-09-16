//! HTTP load runner (deliverable 2b) — drives a RUNNING core-api through
//! the real `GET /api/tickets` endpoint with a deterministic request mix
//! and reports end-to-end p50/p95/p99 latency.
//!
//! This is the scale leg of the perf program: seed at 100k tickets/org
//! (`SeedScale::load`), boot core-api against that DB, then run
//! `perf-harness load` against it. Runs are reproducible: the request mix
//! per worker is driven by an LCG seeded from `--seed`, so two runs with
//! the same parameters issue the same requests in the same order.
//!
//! The runner logs in through the real auth flow (`POST /api/auth/login`)
//! with the seeded harness user, so the measured path includes bearer-token
//! auth, RLS tx setup, serialization and the axum stack — everything the
//! SQL-path measurement (`measure`) deliberately excludes.
//!
//! Deep-page leg (Wave 15 lane 3): the mix's 10% deep-page share rides the
//! KEYSET cursor (the product shape, F2) — one untimed offset call at
//! login resolves the depth-40k anchor from the server's `x-next-cursor`
//! header, and workers page from there. Against a pre-keyset server the
//! anchor is absent and the leg falls back to the legacy offset URL, so
//! the runner stays usable across versions.

use anyhow::Context;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::seed::md5_uuid;
use crate::{PERF_ORG_ID, PERF_PASSWORD, PERF_USER_EMAIL};

/// Configuration for one load run.
#[derive(Debug, Clone)]
pub struct LoadConfig {
    /// Base URL of a running core-api, e.g. `http://127.0.0.1:18080`.
    pub base_url: String,
    /// How long to drive load.
    pub duration: Duration,
    /// Concurrent worker tasks; each issues requests back-to-back.
    pub concurrency: usize,
    /// Seed for the deterministic request mix.
    pub seed: u64,
    /// Fail the run when p95 exceeds this budget (milliseconds).
    pub budget_ms: Option<f64>,
}

impl Default for LoadConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:18080".to_string(),
            duration: Duration::from_secs(30),
            concurrency: 8,
            seed: 42,
            budget_ms: Some(150.0),
        }
    }
}

/// Result of one load run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoadReport {
    pub base_url: String,
    pub duration_s: f64,
    pub concurrency: usize,
    pub seed: u64,
    pub requests: u64,
    pub errors: u64,
    pub throughput_rps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub budget_ms: Option<f64>,
    /// true when p95 came in under the budget (or no budget was set).
    pub within_budget: bool,
}

/// Deterministic request-mix generator: tiny LCG over the six path shapes
/// the ticket workspace actually drives. Mirrors the mix the smoke
/// measures server-side so SQL-path and HTTP-path numbers stay comparable.
struct Mix {
    state: u64,
    assignee: Uuid,
    client: Uuid,
    /// Keyset cursor resolved at depth 40k (or None when the probed
    /// server pre-dates the header and the offset fallback applies).
    deep_cursor: Option<String>,
}

impl Mix {
    fn new(seed: u64, deep_cursor: Option<String>) -> Self {
        Self {
            state: seed.wrapping_mul(0x9E3779B97F4A7C15) | 1,
            assignee: md5_uuid("perf-user-1"),
            client: md5_uuid("perf-client-1"),
            deep_cursor,
        }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64* — small, deterministic, no dep.
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Percentile mix: 60% page-1 default, 10% priority p1, 10% my-work
    /// assignee scope, 10% client scope, 10% deep pagination (keyset when
    /// an anchor resolved; offset otherwise).
    fn next_path(&mut self) -> String {
        let roll = self.next_u64() % 100;
        match roll {
            0..=59 => "/api/tickets?limit=50".to_string(),
            60..=69 => "/api/tickets?limit=50&priority=p1".to_string(),
            70..=79 => format!("/api/tickets?limit=50&assignee_id={}", self.assignee),
            80..=89 => format!("/api/tickets?limit=50&client_id={}", self.client),
            _ => self.deep_path(),
        }
    }

    fn deep_path(&self) -> String {
        match &self.deep_cursor {
            // URL_SAFE_NO_PAD base64 is itself URL-safe — no extra escaping.
            Some(cursor) => format!("/api/tickets?limit=50&cursor={cursor}"),
            None => "/api/tickets?limit=50&offset=40000".to_string(),
        }
    }
}

/// One logical endpoint result from the load workers.
struct Sample {
    latency_ms: f64,
    ok: bool,
}

/// Runs the load test. Logs in once per worker, then drives requests until
/// the wall clock runs out.
pub async fn run(config: &LoadConfig) -> anyhow::Result<LoadReport> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("build http client")?;

    // Login through the real flow; every worker shares the bearer token.
    let login_url = format!("{}/api/auth/login", config.base_url.trim_end_matches('/'));
    let resp = client
        .post(&login_url)
        .json(&serde_json::json!({
            "email": PERF_USER_EMAIL,
            "password": PERF_PASSWORD,
        }))
        .send()
        .await
        .context("login request failed — is core-api running at --base-url?")?;
    anyhow::ensure!(
        resp.status().is_success(),
        "perf login failed with {} — seed the org and boot core-api first",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await?;
    // LoginResponse::Tokens variant carries access_token (MFA variant
    // cannot occur: the seeded harness user has no MFA enrollment).
    let token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .with_context(|| format!("login response missing access_token: {body}"))?
        .to_string();

    // One untimed probe resolves the keyset anchor for the deep-page leg:
    // a single-row request at depth 40k returns the server's resume
    // cursor for exactly that sort position. Pre-keyset servers don't
    // send the header — the leg silently rides the legacy offset URL.
    let deep_cursor = client
        .get(format!(
            "{}/api/tickets?limit=1&offset=40000",
            config.base_url.trim_end_matches('/')
        ))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?
        .headers()
        .get("x-next-cursor")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let deadline = Instant::now() + config.duration;
    let base_seed = config.seed;
    let deep_cursor_worker = deep_cursor.clone();
    let mut handles = Vec::with_capacity(config.concurrency);
    for worker in 0..config.concurrency {
        let client = client.clone();
        let base = config.base_url.clone();
        let token = token.clone();
        let deep_cursor = deep_cursor_worker.clone();
        handles.push(tokio::spawn(async move {
            let mut mix = Mix::new(
                base_seed ^ ((worker as u64).wrapping_mul(0xD1B54A32D192ED03)),
                deep_cursor,
            );
            let mut samples: Vec<Sample> = Vec::new();
            while Instant::now() < deadline {
                let path = mix.next_path();
                let start = Instant::now();
                let result = client
                    .get(format!("{base}{path}"))
                    .bearer_auth(&token)
                    .send()
                    .await;
                let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
                let ok = match result {
                    Ok(resp) => {
                        // Drain the body so the connection returns to the
                        // pool with predictable timing.
                        let status = resp.status();
                        let _ = resp.bytes().await;
                        status.is_success()
                    }
                    Err(_) => false,
                };
                samples.push(Sample { latency_ms, ok });
            }
            samples
        }));
    }

    let mut all = Vec::new();
    for handle in handles {
        let samples = handle.await.context("load worker panicked")?;
        all.extend(samples);
    }

    let requests = all.len() as u64;
    let errors = all.iter().filter(|s| !s.ok).count() as u64;
    let mut latencies: Vec<f64> = all.iter().map(|s| s.latency_ms).collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).expect("no NaN latencies"));

    let p = |frac: f64| -> f64 {
        if latencies.is_empty() {
            0.0
        } else {
            let idx = (frac * (latencies.len() as f64 - 1.0)).round() as usize;
            latencies[idx.min(latencies.len() - 1)]
        }
    };
    let throughput_rps = requests as f64 / config.duration.as_secs_f64();

    let report = LoadReport {
        base_url: config.base_url.clone(),
        duration_s: config.duration.as_secs_f64(),
        concurrency: config.concurrency,
        seed: config.seed,
        requests,
        errors,
        throughput_rps,
        p50_ms: p(0.50),
        p95_ms: p(0.95),
        p99_ms: p(0.99),
        max_ms: latencies.last().copied().unwrap_or(0.0),
        budget_ms: config.budget_ms,
        within_budget: config.budget_ms.map(|b| p(0.95) <= b).unwrap_or(true),
    };
    Ok(report)
}

/// The seeded org id, re-exported so the CLI help can point operators at
/// the org the harness reads from.
#[must_use]
pub fn perf_org() -> Uuid {
    PERF_ORG_ID
}
