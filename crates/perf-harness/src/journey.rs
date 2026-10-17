//! Multi-endpoint load journey (Wave 49 lane R5) — the third perf driver.
//!
//! `load` drives ONE endpoint (`GET /api/tickets`) with a read-only mix. That
//! answers "how fast is the board list", not "how many agents can this box
//! serve". A users-per-instance number needs a workload shaped like an
//! agent's actual session: list the board, open a ticket, read its thread,
//! change its status, leave a note — reads and writes across several
//! endpoints, paced by a think-time.
//!
//! This driver runs that session. Each virtual user (VU) shares one login
//! token (the seeded harness user is the org admin; one session per VU would
//! just re-authenticate the same principal and risk the login throttle), then
//! loops a deterministic, seeded session over the ticket workspace, pausing
//! `think_ms` between steps to model a person reading.
//!
//! It reports per-endpoint latency and, from the aggregate, the two numbers
//! the capacity model needs:
//!
//! * `req_per_user_rps` — the per-user request rate at this think-time, the
//!   "per-user peak req/s" assumption the R1 capacity note left open;
//! * `users_per_instance` — Little's law over the measured run:
//!   `throughput_rps × (think_time + p50)`, the users one instance sustains
//!   at the measured pace.
//!
//! Writes are kept legal without a workflow fixture. Each VU owns one ticket,
//! seeded into the `open` category (`md5('perf-ticket-' || i)` with `i % 20`
//! in 8..=14), and toggles it `open ↔ in_progress`, both allowed under the
//! board's semantic floor when no transition edges exist. Reads browse the
//! org at random. No VU writes another VU's ticket, so the write legs never
//! race.

use anyhow::Context;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::seed::md5_uuid;
use crate::{PERF_PASSWORD, PERF_USER_EMAIL};

/// Configuration for one journey run.
#[derive(Debug, Clone)]
pub struct JourneyConfig {
    /// Base URL of a running core-api, e.g. `http://127.0.0.1:18080`.
    pub base_url: String,
    /// How long to drive load.
    pub duration: Duration,
    /// Virtual users: concurrent sessions, each pacing itself by think-time.
    pub users: usize,
    /// Modeled pause between a user's steps (milliseconds).
    pub think_ms: u64,
    /// Seed for the deterministic session mix.
    pub seed: u64,
    /// Seeded ticket count, so random reads pick ids that exist.
    pub tickets: i64,
    /// Fail the run when aggregate p95 exceeds this budget (milliseconds).
    pub budget_ms: Option<f64>,
    /// Free label carried into the report (e.g. `50k`)
    pub label: Option<String>,
}

impl Default for JourneyConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:18080".to_string(),
            duration: Duration::from_secs(30),
            users: 16,
            think_ms: 500,
            seed: 42,
            tickets: 50_000,
            budget_ms: Some(150.0),
            label: None,
        }
    }
}

/// One session step, in the order an agent might reach for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    ListBoard,
    MyWork,
    OpenTicket,
    ReadThread,
    SlaStatus,
    AddComment,
    ChangeStatus,
}

impl Step {
    /// Stable name used as the report key.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ListBoard => "list_board",
            Self::MyWork => "my_work",
            Self::OpenTicket => "open_ticket",
            Self::ReadThread => "read_thread",
            Self::SlaStatus => "sla_status",
            Self::AddComment => "add_comment",
            Self::ChangeStatus => "change_status",
        }
    }

    /// Whether this step mutates state. Writes are the legs the single-writer
    /// path (§4.1) actually pays for, so the mix keeps them a known share.
    #[must_use]
    pub const fn is_write(self) -> bool {
        matches!(self, Self::AddComment | Self::ChangeStatus)
    }
}

/// Percentile mix, weighted to the reads a board browser issues most. Writes
/// are ~20%: a real agent edits, but reads dominate. The split is a modelling
/// choice, stated here so the number is reproducible and arguable.
const MIX: &[(Step, u64)] = &[
    (Step::ListBoard, 30),
    (Step::OpenTicket, 20),
    (Step::ReadThread, 15),
    (Step::MyWork, 10),
    (Step::SlaStatus, 5),
    (Step::AddComment, 12),
    (Step::ChangeStatus, 8),
];

/// Per-endpoint latency and error totals.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EndpointStat {
    pub endpoint: &'static str,
    pub requests: u64,
    pub errors: u64,
    /// Failed-response status counts (0 = transport error, no response).
    pub error_codes: std::collections::BTreeMap<u16, u64>,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// Result of one journey run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JourneyReport {
    pub base_url: String,
    pub label: Option<String>,
    pub tickets: i64,
    pub users: usize,
    pub think_ms: u64,
    pub seed: u64,
    pub duration_s: f64,
    pub requests: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub throughput_rps: f64,
    /// Requests per second per concurrent user at this think-time — the
    /// per-user rate the capacity model multiplies against instance count.
    pub req_per_user_rps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub budget_ms: Option<f64>,
    pub within_budget: bool,
    /// Little's law: users this instance sustains at the measured pace.
    pub users_per_instance: f64,
    pub endpoints: Vec<EndpointStat>,
}

/// Little's law form used for `users_per_instance`: the number of users a
/// box serves is its throughput times the time each user occupies it
/// (think-time plus service time). Pure so the derivation is unit-tested.
#[must_use]
pub fn users_per_instance(throughput_rps: f64, think_ms: u64, p50_ms: f64) -> f64 {
    throughput_rps * (think_ms as f64 + p50_ms) / 1000.0
}

/// The seeded harness user's inferred ids (mirrors `seed.rs`).
fn ticket_id(index: i64) -> Uuid {
    md5_uuid(&format!("perf-ticket-{index}"))
}

fn status_id(category: &str) -> Uuid {
    md5_uuid(&format!("perf-status-{category}"))
}

/// One virtual user's deterministic session state.
struct Vu {
    state: u64,
    /// The VU's own ticket. Seeded `open`, but its status persists between
    /// runs, so the initial belief is read from the server (see `run`).
    own_ticket: Uuid,
    /// The VU's belief about `own_ticket`'s category (false = open).
    own_in_progress: bool,
}

impl Vu {
    /// The `open`-category ticket (i % 20 == 8) this VU owns. One per VU, so
    /// no two VUs write the same ticket.
    fn own_ticket_index(index: usize, tickets: i64) -> Uuid {
        ticket_id((20 * index as i64 + 8).min(tickets.max(1)))
    }

    fn new(seed: u64, own_ticket: Uuid, initially_in_progress: bool) -> Self {
        Self {
            state: seed.wrapping_mul(0x9E3779B97F4A7C15) | 1,
            own_ticket,
            own_in_progress: initially_in_progress,
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn next_step(&mut self) -> Step {
        let roll = self.next_u64() % 100;
        let mut acc = 0;
        for (step, weight) in MIX {
            acc += weight;
            if roll < acc {
                return *step;
            }
        }
        Step::ListBoard
    }

    fn random_ticket(&mut self, tickets: i64) -> Uuid {
        let span = tickets.max(1) as u64;
        ticket_id(1 + (self.next_u64() % span) as i64)
    }
}

/// A measured request, tagged with the step it came from.
struct Sample {
    step: Step,
    latency_ms: f64,
    ok: bool,
    /// HTTP status, or 0 when the request never produced a response.
    status: u16,
}

/// Runs the journey. Logs in once (shared token), then drives every VU until
/// the wall clock runs out.
pub async fn run(config: &JourneyConfig) -> anyhow::Result<JourneyReport> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("build http client")?;

    let base = config.base_url.trim_end_matches('/').to_string();

    // One login for the whole run: all VUs are the same seeded admin
    // principal, so per-VU logins would just re-authenticate one account and
    // push against the login throttle. Not measured — it is setup.
    let resp = client
        .post(format!("{base}/api/auth/login"))
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
    let token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .with_context(|| format!("login response missing access_token: {body}"))?
        .to_string();

    let deadline = Instant::now() + config.duration;
    let think = Duration::from_millis(config.think_ms);
    let think_ms = config.think_ms;
    let tickets = config.tickets;

    // Own ticket per VU, plus its CURRENT status. The seeded ticket starts in
    // the `open` category, but the org is not reseeded between runs, so a
    // second run would otherwise assume `open` against a ticket the first run
    // left `in_progress`, and its first change would self-transition (422).
    // One untimed read per VU at setup keeps every change legal without a
    // reseed.
    let own_tickets: Vec<Uuid> = (0..config.users)
        .map(|index| Vu::own_ticket_index(index, tickets))
        .collect();
    let mut initial_in_progress = Vec::with_capacity(config.users);
    for id in &own_tickets {
        let mut in_progress = false;
        if let Ok(resp) = client
            .get(format!("{base}/api/tickets/{id}"))
            .bearer_auth(&token)
            .send()
            .await
        {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                in_progress = v.get("category").and_then(|c| c.as_str()) == Some("in_progress");
            }
        }
        initial_in_progress.push(in_progress);
    }

    let mut handles = Vec::with_capacity(config.users);
    for (index, own_ticket) in own_tickets.into_iter().enumerate() {
        let client = client.clone();
        let base = base.clone();
        let token = token.clone();
        let mut vu = Vu::new(
            config.seed ^ ((index as u64).wrapping_mul(0xD1B54A32D192ED03)),
            own_ticket,
            initial_in_progress[index],
        );
        handles.push(tokio::spawn(async move {
            let mut samples: Vec<Sample> = Vec::new();
            while Instant::now() < deadline {
                let step = vu.next_step();
                // The status toggle's target, decided before the request but
                // applied to the VU's belief only if the request succeeds.
                let status_target = if step == Step::ChangeStatus {
                    Some(if vu.own_in_progress {
                        "open"
                    } else {
                        "in_progress"
                    })
                } else {
                    None
                };
                let req = match step {
                    Step::ListBoard => client.get(format!("{base}/api/tickets?limit=50")),
                    Step::MyWork => client.get(format!("{base}/api/tickets/my-work")),
                    Step::OpenTicket => {
                        let id = vu.random_ticket(tickets);
                        client.get(format!("{base}/api/tickets/{id}"))
                    }
                    Step::ReadThread => {
                        let id = vu.random_ticket(tickets);
                        client.get(format!("{base}/api/tickets/{id}/comments"))
                    }
                    Step::SlaStatus => {
                        let id = vu.random_ticket(tickets);
                        client.get(format!("{base}/api/tickets/{id}/sla"))
                    }
                    Step::AddComment => client
                        .post(format!("{base}/api/tickets/{}/comments", vu.own_ticket))
                        .json(&serde_json::json!({
                            "body": "perf journey note",
                            "kind": "internal",
                        })),
                    Step::ChangeStatus => {
                        // Toggle open <-> in_progress: both directions are
                        // legal under the semantic floor, and only this VU
                        // writes this ticket, so the target stays valid.
                        let target = status_target.expect("status target set for change_status");
                        client
                            .post(format!("{base}/api/tickets/{}/status", vu.own_ticket))
                            .json(&serde_json::json!({ "status_id": status_id(target) }))
                    }
                };
                let start = Instant::now();
                let result = req.bearer_auth(&token).send().await;
                let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
                let (ok, status) = match result {
                    Ok(resp) => {
                        // Drain the body so the connection returns to the
                        // pool with predictable timing.
                        let status = resp.status();
                        let code = status.as_u16();
                        let _ = resp.bytes().await;
                        (status.is_success(), code)
                    }
                    Err(_) => (false, 0),
                };
                samples.push(Sample {
                    step,
                    latency_ms,
                    ok,
                    status,
                });
                // Only advance the belief on success: a failed change left the
                // server as it was, so flipping the belief would desync the
                // next target and cascade into self-transitions.
                if step == Step::ChangeStatus && ok {
                    vu.own_in_progress = !vu.own_in_progress;
                }
                if think_ms > 0 {
                    tokio::time::sleep(think).await;
                }
            }
            samples
        }));
    }

    let mut all = Vec::new();
    for handle in handles {
        all.extend(handle.await.context("journey worker panicked")?);
    }

    Ok(build_report(config, &base, all))
}

/// Turns the raw samples into the report. Split out so the aggregation (and
/// the Little's-law number) is testable without a running server.
fn build_report(config: &JourneyConfig, base: &str, all: Vec<Sample>) -> JourneyReport {
    let requests = all.len() as u64;
    let errors = all.iter().filter(|s| !s.ok).count() as u64;
    let duration_s = config.duration.as_secs_f64();
    let throughput_rps = requests as f64 / duration_s;

    let mut latencies: Vec<f64> = all.iter().map(|s| s.latency_ms).collect();
    let (p50_ms, p95_ms, p99_ms, max_ms) = percentiles(&mut latencies);

    let mut endpoints: Vec<EndpointStat> = Vec::new();
    for step in [
        Step::ListBoard,
        Step::OpenTicket,
        Step::ReadThread,
        Step::MyWork,
        Step::SlaStatus,
        Step::AddComment,
        Step::ChangeStatus,
    ] {
        let mut lat: Vec<f64> = all
            .iter()
            .filter(|s| s.step == step)
            .map(|s| s.latency_ms)
            .collect();
        if lat.is_empty() {
            continue;
        }
        let errs = all.iter().filter(|s| s.step == step && !s.ok).count() as u64;
        let mut error_codes: std::collections::BTreeMap<u16, u64> =
            std::collections::BTreeMap::new();
        for s in all.iter().filter(|s| s.step == step && !s.ok) {
            *error_codes.entry(s.status).or_default() += 1;
        }
        let (e50, e95, e99, emax) = percentiles(&mut lat);
        endpoints.push(EndpointStat {
            endpoint: step.name(),
            requests: lat.len() as u64,
            errors: errs,
            error_codes,
            p50_ms: e50,
            p95_ms: e95,
            p99_ms: e99,
            max_ms: emax,
        });
    }

    JourneyReport {
        base_url: base.to_string(),
        label: config.label.clone(),
        tickets: config.tickets,
        users: config.users,
        think_ms: config.think_ms,
        seed: config.seed,
        duration_s,
        requests,
        errors,
        error_rate: if requests == 0 {
            0.0
        } else {
            errors as f64 / requests as f64
        },
        throughput_rps,
        req_per_user_rps: if config.users == 0 {
            0.0
        } else {
            throughput_rps / config.users as f64
        },
        p50_ms,
        p95_ms,
        p99_ms,
        max_ms,
        budget_ms: config.budget_ms,
        within_budget: config.budget_ms.map(|b| p95_ms <= b).unwrap_or(true),
        users_per_instance: users_per_instance(throughput_rps, config.think_ms, p50_ms),
        endpoints,
    }
}

/// Sorts in place and returns `(p50, p95, p99, max)`.
fn percentiles(xs: &mut [f64]) -> (f64, f64, f64, f64) {
    if xs.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    xs.sort_by(|a, b| a.partial_cmp(b).expect("no NaN latencies"));
    let p = |frac: f64| -> f64 {
        let idx = (frac * (xs.len() as f64 - 1.0)).round() as usize;
        xs[idx.min(xs.len() - 1)]
    };
    (p(0.50), p(0.95), p(0.99), *xs.last().expect("non-empty"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_covers_every_step_and_keeps_writes_a_minority() {
        let mut vu = Vu::new(42, Vu::own_ticket_index(0, 50_000), false);
        let mut counts: std::collections::HashMap<Step, u64> = std::collections::HashMap::new();
        let n = 10_000;
        for _ in 0..n {
            *counts.entry(vu.next_step()).or_default() += 1;
        }
        for (step, _) in MIX {
            assert!(
                counts.get(step).copied().unwrap_or(0) > 0,
                "step {} never chosen",
                step.name()
            );
        }
        let writes: u64 = counts
            .iter()
            .filter(|(s, _)| s.is_write())
            .map(|(_, c)| *c)
            .sum();
        let share = writes as f64 / n as f64;
        assert!(
            (0.10..0.30).contains(&share),
            "writes should be a minority (~20%), got {share}"
        );
    }

    #[test]
    fn own_ticket_is_seeded_open_and_unique_per_vu() {
        // Category `open` is `i % 20` in 8..=14; the own-ticket formula must
        // land there, and two VUs must not share one ticket.
        let mut seen = std::collections::HashSet::new();
        for vu in 0..64 {
            let index = 20 * vu + 8;
            assert!(
                (8..=14).contains(&(index % 20)),
                "own ticket {index} is not open-category"
            );
            assert!(seen.insert(index), "ticket {index} handed to two VUs");
        }
    }

    #[test]
    fn own_ticket_ids_match_the_seed_derivation() {
        // The id must equal what `seed.rs` inserts, or reads 404.
        assert_eq!(ticket_id(1), md5_uuid("perf-ticket-1"));
        assert_eq!(status_id("open"), md5_uuid("perf-status-open"));
        assert_eq!(
            status_id("in_progress"),
            md5_uuid("perf-status-in_progress")
        );
    }

    #[test]
    fn little_users_is_throughput_times_occupancy() {
        // 100 rps, 500ms think, 20ms service: each user holds the box 0.52s,
        // so the box serves ~52 users.
        let users = users_per_instance(100.0, 500, 20.0);
        assert!((users - 52.0).abs() < 1e-9, "got {users}");
    }

    #[test]
    fn build_report_derives_per_endpoint_and_aggregate() {
        let config = JourneyConfig {
            duration: Duration::from_secs(10),
            users: 4,
            think_ms: 300,
            tickets: 50_000,
            budget_ms: Some(350.0),
            ..JourneyConfig::default()
        };
        let mk = |step, latency_ms, ok| Sample {
            step,
            latency_ms,
            ok,
            status: if ok { 200 } else { 409 },
        };
        let samples = vec![
            mk(Step::ListBoard, 10.0, true),
            mk(Step::ListBoard, 20.0, true),
            mk(Step::ListBoard, 300.0, true),
            mk(Step::AddComment, 40.0, true),
            mk(Step::ChangeStatus, 25.0, false),
            mk(Step::OpenTicket, 15.0, true),
        ];
        let report = build_report(&config, "http://x", samples);
        assert_eq!(report.requests, 6);
        assert_eq!(report.errors, 1);
        assert!((report.error_rate - 1.0 / 6.0).abs() < 1e-9);
        assert!((report.throughput_rps - 0.6).abs() < 1e-9);
        assert!((report.req_per_user_rps - 0.15).abs() < 1e-9);
        // Six samples put the p95 index on the top value (300ms).
        assert!(
            (report.p95_ms - 300.0).abs() < 1e-9,
            "got {}",
            report.p95_ms
        );
        assert!(report.within_budget, "300ms is inside the 350ms budget");
        // Per-endpoint rows only for steps that occurred.
        assert_eq!(report.endpoints.len(), 4);
        let list = report
            .endpoints
            .iter()
            .find(|e| e.endpoint == "list_board")
            .expect("list row");
        assert_eq!(list.requests, 3);
        assert!((list.p50_ms - 20.0).abs() < 1e-9);
        // The one failed request was a change_status 409, captured by code.
        let status_row = report
            .endpoints
            .iter()
            .find(|e| e.endpoint == "change_status")
            .expect("status row");
        assert_eq!(status_row.error_codes.get(&409), Some(&1));
    }

    #[test]
    fn build_report_flags_budget_breach() {
        let config = JourneyConfig {
            duration: Duration::from_secs(10),
            budget_ms: Some(50.0),
            ..JourneyConfig::default()
        };
        let samples: Vec<Sample> = (0..20)
            .map(|_| Sample {
                step: Step::ListBoard,
                latency_ms: 200.0,
                ok: true,
                status: 200,
            })
            .collect();
        let report = build_report(&config, "http://x", samples);
        assert!(!report.within_budget);
    }
}
