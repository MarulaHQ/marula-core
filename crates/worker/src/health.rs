//! The worker's health surface: `/livez`, `/readyz`, and the legacy `/healthz`.
//!
//! Purpose: give an orchestrator the two questions it actually asks, and answer
//! them with different answers.
//!
//! - `/livez` — "is this process wedged?" It reads nothing but its own ability
//!   to answer, so it stays 200 while the process is inside its shutdown
//!   window. A drain that fails liveness would restart the pod it is trying to
//!   retire.
//! - `/readyz` — "should this process be kept?" It is 503 while draining and
//!   while any loop this process owns has died.
//! - `/healthz` — the path that shipped first, and the one the compose stack,
//!   the demo script and the core-api self-host pane already probe
//!   (`core-api/src/routes/self_host.rs` requires a 2xx AND `status: "ok"`, so
//!   it reads a 503 exactly as it should: down). It keeps the per-loop body and
//!   now answers 503 when a loop it owns has died — before this, the body said
//!   `unhealthy` and the status said `200`, so nothing acted on it.
//!
//! A loop that dies is a restart-worthy fault for the worker (it owns no
//! traffic, so draining readiness alone would leave the pod running with a dead
//! consumer), and that is why the chart wires liveness to `/healthz` rather than
//! to `/livez`. `/healthz` stays 200 while draining: the loops exiting there is
//! the plan, not a fault.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::config::WorkerRoles;

/// Loops that report no liveness flag, so the drain report cannot name them.
/// They are listed in the shutdown log instead of being omitted from it.
pub const UNTRACKED_LOOPS: [&str; 6] = [
    "wakeup",
    "notifier",
    "digest",
    "search_indexer",
    "dedupe",
    "decision",
];

/// Every loop's liveness flag plus the roles this process owns.
#[derive(Clone)]
pub struct HealthFlags {
    pub relay_alive: Arc<AtomicBool>,
    pub email_alive: Arc<AtomicBool>,
    pub time_alive: Arc<AtomicBool>,
    pub queue_alive: Arc<AtomicBool>,
    pub sla_alive: Arc<AtomicBool>,
    pub automations_alive: Arc<AtomicBool>,
    pub webhooks_alive: Arc<AtomicBool>,
    pub delivery_alive: Arc<AtomicBool>,
    pub pm_alive: Arc<AtomicBool>,
    pub rotation_alive: Arc<AtomicBool>,
    pub retention_alive: Arc<AtomicBool>,
    pub outbox_alive: Arc<AtomicBool>,
    pub usage_alive: Arc<AtomicBool>,
    pub workflow_alive: Arc<AtomicBool>,
    /// Which groups this process runs. A loop it does not own is reported as
    /// `disabled` and does not affect any status: a realtime-only replica is
    /// healthy even though it runs no sweeps.
    pub roles: WorkerRoles,
    /// The shared stop flag. `true` means the process is inside its shutdown
    /// window: readiness is off, liveness stays on.
    pub draining: watch::Receiver<bool>,
}

impl HealthFlags {
    /// (name, alive, owned here) for every loop this struct tracks.
    pub fn loops(&self) -> [(&'static str, bool, bool); 14] {
        let realtime = self.roles.realtime();
        let housekeeping = self.roles.housekeeping();
        [
            ("relay", self.relay_alive.load(Ordering::Relaxed), realtime),
            ("email", self.email_alive.load(Ordering::Relaxed), realtime),
            ("queue", self.queue_alive.load(Ordering::Relaxed), realtime),
            (
                "automations",
                self.automations_alive.load(Ordering::Relaxed),
                realtime,
            ),
            (
                "webhooks",
                self.webhooks_alive.load(Ordering::Relaxed),
                realtime,
            ),
            (
                "workflows",
                self.workflow_alive.load(Ordering::Relaxed),
                realtime,
            ),
            ("sla", self.sla_alive.load(Ordering::Relaxed), housekeeping),
            (
                "delivery",
                self.delivery_alive.load(Ordering::Relaxed),
                housekeeping,
            ),
            ("pm", self.pm_alive.load(Ordering::Relaxed), housekeeping),
            (
                "rotation",
                self.rotation_alive.load(Ordering::Relaxed),
                housekeeping,
            ),
            (
                "retention",
                self.retention_alive.load(Ordering::Relaxed),
                housekeeping,
            ),
            (
                "outbox",
                self.outbox_alive.load(Ordering::Relaxed),
                housekeeping,
            ),
            (
                "usage",
                self.usage_alive.load(Ordering::Relaxed),
                housekeeping,
            ),
            (
                "time",
                self.time_alive.load(Ordering::Relaxed),
                housekeeping,
            ),
        ]
    }

    /// Only the loops this process owns decide health.
    pub fn healthy(&self) -> bool {
        self.loops()
            .iter()
            .all(|(_, alive, owned)| !owned || *alive)
    }

    pub fn draining(&self) -> bool {
        *self.draining.borrow()
    }

    /// The loops this process owns, by name: the work a drain has to wait for.
    pub fn owned_loops(&self) -> Vec<&'static str> {
        self.loops()
            .iter()
            .filter(|(_, _, owned)| *owned)
            .map(|(name, _, _)| *name)
            .collect()
    }

    /// Owned loops still running — what a drain reports as cut if they are
    /// still here when the window closes.
    pub fn still_running(&self) -> Vec<&'static str> {
        self.loops()
            .iter()
            .filter(|(_, alive, owned)| *owned && *alive)
            .map(|(name, _, _)| *name)
            .collect()
    }
}

/// `GET /livez` — the process is answering; nothing else is asked.
pub async fn livez() -> StatusCode {
    StatusCode::OK
}

/// `GET /readyz` — 200 only when not draining and every owned loop is alive.
pub async fn readyz(State(flags): State<Arc<HealthFlags>>) -> (StatusCode, Json<Value>) {
    let draining = flags.draining();
    let down: Vec<&'static str> = flags
        .loops()
        .iter()
        .filter(|(_, alive, owned)| *owned && !alive)
        .map(|(name, _, _)| *name)
        .collect();
    let ready = !draining && down.is_empty();
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status": if ready { "ready" } else { "not_ready" },
            "draining": draining,
            "down": down,
            "roles": flags.roles.label(),
        })),
    )
}

/// `GET /healthz` — the legacy path, same body shape, honest status code.
pub async fn healthz(State(flags): State<Arc<HealthFlags>>) -> (StatusCode, Json<Value>) {
    let roles = flags.roles;
    let draining = flags.draining();
    let loops = flags.loops();
    // While draining, loops exiting is the plan: their flags go false, and
    // letting that answer 503 would have a liveness probe restart the pod the
    // drain is retiring. `draining: true` in the body carries the reason.
    let healthy = draining || flags.healthy();
    let mut body = serde_json::Map::new();
    body.insert(
        "status".into(),
        json!(if healthy { "ok" } else { "unhealthy" }),
    );
    body.insert("glitchtip".into(), json!(dsn_state("GLITCHTIP_DSN_WORKER")));
    body.insert("roles".into(), json!(roles.label()));
    body.insert("draining".into(), json!(draining));
    for (name, alive, owned) in loops {
        body.insert(
            name.to_string(),
            json!(if !owned {
                "disabled"
            } else if alive {
                "alive"
            } else {
                "down"
            }),
        );
    }
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(Value::Object(body)))
}

/// `configured` | `unset` — GlitchTip DSN presence for the self-host settings
/// pane (Wave 36 lane 3); the DSN value never leaves env.
fn dsn_state(name: &str) -> &'static str {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => "configured",
        _ => "unset",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(roles: WorkerRoles) -> HealthFlags {
        let dead = |v: bool| Arc::new(AtomicBool::new(v));
        HealthFlags {
            relay_alive: dead(true),
            email_alive: dead(true),
            time_alive: dead(true),
            queue_alive: dead(true),
            sla_alive: dead(true),
            automations_alive: dead(true),
            webhooks_alive: dead(true),
            delivery_alive: dead(true),
            pm_alive: dead(true),
            rotation_alive: dead(true),
            retention_alive: dead(true),
            outbox_alive: dead(true),
            usage_alive: dead(true),
            workflow_alive: dead(true),
            roles,
            draining: watch::channel(false).1,
        }
    }

    #[tokio::test]
    async fn healthy_replica_answers_200_on_every_path() {
        let flags = Arc::new(flags(WorkerRoles::all()));
        let (status, body) = healthz(State(Arc::clone(&flags))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["status"], "ok");
        assert_eq!(body.0["relay"], "alive");
        assert_eq!(body.0["email"], "alive");
        assert_eq!(body.0["webhooks"], "alive");
        assert_eq!(body.0["delivery"], "alive");
        assert_eq!(body.0["outbox"], "alive");
        assert_eq!(body.0["roles"], "all");
        assert_eq!(body.0["draining"], false);

        let (status, body) = readyz(State(Arc::clone(&flags))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["status"], "ready");
        assert_eq!(livez().await, StatusCode::OK);
    }

    /// The finding this replaces: the body said `unhealthy` and the status said
    /// 200, so no orchestrator ever acted on it.
    #[tokio::test]
    async fn a_dead_loop_is_503_on_health_and_readiness_but_still_live() {
        let flags = Arc::new(flags(WorkerRoles::all()));
        flags.queue_alive.store(false, Ordering::Relaxed);

        let (status, body) = healthz(State(Arc::clone(&flags))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["status"], "unhealthy");
        assert_eq!(body.0["queue"], "down");
        assert_eq!(body.0["relay"], "alive");

        let (status, body) = readyz(State(Arc::clone(&flags))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["status"], "not_ready");
        assert_eq!(body.0["down"], json!(["queue"]));

        // Liveness is about the process, not the loops: a wedged process is
        // restarted by its probe timing out, not by a loop flag.
        assert_eq!(livez().await, StatusCode::OK);
    }

    /// A draining process is being retired on purpose: readiness off, liveness
    /// on, and the legacy health path quiet so a liveness probe on it cannot
    /// fight the drain.
    #[tokio::test]
    async fn draining_is_not_ready_but_still_live() {
        let (sender, receiver) = watch::channel(false);
        let mut flags = flags(WorkerRoles::all());
        flags.draining = receiver;
        let flags = Arc::new(flags);
        sender.send(true).expect("signal");

        let (status, body) = readyz(State(Arc::clone(&flags))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["draining"], true);

        let (status, body) = healthz(State(Arc::clone(&flags))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["status"], "ok");
        assert_eq!(body.0["draining"], true);

        assert_eq!(livez().await, StatusCode::OK);
    }

    /// A role-limited replica is healthy for the loops it owns and reports the
    /// rest as `disabled`, so an orchestrator does not read a realtime-only
    /// worker as eight dead sweeps.
    #[tokio::test]
    async fn loops_this_process_does_not_own_cannot_take_it_down() {
        let flags = Arc::new(flags(WorkerRoles::realtime_only()));
        let (status, body) = healthz(State(Arc::clone(&flags))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["status"], "ok");
        assert_eq!(body.0["roles"], "realtime");
        assert_eq!(body.0["relay"], "alive");
        assert_eq!(body.0["webhooks"], "alive");
        assert_eq!(body.0["sla"], "disabled");
        assert_eq!(body.0["outbox"], "disabled");
        assert_eq!(body.0["time"], "disabled");
        assert_eq!(flags.owned_loops().len(), 6);

        // A sweep this process does not run cannot take it down; the loops it
        // does own still can.
        let mut housekeeping = flags.as_ref().clone();
        housekeeping.roles = WorkerRoles::housekeeping_only();
        let housekeeping = Arc::new(housekeeping);
        let (status, body) = healthz(State(Arc::clone(&housekeeping))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.0["relay"], "disabled");
        housekeeping.rotation_alive.store(false, Ordering::Relaxed);
        let (status, _) = healthz(State(Arc::clone(&housekeeping))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_drain_waits_only_on_the_loops_this_process_owns() {
        let flags = Arc::new(flags(WorkerRoles::realtime_only()));
        // Loops it does not own may be "alive" or not; they are not work this
        // process owes an answer for.
        assert!(!flags.still_running().contains(&"sla"));
        assert!(flags.still_running().contains(&"relay"));
        assert_eq!(flags.still_running().len(), 6);
    }
}
