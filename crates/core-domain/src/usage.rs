//! Org resource-usage vocabulary — the closed set of metrics the hosted
//! tier monitors and the percentage arithmetic that reports them against
//! configured limits (TASKLIST Wave 47 lane A6).
//!
//! Posture, stated once so every caller inherits it: usage is a REPORT.
//! Nothing in this module, in the storage layer, or in the API throttles,
//! rejects, or gates on a number it produces. A percentage here is a
//! measurement an operator reads, never a verdict the core acts on.
//!
//! Honesty, stated once as well: a metric is either computed exactly from
//! recorded rows or it is NOT reported. Two facts a shared database cannot
//! attribute per org (API-call volume, per-org database bytes) are absent
//! from [`UsageMetric`] by construction — there is no variant to fake them
//! into. The object-store figure counts recorded payload bytes only; its
//! gap is documented in migration 0131 alongside the view.

use serde::{Deserialize, Serialize};

/// The org-settings key prefix configured limits live under. One family,
/// one prefix: an org that has configured nothing reads back "no limits"
/// instead of a default that looks like a limit.
pub const QUOTA_SETTING_PREFIX: &str = "usage_quota.";

/// The closed set of metrics a configured limit can constrain. Each
/// variant is a REAL count the materialized usage row actually holds; the
/// list is deliberately shorter than the usage row's columns because only
/// these metrics have a meaningful "capacity" for a hosted tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum UsageMetric {
    /// Staff accounts that can still sign in.
    ActiveUsers,
    /// Every ticket in every category (open + in progress + waiting +
    /// closed) — the total an inventory limit constrains.
    Tickets,
    /// Recorded object-store payload bytes (the floor described in
    /// migration 0131).
    ObjectStoreBytes,
    /// Ledgered automation execution attempts.
    AutomationRuns,
    /// Ledgered webhook delivery attempts.
    WebhookDeliveries,
    /// Active managed client devices.
    ManagedDevices,
}

impl UsageMetric {
    /// Every metric, in wire order (the order the quota surface lists
    /// them, so a rendered table never re-shuffles between calls).
    pub const ALL: [UsageMetric; 6] = [
        UsageMetric::ActiveUsers,
        UsageMetric::Tickets,
        UsageMetric::ManagedDevices,
        UsageMetric::ObjectStoreBytes,
        UsageMetric::AutomationRuns,
        UsageMetric::WebhookDeliveries,
    ];

    /// The wire name of this metric (the same token the SDK carries).
    pub const fn as_str(self) -> &'static str {
        match self {
            UsageMetric::ActiveUsers => "active_users",
            UsageMetric::Tickets => "tickets",
            UsageMetric::ManagedDevices => "managed_devices",
            UsageMetric::ObjectStoreBytes => "object_store_bytes",
            UsageMetric::AutomationRuns => "automation_runs",
            UsageMetric::WebhookDeliveries => "webhook_deliveries",
        }
    }

    /// The org-settings key this metric's limit is read from. Every key
    /// starts with [`QUOTA_SETTING_PREFIX`]; a key outside the family is
    /// not a limit and is ignored by the reader.
    pub const fn limit_setting_key(self) -> &'static str {
        match self {
            UsageMetric::ActiveUsers => "usage_quota.active_users",
            UsageMetric::Tickets => "usage_quota.tickets",
            UsageMetric::ManagedDevices => "usage_quota.managed_devices",
            UsageMetric::ObjectStoreBytes => "usage_quota.object_store_bytes",
            UsageMetric::AutomationRuns => "usage_quota.automation_runs",
            UsageMetric::WebhookDeliveries => "usage_quota.webhook_deliveries",
        }
    }

    /// Recognizes a settings key as one of this family, returning the
    /// metric it configures. Unknown keys — including keys that merely
    /// share the prefix — resolve to `None` so a typo configures nothing
    /// instead of silently constraining a different metric.
    pub fn from_limit_setting_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.limit_setting_key() == key)
    }
}

/// `used` as a percentage of `limit`, rounded to one decimal place, or
/// `None` when there is no configured limit to measure against.
///
/// A limit of zero returns `None` rather than infinity or a panic: the
/// core does not enforce quotas, so a zero configured limit is a number an
/// operator typed, not a boundary to divide by. Callers that want to show
/// "over" can compare the raw counts.
///
/// One decimal is the contract, not an accident: percentages feed a
/// monitoring surface where 0.1% is far below any decision and a
/// full-precision float would make the same usage render differently on
/// every call.
pub fn percent_of(used: i64, limit: Option<i64>) -> Option<f64> {
    let limit = limit?;
    if limit <= 0 {
        return None;
    }
    // Round half-away-from-zero through a scaled integer so the value is
    // identical on every platform instead of depending on the FPU's
    // rounding mode for exact halves.
    let scaled = (used as f64) * 1000.0 / (limit as f64);
    let rounded = if scaled < 0.0 {
        -scale_round_half_away(-scaled)
    } else {
        scale_round_half_away(scaled)
    };
    Some(rounded / 10.0)
}

/// Half-away-from-zero rounding of an already-scaled value, kept separate
/// so the arithmetic above stays readable.
fn scale_round_half_away(scaled: f64) -> f64 {
    (scaled + 0.5).floor()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_used_over_limit_to_one_decimal() {
        assert_eq!(percent_of(3, Some(50)), Some(6.0));
        // 1/3 of the limit: 33.33…% reports 33.3, not a float tail.
        assert_eq!(percent_of(1, Some(3)), Some(33.3));
        // 2/3 crosses the rounding boundary upward.
        assert_eq!(percent_of(2, Some(3)), Some(66.7));
    }

    #[test]
    fn percent_is_stable_for_exact_halves() {
        // 1.25% is the classic half case; the contract is a deterministic
        // answer, not a platform-dependent one.
        assert_eq!(percent_of(1, Some(80)), Some(1.3));
        assert_eq!(percent_of(3, Some(80)), Some(3.8));
    }

    #[test]
    fn a_full_or_over_limit_reports_100_and_beyond() {
        assert_eq!(percent_of(50, Some(50)), Some(100.0));
        // A report, not a gate: exceeding a limit changes nothing, the
        // number simply goes past 100.
        assert_eq!(percent_of(150, Some(50)), Some(300.0));
    }

    #[test]
    fn no_limit_and_zero_limit_both_report_no_percent() {
        assert_eq!(percent_of(3, None), None);
        assert_eq!(percent_of(3, Some(0)), None);
        assert_eq!(percent_of(0, Some(-5)), None);
    }

    #[test]
    fn zero_usage_against_a_limit_is_zero_percent() {
        assert_eq!(percent_of(0, Some(10)), Some(0.0));
    }

    #[test]
    fn the_limit_key_family_is_closed_and_prefixed() {
        for metric in UsageMetric::ALL {
            let key = metric.limit_setting_key();
            assert!(key.starts_with(QUOTA_SETTING_PREFIX), "{key}");
            assert_eq!(UsageMetric::from_limit_setting_key(key), Some(metric));
        }
        // A key that shares the prefix but names no known metric is not a
        // limit — it configures nothing.
        assert_eq!(
            UsageMetric::from_limit_setting_key("usage_quota.teapots"),
            None
        );
        assert_eq!(UsageMetric::from_limit_setting_key("active_users"), None);
    }

    #[test]
    fn limit_keys_are_unique() {
        // Two metrics sharing one settings key would make a single limit
        // constrain both, silently. Pinned because it is a data-shape
        // invariant, not a behaviour anyone would notice by testing it.
        let mut keys: Vec<&str> = UsageMetric::ALL
            .into_iter()
            .map(|m| m.limit_setting_key())
            .collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate limit setting key");
    }
}
