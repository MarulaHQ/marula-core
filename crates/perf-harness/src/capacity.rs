//! Capacity model (Wave 49 lane R8) — turns a measured journey into a sizing
//! plan.
//!
//! R5's `journey` driver measures what one instance sustains over a realistic
//! session. This module carries those numbers into the two figures a sizing
//! decision needs: the users one instance holds at the measured pace, and how
//! many instances a target user count wants. The prose model, the recorded
//! anchors and the connection arithmetic live in `docs/capacity-model.md`; this
//! is the part a script can run.
//!
//! The tool-side knobs the plan reads come from the deploy descriptor (R7);
//! this module deliberately holds only the arithmetic so it stays free of
//! deployment logic.

use anyhow::Context;

/// The measured per-instance anchor: what one instance sustains at a pace.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Anchor {
    /// Offered load the instance sustained in the run (requests/second).
    pub throughput_rps: f64,
    /// Median end-to-end latency in the run (milliseconds).
    pub p50_ms: f64,
    /// The modelled pause between a user's steps (milliseconds).
    pub think_ms: u64,
}

impl Anchor {
    /// Users one instance holds: Little's law over the measured run, the same
    /// derivation `journey` reports as `users_per_instance`.
    #[must_use]
    pub fn users_per_instance(&self) -> f64 {
        self.throughput_rps * (self.think_ms as f64 + self.p50_ms) / 1000.0
    }
}

/// Per-instance Postgres connections at the default pool ceilings (Wave 49
/// lane R1): core-api app 20 + reporting 25, worker 20 + delivery 5, and one
/// LISTEN connection. `docs/perf.md` §5 carries the arithmetic, and every
/// ceiling is an env knob.
pub const DEFAULT_PER_INSTANCE_CONNECTIONS: u64 = 71;

/// A sizing plan for a target user count.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CapacityPlan {
    pub anchor: Anchor,
    pub target_users: u64,
    pub users_per_instance: f64,
    pub instances: u64,
    pub per_instance_connections: u64,
    pub total_connections: u64,
}

/// Sizes a target user count against a measured anchor. Instances are rounded
/// up, because half an instance does not exist. A zero target wants zero.
#[must_use]
pub fn plan(anchor: Anchor, target_users: u64, per_instance_connections: u64) -> CapacityPlan {
    let users_per_instance = anchor.users_per_instance();
    let instances = if target_users == 0 || users_per_instance <= 0.0 {
        0
    } else {
        ((target_users as f64) / users_per_instance).ceil() as u64
    };
    CapacityPlan {
        anchor,
        target_users,
        users_per_instance,
        instances,
        per_instance_connections,
        total_connections: instances * per_instance_connections,
    }
}

/// Reads the anchor out of a `perf-harness journey --out` report.
pub fn anchor_from_journey_json(json: &str) -> anyhow::Result<Anchor> {
    let v: serde_json::Value = serde_json::from_str(json).context("journey report is not JSON")?;
    let num = |key: &str| {
        v.get(key)
            .and_then(serde_json::Value::as_f64)
            .with_context(|| format!("journey report missing {key}"))
    };
    Ok(Anchor {
        throughput_rps: num("throughput_rps")?,
        p50_ms: num("p50_ms")?,
        think_ms: num("think_ms")? as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> Anchor {
        // The R5 clean-window knee at 50k: 695 rps, 25.9ms p50, 500ms think.
        Anchor {
            throughput_rps: 695.3,
            p50_ms: 25.9,
            think_ms: 500,
        }
    }

    #[test]
    fn users_per_instance_is_little_law() {
        let a = anchor();
        // 695.3 * (500 + 25.9) / 1000 = ~365.6 users.
        assert!((a.users_per_instance() - 365.63).abs() < 0.5);
    }

    #[test]
    fn plan_rounds_instances_up_and_totals_connections() {
        let p = plan(anchor(), 1000, DEFAULT_PER_INSTANCE_CONNECTIONS);
        // 1000 / ~365.6 = 2.73 -> 3 instances.
        assert_eq!(p.instances, 3);
        assert_eq!(p.total_connections, 3 * 71);
    }

    #[test]
    fn a_small_target_fits_one_instance() {
        let p = plan(anchor(), 250, DEFAULT_PER_INSTANCE_CONNECTIONS);
        assert_eq!(p.instances, 1);
        assert_eq!(p.total_connections, 71);
    }

    #[test]
    fn zero_target_wants_zero_instances() {
        let p = plan(anchor(), 0, DEFAULT_PER_INSTANCE_CONNECTIONS);
        assert_eq!(p.instances, 0);
        assert_eq!(p.total_connections, 0);
    }

    #[test]
    fn anchor_parses_from_a_journey_report() {
        let json = r#"{"throughput_rps":695.3,"p50_ms":25.9,"think_ms":500,"users":384}"#;
        let a = anchor_from_journey_json(json).expect("parse");
        assert!((a.throughput_rps - 695.3).abs() < 1e-9);
        assert_eq!(a.think_ms, 500);

        assert!(anchor_from_journey_json("{}").is_err());
        assert!(anchor_from_journey_json("not json").is_err());
    }
}
