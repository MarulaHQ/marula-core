//! The retry policy — one place that answers "how long, how often, and is it
//! worth it".
//!
//! Purpose: every loop in the product that waits before trying again reads its
//! base, factor, cap, attempt budget, jitter and error classification from
//! here. Before this module each subsystem carried its own `const`: the job
//! queue computed `5·2^n` clamped at 1280 s under a docstring that claimed an
//! hour, email and webhooks each doubled to a cap of their own, and the stream
//! consumers retried forever with a fixed 5 s ceiling. A budget that lives in
//! five places is five budgets that disagree.
//!
//! Non-purpose: no sleeping, no clocks, no environment. `delay` is a pure
//! function of the attempt number and a caller-supplied sample, so a test can
//! pin every value and the operational code can still jitter. Reading the
//! environment is the binaries' job (`RetryMatrix::from_lookup` takes the
//! lookup); this module only owns the arithmetic and the defaults.
//!
//! ## The two decisions that live here
//!
//! **A budget is attempts, not elapsed time.** `AttemptBudget::Limited(n)` is
//! the number of tries a unit of work gets in total, and delays are derived
//! from how many have already failed. Wall-clock budgets drift with load; an
//! attempt count is the same on a busy box and an idle one.
//!
//! **A 4xx is not automatically permanent.** 408 and 429 mean "ask again";
//! 5xx and transport failures mean "ask again"; most other 4xx mean "this
//! request is wrong" and never retry. 409 is the ambiguous one — it means a
//! concurrent writer won, or the row moved under us, which is worth one more
//! attempt exactly when replaying the request cannot double-apply. That is a
//! property of the operation, so each policy states its posture instead of the
//! code guessing. The effects seam opts into retry because every endpoint it
//! posts to is replay-inert (see `Subsystem::Effects`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// Which retry loop is asking. Seven subsystems, because the escape hatches
/// have to be separable: an operator whose webhook receivers are rate limiting
/// them should be able to widen that budget without also widening the queue's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subsystem {
    /// The general job queue (`jobs` table). Durable, at-least-once, claimed
    /// by `SKIP LOCKED`; the queue owns durability for effects the worker
    /// posts, which is why `Effects` is single-attempt.
    Queue,
    /// Outbound email claimed from `email_outbound`.
    Email,
    /// Signed webhook delivery POSTed to a third party.
    Webhook,
    /// The worker → core-api machine seam. One attempt per invocation by
    /// design: the job that carried the effect retries under `Queue`.
    Effects,
    /// Redis stream consumer reconnect (a lost connection, not a failed job).
    Consumer,
    /// Outbox relay and realtime listener reconnect.
    Relay,
    /// Upstream model provider retries inside the decision provider.
    Decision,
}

impl Subsystem {
    /// Every subsystem, in declaration order. The descriptor and the
    /// reliability sweep both walk this.
    pub const ALL: [Self; 7] = [
        Self::Queue,
        Self::Email,
        Self::Webhook,
        Self::Effects,
        Self::Consumer,
        Self::Relay,
        Self::Decision,
    ];

    /// The env-group token: `queue`, `email`, `webhook`, `effects`,
    /// `consumer`, `relay`, `decision`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Email => "email",
            Self::Webhook => "webhook",
            Self::Effects => "effects",
            Self::Consumer => "consumer",
            Self::Relay => "relay",
            Self::Decision => "decision",
        }
    }

    /// Parses a group token; case-insensitive so an operator's `QUEUE` works.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let lower = value.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|s| s.as_str() == lower)
    }
}

/// The product decision: one name that sets every budget at once.
///
/// `balanced` is what ships. `durable` buys more attempts and the full cap for
/// installs that would rather retry for a day than page someone. `best_effort`
/// gives up sooner for latency-sensitive or cost-sensitive deployments.
/// `single_box` is the one-container shape: short caps because there is no
/// second replica to absorb a long retry window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobustnessPreset {
    Balanced,
    Durable,
    BestEffort,
    SingleBox,
}

impl RobustnessPreset {
    pub const ALL: [Self; 4] = [
        Self::Balanced,
        Self::Durable,
        Self::BestEffort,
        Self::SingleBox,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Durable => "durable",
            Self::BestEffort => "best_effort",
            Self::SingleBox => "single_box",
        }
    }

    /// Parses a preset name. Accepts `best-effort` as well as `best_effort`
    /// because both spellings are already in operator hands.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let lower = value.trim().to_ascii_lowercase().replace('-', "_");
        Self::ALL.into_iter().find(|p| p.as_str() == lower)
    }

    /// The shipped policy for one subsystem under this preset.
    #[must_use]
    pub const fn policy(self, subsystem: Subsystem) -> RetryPolicy {
        // One `match` per preset rather than a table keyed by
        // (preset, subsystem): the numbers for one preset read as a story, and
        // adding a subsystem fails to compile until every preset answers.
        match self {
            Self::Balanced => match subsystem {
                // 5 s doubling, a real hour cap, ten tries — ~2 hours of
                // coverage before the job is visible as dead.
                Subsystem::Queue => RetryPolicy::new(
                    Subsystem::Queue,
                    Duration::from_secs(5),
                    2,
                    Duration::from_secs(3_600),
                    AttemptBudget::Limited(10),
                    Jitter::Full,
                ),
                // 30 s doubling to an hour, six attempts (the historical
                // email ledger budget, now stated once).
                Subsystem::Email => RetryPolicy::new(
                    Subsystem::Email,
                    Duration::from_secs(30),
                    2,
                    Duration::from_secs(3_600),
                    AttemptBudget::Limited(6),
                    Jitter::Equal,
                ),
                // A receiver that is down should be given up on within a few
                // hours: six attempts, 30 s to an hour.
                Subsystem::Webhook => RetryPolicy::new(
                    Subsystem::Webhook,
                    Duration::from_secs(30),
                    2,
                    Duration::from_secs(3_600),
                    AttemptBudget::Limited(6),
                    Jitter::Full,
                ),
                // Classification only: one attempt, and the class decides
                // whether the carrying job retries.
                Subsystem::Effects => RetryPolicy::new(
                    Subsystem::Effects,
                    Duration::from_millis(100),
                    2,
                    Duration::from_millis(100),
                    AttemptBudget::Limited(1),
                    Jitter::None,
                )
                .on_conflict(ConflictPosture::Retry),
                Subsystem::Consumer => RetryPolicy::new(
                    Subsystem::Consumer,
                    Duration::from_millis(250),
                    2,
                    Duration::from_secs(10),
                    AttemptBudget::Unlimited,
                    Jitter::Full,
                ),
                Subsystem::Relay => RetryPolicy::new(
                    Subsystem::Relay,
                    Duration::from_millis(250),
                    2,
                    Duration::from_secs(10),
                    AttemptBudget::Unlimited,
                    Jitter::Full,
                ),
                // Three tries, 200 ms doubling: a provider that is rate
                // limiting answers or refuses, and a caller waiting longer
                // than this is better off failing honestly.
                Subsystem::Decision => RetryPolicy::new(
                    Subsystem::Decision,
                    Duration::from_millis(200),
                    2,
                    Duration::from_secs(2),
                    AttemptBudget::Limited(3),
                    Jitter::Full,
                ),
            },
            Self::Durable => match subsystem {
                Subsystem::Queue => RetryPolicy::new(
                    Subsystem::Queue,
                    Duration::from_secs(5),
                    2,
                    Duration::from_secs(3_600),
                    AttemptBudget::Limited(14),
                    Jitter::Full,
                ),
                Subsystem::Email => RetryPolicy::new(
                    Subsystem::Email,
                    Duration::from_secs(30),
                    2,
                    Duration::from_secs(3_600),
                    AttemptBudget::Limited(10),
                    Jitter::Full,
                ),
                Subsystem::Webhook => RetryPolicy::new(
                    Subsystem::Webhook,
                    Duration::from_secs(30),
                    2,
                    Duration::from_secs(3_600),
                    AttemptBudget::Limited(8),
                    Jitter::Full,
                ),
                Subsystem::Effects => Self::Balanced.policy(Subsystem::Effects),
                Subsystem::Consumer => RetryPolicy::new(
                    Subsystem::Consumer,
                    Duration::from_millis(250),
                    2,
                    Duration::from_secs(30),
                    AttemptBudget::Unlimited,
                    Jitter::Full,
                ),
                Subsystem::Relay => RetryPolicy::new(
                    Subsystem::Relay,
                    Duration::from_millis(250),
                    2,
                    Duration::from_secs(30),
                    AttemptBudget::Unlimited,
                    Jitter::Full,
                ),
                Subsystem::Decision => RetryPolicy::new(
                    Subsystem::Decision,
                    Duration::from_millis(200),
                    2,
                    Duration::from_secs(5),
                    AttemptBudget::Limited(5),
                    Jitter::Full,
                ),
            },
            Self::BestEffort => match subsystem {
                Subsystem::Queue => RetryPolicy::new(
                    Subsystem::Queue,
                    Duration::from_secs(2),
                    2,
                    Duration::from_secs(300),
                    AttemptBudget::Limited(4),
                    Jitter::Equal,
                ),
                Subsystem::Email => RetryPolicy::new(
                    Subsystem::Email,
                    Duration::from_secs(30),
                    2,
                    Duration::from_secs(900),
                    AttemptBudget::Limited(3),
                    Jitter::Equal,
                ),
                Subsystem::Webhook => RetryPolicy::new(
                    Subsystem::Webhook,
                    Duration::from_secs(30),
                    2,
                    Duration::from_secs(900),
                    AttemptBudget::Limited(3),
                    Jitter::Full,
                ),
                Subsystem::Effects => Self::Balanced.policy(Subsystem::Effects),
                Subsystem::Consumer => RetryPolicy::new(
                    Subsystem::Consumer,
                    Duration::from_millis(250),
                    2,
                    Duration::from_secs(30),
                    AttemptBudget::Unlimited,
                    Jitter::Full,
                ),
                Subsystem::Relay => RetryPolicy::new(
                    Subsystem::Relay,
                    Duration::from_millis(250),
                    2,
                    Duration::from_secs(10),
                    AttemptBudget::Unlimited,
                    Jitter::Full,
                ),
                Subsystem::Decision => RetryPolicy::new(
                    Subsystem::Decision,
                    Duration::from_millis(200),
                    2,
                    Duration::from_secs(1),
                    AttemptBudget::Limited(2),
                    Jitter::Full,
                ),
            },
            Self::SingleBox => match subsystem {
                Subsystem::Queue => RetryPolicy::new(
                    Subsystem::Queue,
                    Duration::from_secs(5),
                    2,
                    Duration::from_secs(900),
                    AttemptBudget::Limited(10),
                    Jitter::Full,
                ),
                Subsystem::Email => RetryPolicy::new(
                    Subsystem::Email,
                    Duration::from_secs(30),
                    2,
                    Duration::from_secs(3_600),
                    AttemptBudget::Limited(8),
                    Jitter::Full,
                ),
                Subsystem::Webhook => RetryPolicy::new(
                    Subsystem::Webhook,
                    Duration::from_secs(30),
                    2,
                    Duration::from_secs(3_600),
                    AttemptBudget::Limited(6),
                    Jitter::Full,
                ),
                Subsystem::Effects => Self::Balanced.policy(Subsystem::Effects),
                Subsystem::Consumer => RetryPolicy::new(
                    Subsystem::Consumer,
                    Duration::from_millis(250),
                    2,
                    Duration::from_secs(10),
                    AttemptBudget::Unlimited,
                    Jitter::Full,
                ),
                Subsystem::Relay => RetryPolicy::new(
                    Subsystem::Relay,
                    Duration::from_millis(250),
                    2,
                    Duration::from_secs(10),
                    AttemptBudget::Unlimited,
                    Jitter::Full,
                ),
                Subsystem::Decision => Self::Balanced.policy(Subsystem::Decision),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Sweep failure backoff
// ---------------------------------------------------------------------------
//
// A housekeeping sweep is not an attempt at a unit of work: it re-reads on a
// clock, has nothing to dead-letter into, and gets no `Subsystem` row or
// operator lever. What it does not get either is its own copy of "how long
// after a failed read" — that duplication is what this module removed, and two
// worker loops still carried it.

/// Backoff after a failed embedding re-index tick. Short: a failed tick is not
/// a reason to stop, and the next one may be the one that works.
pub const EMBEDDING_REINDEX_RETRY: Duration = Duration::from_secs(2);

/// Backoff after a failed retrieval-watch tick. Slower: the count moves with
/// the re-index job, not with requests, so a stricter reading would only chart
/// a stale number more often.
pub const RETRIEVAL_WATCH_RETRY: Duration = Duration::from_secs(30);

/// How many tries a unit of work gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptBudget {
    /// Never give up — a reconnect loop has nothing to dead-letter into, and
    /// stopping means a process that silently stops listening.
    Unlimited,
    /// Total tries including the first.
    Limited(u32),
}

impl AttemptBudget {
    /// `true` when `attempts_made` has reached the budget, i.e. the next
    /// failure must dead-letter instead of scheduling another try.
    #[must_use]
    pub const fn exhausted(self, attempts_made: u32) -> bool {
        match self {
            Self::Unlimited => false,
            Self::Limited(max) => attempts_made >= max,
        }
    }

    #[must_use]
    pub const fn limit(self) -> Option<u32> {
        match self {
            Self::Unlimited => None,
            Self::Limited(max) => Some(max),
        }
    }

    /// The budget as the `i32` the job row stores.
    #[must_use]
    pub const fn limit_i32(self, fallback: i32) -> i32 {
        match self {
            Self::Unlimited => fallback,
            Self::Limited(max) if max > 0 && max <= i32::MAX as u32 => max as i32,
            Self::Limited(_) => fallback,
        }
    }
}

/// How much of the computed delay is spread out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jitter {
    /// The raw delay. Two workers that failed at the same instant retry at the
    /// same instant — a thundering herd against the thing that just failed.
    None,
    /// Uniform in `[0, delay]`. The server-side standard: it can retry
    /// immediately, which is fine because the herd is fully broken up.
    Full,
    /// Uniform in `[delay/2, delay]`. Keeps a floor under the wait when the
    /// handler cannot tolerate a zero-delay replay.
    Equal,
}

impl Jitter {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Full => "full",
            Self::Equal => "equal",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Self::None),
            "full" => Some(Self::Full),
            "equal" | "half" => Some(Self::Equal),
            _ => None,
        }
    }

    /// Applies `sample` (in `[0, 1)`) to a computed delay.
    ///
    /// The result never exceeds `delay` and is never negative, so jitter can
    /// only shorten a wait. A policy that jittered *past* its cap would let a
    /// "capped at an hour" claim be false.
    #[must_use]
    pub fn apply(self, delay: Duration, sample: f64) -> Duration {
        let sample = if sample.is_finite() {
            sample.clamp(0.0, 1.0)
        } else {
            0.0
        };
        match self {
            Self::None => delay,
            Self::Full => delay.mul_f64(sample),
            Self::Equal => {
                let half = delay / 2;
                half + half.mul_f64(sample)
            }
        }
    }
}

/// What a refused request means for the retry decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Asking again may succeed with no change on our side: transport
    /// failure, 5xx, 408, 429.
    Transient,
    /// Refused on client-side grounds where a replay may help exactly when the
    /// operation is replay-inert — 409 above all. The policy decides.
    Client,
    /// The request is wrong, or the resource is gone. Retrying burns budget
    /// and hides the bug: 400, 401, 403, 404, 405, 410, 413, 422, and every
    /// other 4xx.
    Permanent,
}

impl ErrorClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Client => "client",
            Self::Permanent => "permanent",
        }
    }
}

/// Classifies an HTTP status without knowing the operation.
///
/// `409` deliberately lands in [`ErrorClass::Client`] rather than being
/// resolved here: "someone else wrote first" is retryable for an idempotent
/// replay and fatal for a non-idempotent one, and only the policy knows which
/// it is.
#[must_use]
pub const fn classify_http(status: u16) -> ErrorClass {
    match status {
        408 | 429 => ErrorClass::Transient,
        409 => ErrorClass::Client,
        500..=599 => ErrorClass::Transient,
        400..=499 => ErrorClass::Permanent,
        // 1xx/2xx/3xx are not failures; anything else (including a bogus 0
        // from a caller that lost the status) is treated as permanent so a
        // malformed input cannot turn into an infinite retry.
        _ => ErrorClass::Permanent,
    }
}

/// Whether a policy retries a `409`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPosture {
    /// Replay it. Correct only when the request cannot double-apply — every
    /// endpoint on the effects seam is replay-inert, which is why that seam
    /// takes this posture.
    Retry,
    /// Treat it like any other 4xx. The default for anything that is not
    /// provably replay-inert.
    Permanent,
}

impl ConflictPosture {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::Permanent => "permanent",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "retry" | "transient" => Some(Self::Retry),
            "permanent" | "dead" => Some(Self::Permanent),
            _ => None,
        }
    }
}

/// One subsystem's retry rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub subsystem: Subsystem,
    /// Delay before the second attempt.
    pub base: Duration,
    /// Multiplier per failure. `2` is doubling.
    pub factor: u32,
    /// No computed delay ever exceeds this. It is a real ceiling: the
    /// pre-policy queue clamped its shift, which silently made its stated
    /// hour cap unreachable.
    pub cap: Duration,
    pub budget: AttemptBudget,
    pub jitter: Jitter,
    /// What a `409` means here. Everything else follows
    /// [`classify_http`].
    pub on_conflict: ConflictPosture,
}

impl RetryPolicy {
    #[must_use]
    pub const fn new(
        subsystem: Subsystem,
        base: Duration,
        factor: u32,
        cap: Duration,
        budget: AttemptBudget,
        jitter: Jitter,
    ) -> Self {
        Self {
            subsystem,
            base,
            factor,
            cap,
            budget,
            jitter,
            // Conservative default: a policy has to opt into replaying a
            // conflict, never out of it.
            on_conflict: ConflictPosture::Permanent,
        }
    }

    #[must_use]
    pub const fn on_conflict(mut self, posture: ConflictPosture) -> Self {
        self.on_conflict = posture;
        self
    }

    #[must_use]
    pub const fn with_base(mut self, base: Duration) -> Self {
        self.base = base;
        self
    }

    #[must_use]
    pub const fn with_cap(mut self, cap: Duration) -> Self {
        self.cap = cap;
        self
    }

    #[must_use]
    pub const fn with_budget(mut self, budget: AttemptBudget) -> Self {
        self.budget = budget;
        self
    }

    #[must_use]
    pub const fn with_jitter(mut self, jitter: Jitter) -> Self {
        self.jitter = jitter;
        self
    }

    /// The delay before the attempt following `attempts_made` failures,
    /// before jitter. `attempts_made` is 1-based: the first failure waits
    /// `base`.
    #[must_use]
    pub fn base_delay(&self, attempts_made: u32) -> Duration {
        let exponent = attempts_made.saturating_sub(1).min(30);
        let factor = self.factor.max(1);
        let mut delay = self.base;
        for _ in 0..exponent {
            delay = delay.saturating_mul(factor);
            if delay >= self.cap {
                return self.cap;
            }
        }
        delay.min(self.cap)
    }

    /// The delay actually slept, with jitter applied from `sample`.
    #[must_use]
    pub fn delay(&self, attempts_made: u32, sample: f64) -> Duration {
        self.jitter.apply(self.base_delay(attempts_made), sample)
    }

    /// [`delay`](Self::delay) in whole seconds, at least 1, for callers that
    /// schedule in Postgres time (`make_interval(secs => $2)`).
    #[must_use]
    pub fn delay_secs(&self, attempts_made: u32, sample: f64) -> i64 {
        let delay = self.delay(attempts_made, sample);
        // Round up: a 500 ms backoff scheduled as 0 would turn a retry loop
        // into a spin.
        i64::try_from(delay.as_millis().div_ceil(1_000))
            .unwrap_or(i64::MAX)
            .max(1)
    }

    /// Whether a failure of this class should be retried at all. This is the
    /// classification half of the policy: it answers "is another attempt
    /// allowed to help", independent of the attempt budget.
    #[must_use]
    pub const fn retryable(&self, class: ErrorClass) -> bool {
        match class {
            ErrorClass::Transient => true,
            ErrorClass::Client => matches!(self.on_conflict, ConflictPosture::Retry),
            ErrorClass::Permanent => false,
        }
    }

    /// Classifies a status and applies the policy in one step.
    #[must_use]
    pub const fn retries_status(&self, status: u16) -> bool {
        self.retryable(classify_http(status))
    }

    /// `true` when the budget for `attempts_made` is used up.
    #[must_use]
    pub const fn exhausted(&self, attempts_made: u32) -> bool {
        self.budget.exhausted(attempts_made)
    }

    /// The job row's `max_attempts` for this policy.
    #[must_use]
    pub const fn max_attempts_i32(&self, fallback: i32) -> i32 {
        self.budget.limit_i32(fallback)
    }
}

/// Every policy in force for one process, plus where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryMatrix {
    preset: RobustnessPreset,
    policies: [RetryPolicy; 7],
}

impl RetryMatrix {
    /// The matrix a preset describes.
    #[must_use]
    pub fn for_preset(preset: RobustnessPreset) -> Self {
        let policies = Subsystem::ALL.map(|s| preset.policy(s));
        Self { preset, policies }
    }

    #[must_use]
    pub const fn preset(&self) -> RobustnessPreset {
        self.preset
    }

    #[must_use]
    pub fn policy(&self, subsystem: Subsystem) -> RetryPolicy {
        self.policies
            .into_iter()
            .find(|p| p.subsystem == subsystem)
            .unwrap_or_else(|| self.preset.policy(subsystem))
    }

    #[must_use]
    pub const fn policies(&self) -> &[RetryPolicy; 7] {
        &self.policies
    }

    fn replace(&mut self, policy: RetryPolicy) {
        if let Some(slot) = self
            .policies
            .iter_mut()
            .find(|p| p.subsystem == policy.subsystem)
        {
            *slot = policy;
        }
    }

    /// Resolves the preset and the per-subsystem overrides from a key lookup.
    ///
    /// The order is fixed and is the whole contract: **code default → preset →
    /// env group**. An override always wins over the preset; an unparseable
    /// override is reported and ignored rather than silently replaced by a
    /// guess, because an operator who typed `MARULA_RETRY_QUEUE_ATTEMPTS=ten`
    /// needs to know it did not take.
    #[must_use]
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> RetryResolution {
        let mut warnings = Vec::new();
        let preset = match lookup(ROBUSTNESS_ENV) {
            None => RobustnessPreset::Balanced,
            Some(raw) if raw.trim().is_empty() => RobustnessPreset::Balanced,
            Some(raw) => RobustnessPreset::parse(&raw).unwrap_or_else(|| {
                warnings.push(format!(
                    "{ROBUSTNESS_ENV}={raw:?} is not a preset (balanced, durable, best_effort, \
                     single_box); using balanced"
                ));
                RobustnessPreset::Balanced
            }),
        };
        let mut matrix = Self::for_preset(preset);
        for subsystem in Subsystem::ALL {
            let policy = matrix.policy(subsystem);
            let mut next = policy;
            for lever in Lever::ALL {
                let name = env_name(subsystem, lever);
                let Some(raw) = lookup(name) else {
                    continue;
                };
                if raw.trim().is_empty() {
                    continue;
                }
                match lever.apply(next, &raw) {
                    Ok(applied) => next = applied,
                    Err(reason) => warnings.push(format!("{name}: {reason}")),
                }
            }
            if next != policy {
                matrix.replace(next);
            }
        }
        RetryResolution { matrix, warnings }
    }
}

/// The result of resolving retries from an environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryResolution {
    pub matrix: RetryMatrix,
    /// One line per override that was refused, for the boot log. Empty is the
    /// normal case.
    pub warnings: Vec<String>,
}

/// The env that selects the preset.
pub const ROBUSTNESS_ENV: &str = "MARULA_ROBUSTNESS";

/// A per-subsystem lever an operator may override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lever {
    BaseMs,
    CapMs,
    Attempts,
    Jitter,
    Conflict,
}

impl Lever {
    pub const ALL: [Self; 5] = [
        Self::BaseMs,
        Self::CapMs,
        Self::Attempts,
        Self::Jitter,
        Self::Conflict,
    ];

    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::BaseMs => "BASE_MS",
            Self::CapMs => "CAP_MS",
            Self::Attempts => "ATTEMPTS",
            Self::Jitter => "JITTER",
            Self::Conflict => "CONFLICT",
        }
    }

    fn apply(self, policy: RetryPolicy, raw: &str) -> Result<RetryPolicy, String> {
        match self {
            Self::BaseMs => {
                let ms: u64 = parse_u64(raw)?;
                let base = Duration::from_millis(ms);
                // A base above the cap would make every computed delay equal
                // the cap; refuse it instead of silently flattening the curve.
                if base > policy.cap {
                    return Err(format!(
                        "base {ms} ms is above the cap of {} ms",
                        policy.cap.as_millis()
                    ));
                }
                Ok(policy.with_base(base))
            }
            Self::CapMs => {
                let ms: u64 = parse_u64(raw)?;
                let cap = Duration::from_millis(ms);
                if cap < policy.base {
                    return Err(format!(
                        "cap {ms} ms is below the base of {} ms",
                        policy.base.as_millis()
                    ));
                }
                Ok(policy.with_cap(cap))
            }
            Self::Attempts => {
                let attempts: u32 = raw
                    .trim()
                    .parse()
                    .map_err(|_| format!("{raw:?} is not an attempt count"))?;
                // 0 is "never give up" — the meaning a reconnect loop
                // already has, so the lever reads the same for every
                // subsystem instead of being inert on two of them.
                if attempts == 0 {
                    return Ok(policy.with_budget(AttemptBudget::Unlimited));
                }
                Ok(policy.with_budget(AttemptBudget::Limited(attempts)))
            }
            Self::Jitter => Jitter::parse(raw)
                .map(|j| policy.with_jitter(j))
                .ok_or_else(|| format!("{raw:?} is not none, full or equal")),
            Self::Conflict => ConflictPosture::parse(raw)
                .map(|c| policy.on_conflict(c))
                .ok_or_else(|| format!("{raw:?} is not retry or permanent")),
        }
    }
}

fn parse_u64(raw: &str) -> Result<u64, String> {
    let trimmed = raw.trim();
    trimmed
        .parse()
        .map_err(|_| format!("{trimmed:?} is not a number of milliseconds"))
}

/// The env name for one lever: `MARULA_RETRY_QUEUE_BASE_MS`.
///
/// A static table rather than `format!` so the deploy descriptor can emit a
/// `&'static str` name per lever without allocating, and so the full set is
/// greppable in one place.
#[must_use]
pub const fn env_name(subsystem: Subsystem, lever: Lever) -> &'static str {
    let row = match subsystem {
        Subsystem::Queue => 0,
        Subsystem::Email => 1,
        Subsystem::Webhook => 2,
        Subsystem::Effects => 3,
        Subsystem::Consumer => 4,
        Subsystem::Relay => 5,
        Subsystem::Decision => 6,
    };
    let column = match lever {
        Lever::BaseMs => 0,
        Lever::CapMs => 1,
        Lever::Attempts => 2,
        Lever::Jitter => 3,
        Lever::Conflict => 4,
    };
    RETRY_ENV_NAMES[row][column]
}

/// The table [`env_name`] reads. One row per [`Subsystem`], one column per
/// [`Lever`], in declaration order.
pub const RETRY_ENV_NAMES: [[&str; 5]; 7] = [
    [
        "MARULA_RETRY_QUEUE_BASE_MS",
        "MARULA_RETRY_QUEUE_CAP_MS",
        "MARULA_RETRY_QUEUE_ATTEMPTS",
        "MARULA_RETRY_QUEUE_JITTER",
        "MARULA_RETRY_QUEUE_CONFLICT",
    ],
    [
        "MARULA_RETRY_EMAIL_BASE_MS",
        "MARULA_RETRY_EMAIL_CAP_MS",
        "MARULA_RETRY_EMAIL_ATTEMPTS",
        "MARULA_RETRY_EMAIL_JITTER",
        "MARULA_RETRY_EMAIL_CONFLICT",
    ],
    [
        "MARULA_RETRY_WEBHOOK_BASE_MS",
        "MARULA_RETRY_WEBHOOK_CAP_MS",
        "MARULA_RETRY_WEBHOOK_ATTEMPTS",
        "MARULA_RETRY_WEBHOOK_JITTER",
        "MARULA_RETRY_WEBHOOK_CONFLICT",
    ],
    [
        "MARULA_RETRY_EFFECTS_BASE_MS",
        "MARULA_RETRY_EFFECTS_CAP_MS",
        "MARULA_RETRY_EFFECTS_ATTEMPTS",
        "MARULA_RETRY_EFFECTS_JITTER",
        "MARULA_RETRY_EFFECTS_CONFLICT",
    ],
    [
        "MARULA_RETRY_CONSUMER_BASE_MS",
        "MARULA_RETRY_CONSUMER_CAP_MS",
        "MARULA_RETRY_CONSUMER_ATTEMPTS",
        "MARULA_RETRY_CONSUMER_JITTER",
        "MARULA_RETRY_CONSUMER_CONFLICT",
    ],
    [
        "MARULA_RETRY_RELAY_BASE_MS",
        "MARULA_RETRY_RELAY_CAP_MS",
        "MARULA_RETRY_RELAY_ATTEMPTS",
        "MARULA_RETRY_RELAY_JITTER",
        "MARULA_RETRY_RELAY_CONFLICT",
    ],
    [
        "MARULA_RETRY_DECISION_BASE_MS",
        "MARULA_RETRY_DECISION_CAP_MS",
        "MARULA_RETRY_DECISION_ATTEMPTS",
        "MARULA_RETRY_DECISION_JITTER",
        "MARULA_RETRY_DECISION_CONFLICT",
    ],
];

/// Every env name the matrix reads, preset first. The deploy descriptor is
/// checked against this list so a lever cannot exist without being
/// discoverable.
#[must_use]
pub fn env_names() -> Vec<&'static str> {
    let mut names = vec![ROBUSTNESS_ENV];
    for row in &RETRY_ENV_NAMES {
        names.extend(row.iter().copied());
    }
    names
}

static MATRIX: OnceLock<RetryMatrix> = OnceLock::new();

/// Installs the process matrix. First caller wins; a second call is a no-op
/// returning the matrix already in force, so all-in-one mode can initialise
/// from either half without a boot-order dependency.
pub fn install(matrix: RetryMatrix) -> RetryMatrix {
    *MATRIX.get_or_init(|| matrix)
}

/// The matrix in force. `balanced` until something installs one, which is what
/// tests and one-off binaries get.
#[must_use]
pub fn matrix() -> RetryMatrix {
    *MATRIX.get_or_init(|| RetryMatrix::for_preset(RobustnessPreset::Balanced))
}

/// Shorthand for the hot paths: `retry::policy(Subsystem::Queue)`.
#[must_use]
pub fn policy(subsystem: Subsystem) -> RetryPolicy {
    matrix().policy(subsystem)
}

/// A subsystem's attempt budget as the `i32` a job row stores. `Unlimited`
/// resolves to `i32::MAX`, which is what "no budget" means in a column that
/// has no null.
#[must_use]
pub fn attempt_limit_i32(subsystem: Subsystem) -> i32 {
    policy(subsystem)
        .budget
        .limit()
        .map_or(i32::MAX, |n| i32::try_from(n).unwrap_or(i32::MAX))
}

static SEED: OnceLock<u64> = OnceLock::new();
static SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Seeds the jitter sample stream. First caller wins; unseeded processes use a
/// fixed stream, which is fine for a single worker and keeps tests
/// deterministic.
pub fn seed_jitter(seed: u64) {
    let _ = SEED.set(seed);
}

/// A jitter sample in `[0, 1)`. Splitmix64 over a per-call counter: cheap, no
/// dependency, and decorrelated across the attempts of one process — the herd
/// this exists to break up is made of failures that happen together.
#[must_use]
pub fn sample() -> f64 {
    let seed = *SEED.get_or_init(|| 0x9E37_79B9_7F4A_7C15);
    let counter = SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut z = seed
        .wrapping_add(counter.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // 53 bits of mantissa is exactly the float precision; dividing keeps the
    // result in [0, 1).
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// The delay for `attempts_made` failures under the installed policy, with a
/// fresh jitter sample. The call site every retry loop should use.
#[must_use]
pub fn delay(subsystem: Subsystem, attempts_made: u32) -> Duration {
    policy(subsystem).delay(attempts_made, sample())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_answers_for_every_subsystem() {
        for preset in RobustnessPreset::ALL {
            let matrix = RetryMatrix::for_preset(preset);
            for subsystem in Subsystem::ALL {
                let policy = matrix.policy(subsystem);
                assert_eq!(policy.subsystem, subsystem);
                assert!(
                    policy.cap >= policy.base,
                    "{} {} has a cap below its base",
                    preset.as_str(),
                    subsystem.as_str()
                );
                assert!(policy.factor >= 1, "factor zero would freeze the backoff");
            }
        }
    }

    #[test]
    fn base_delay_doubles_then_sits_on_the_cap() {
        let policy = RetryPolicy::new(
            Subsystem::Queue,
            Duration::from_secs(5),
            2,
            Duration::from_secs(3_600),
            AttemptBudget::Limited(10),
            Jitter::None,
        );
        assert_eq!(policy.base_delay(1), Duration::from_secs(5));
        assert_eq!(policy.base_delay(2), Duration::from_secs(10));
        assert_eq!(policy.base_delay(3), Duration::from_secs(20));
        // 5·2^9 = 2560, still under the hour.
        assert_eq!(policy.base_delay(10), Duration::from_secs(2_560));
        // 5·2^10 = 5120 is clamped to the cap, and stays there forever.
        assert_eq!(policy.base_delay(11), Duration::from_secs(3_600));
        assert_eq!(policy.base_delay(64), Duration::from_secs(3_600));
        // A nonsensical attempt count must not shift by 4 billion.
        assert_eq!(policy.base_delay(0), Duration::from_secs(5));
    }

    /// The defect this lane exists for: the queue clamped its shift at 8, so
    /// `min(3600)` was dead code and the docstring's hour was unreachable. The
    /// cap must be reachable.
    #[test]
    fn the_stated_cap_is_reachable() {
        let configured = policy_preset(Subsystem::Queue);
        assert_eq!(
            configured.base_delay(12),
            configured.cap,
            "the cap must be reached by the delay curve, not just declared"
        );
        assert!(
            configured.cap >= Duration::from_secs(3_600),
            "the queue promises an hour; got {:?}",
            configured.cap
        );
    }

    fn policy_preset(subsystem: Subsystem) -> RetryPolicy {
        RobustnessPreset::Balanced.policy(subsystem)
    }

    #[test]
    fn jitter_never_exceeds_the_cap_or_goes_negative() {
        let policy = policy_preset(Subsystem::Queue);
        for attempt in 1..40 {
            for sample in [0.0, 0.25, 0.5, 0.999_999] {
                let delay = policy.delay(attempt, sample);
                assert!(delay <= policy.cap, "jitter pushed past the cap");
                assert!(
                    delay <= policy.base_delay(attempt),
                    "jitter lengthened the wait"
                );
            }
        }
        // A sample outside [0,1) — a caller bug — must not produce a delay
        // longer than the cap or a panic.
        assert!(policy.delay(3, 4.0) <= policy.cap);
        assert_eq!(policy.delay(3, f64::NAN), Duration::ZERO);
    }

    #[test]
    fn full_jitter_spans_zero_to_the_delay_and_equal_jitter_keeps_a_floor() {
        let full = RetryPolicy::new(
            Subsystem::Queue,
            Duration::from_secs(10),
            2,
            Duration::from_secs(3_600),
            AttemptBudget::Limited(3),
            Jitter::Full,
        );
        assert_eq!(full.delay(1, 0.0), Duration::ZERO);
        assert_eq!(full.delay(1, 1.0), Duration::from_secs(10));
        assert_eq!(full.delay(1, 0.5), Duration::from_secs(5));

        let equal = RetryPolicy::new(
            Subsystem::Email,
            Duration::from_secs(10),
            2,
            Duration::from_secs(3_600),
            AttemptBudget::Limited(3),
            Jitter::Equal,
        );
        assert_eq!(equal.delay(1, 0.0), Duration::from_secs(5));
        assert_eq!(equal.delay(1, 1.0), Duration::from_secs(10));

        let none = RetryPolicy::new(
            Subsystem::Effects,
            Duration::from_secs(10),
            2,
            Duration::from_secs(3_600),
            AttemptBudget::Limited(1),
            Jitter::None,
        );
        assert_eq!(none.delay(1, 0.0), Duration::from_secs(10));
    }

    /// Two callers that fail at the same instant must not sleep for the same
    /// duration — that is the whole point of jitter, and it is what every
    /// retry loop in the tree was missing.
    #[test]
    fn the_sample_stream_decorrelates_successive_calls() {
        let first = sample();
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..32 {
            let value = sample();
            assert!((0.0..1.0).contains(&value), "sample outside [0,1): {value}");
            distinct.insert((value * 1_000_000.0) as u64);
        }
        assert!(distinct.len() > 24, "the sample stream is clustering");
        assert!((0.0..1.0).contains(&first));
    }

    /// 408 and 429 are the two 4xx that mean "ask again". Before this lane
    /// every 4xx was permanent at the effects seam, so a rate-limited core-api
    /// dead-lettered the job that hit it.
    #[test]
    fn timeout_and_rate_limit_are_transient_every_other_4xx_is_permanent() {
        assert_eq!(classify_http(408), ErrorClass::Transient);
        assert_eq!(classify_http(425), ErrorClass::Permanent);
        assert_eq!(classify_http(429), ErrorClass::Transient);
        for status in [400, 401, 403, 404, 405, 410, 413, 415, 422, 404] {
            assert_eq!(
                classify_http(status),
                ErrorClass::Permanent,
                "{status} must not be retried"
            );
        }
        for status in [500, 502, 503, 504, 522] {
            assert_eq!(classify_http(status), ErrorClass::Transient);
        }
        // A status no HTTP stack should produce stays permanent: retrying it
        // forever would be the worst possible reading.
        assert_eq!(classify_http(0), ErrorClass::Permanent);
        assert_eq!(classify_http(302), ErrorClass::Permanent);
    }

    /// 409 is the operation's call. The seam that can replay opts in; a policy
    /// that has not said so must not.
    #[test]
    fn conflict_is_retried_only_where_the_policy_opts_in() {
        let effects = policy_preset(Subsystem::Effects);
        assert_eq!(effects.on_conflict, ConflictPosture::Retry);
        assert!(effects.retries_status(409));
        assert!(effects.retries_status(429));
        assert!(!effects.retries_status(404));

        let mut conservative = effects.on_conflict(ConflictPosture::Permanent);
        assert!(!conservative.retries_status(409));
        // The opt-out is only about 409: a 429 still retries.
        assert!(conservative.retries_status(429));

        conservative = conservative.on_conflict(ConflictPosture::Retry);
        assert!(conservative.retryable(ErrorClass::Client));
    }

    #[test]
    fn the_effects_seam_keeps_one_attempt_so_the_queue_owns_durability() {
        for preset in RobustnessPreset::ALL {
            let effects = preset.policy(Subsystem::Effects);
            assert_eq!(effects.budget, AttemptBudget::Limited(1));
            assert!(
                effects.exhausted(1),
                "a single attempt is spent after one failure"
            );
        }
    }

    #[test]
    fn attempt_budget_counts_tries_including_the_first() {
        let budget = AttemptBudget::Limited(3);
        assert!(!budget.exhausted(1));
        assert!(!budget.exhausted(2));
        assert!(budget.exhausted(3));
        assert!(budget.exhausted(4));
        assert!(!AttemptBudget::Unlimited.exhausted(10_000));
        assert_eq!(AttemptBudget::Limited(6).limit_i32(1), 6);
        assert_eq!(AttemptBudget::Unlimited.limit_i32(4), 4);
    }

    #[test]
    fn presets_parse_both_spellings_and_refuse_nonsense() {
        assert_eq!(
            RobustnessPreset::parse("balanced"),
            Some(RobustnessPreset::Balanced)
        );
        assert_eq!(
            RobustnessPreset::parse(" DURABLE "),
            Some(RobustnessPreset::Durable)
        );
        assert_eq!(
            RobustnessPreset::parse("best_effort"),
            Some(RobustnessPreset::BestEffort)
        );
        assert_eq!(
            RobustnessPreset::parse("best-effort"),
            Some(RobustnessPreset::BestEffort)
        );
        assert_eq!(
            RobustnessPreset::parse("single_box"),
            Some(RobustnessPreset::SingleBox)
        );
        assert_eq!(RobustnessPreset::parse("singlebox"), None);
        assert_eq!(
            RobustnessPreset::parse("durable "),
            Some(RobustnessPreset::Durable)
        );
    }

    #[test]
    fn the_preset_is_what_changes_the_budget() {
        let balanced = RetryMatrix::for_preset(RobustnessPreset::Balanced);
        let durable = RetryMatrix::for_preset(RobustnessPreset::Durable);
        assert!(
            durable.policy(Subsystem::Queue).budget.limit()
                > balanced.policy(Subsystem::Queue).budget.limit(),
            "durable must try more times than balanced"
        );
        let best_effort = RetryMatrix::for_preset(RobustnessPreset::BestEffort);
        assert!(
            best_effort.policy(Subsystem::Queue).budget.limit()
                < balanced.policy(Subsystem::Queue).budget.limit()
        );
    }

    #[test]
    fn an_unset_environment_is_balanced() {
        let resolved = RetryMatrix::from_lookup(|_| None);
        assert_eq!(resolved.matrix.preset(), RobustnessPreset::Balanced);
        assert_eq!(
            resolved.matrix,
            RetryMatrix::for_preset(RobustnessPreset::Balanced)
        );
        assert!(resolved.warnings.is_empty());
    }

    #[test]
    fn an_unknown_preset_warns_and_keeps_balanced_rather_than_guessing() {
        let resolved = RetryMatrix::from_lookup(|key| match key {
            ROBUSTNESS_ENV => Some("turbo".into()),
            _ => None,
        });
        assert_eq!(resolved.matrix.preset(), RobustnessPreset::Balanced);
        assert_eq!(resolved.warnings.len(), 1);
        assert!(resolved.warnings[0].contains("turbo"));
    }

    /// Precedence: code default → preset → env group. The last one wins, and
    /// only for the subsystem it names.
    #[test]
    fn a_subsystem_group_overrides_the_preset_without_touching_its_siblings() {
        let resolved = RetryMatrix::from_lookup(|key| match key {
            ROBUSTNESS_ENV => Some("durable".into()),
            "MARULA_RETRY_QUEUE_ATTEMPTS" => Some("3".into()),
            "MARULA_RETRY_QUEUE_BASE_MS" => Some("1500".into()),
            "MARULA_RETRY_QUEUE_JITTER" => Some("equal".into()),
            "MARULA_RETRY_WEBHOOK_CONFLICT" => Some("permanent".into()),
            _ => None,
        });
        assert!(resolved.warnings.is_empty(), "{:?}", resolved.warnings);
        assert_eq!(resolved.matrix.preset(), RobustnessPreset::Durable);

        let queue = resolved.matrix.policy(Subsystem::Queue);
        assert_eq!(queue.budget, AttemptBudget::Limited(3));
        assert_eq!(queue.base, Duration::from_millis(1_500));
        assert_eq!(queue.jitter, Jitter::Equal);
        // Untouched levers still come from the preset.
        assert_eq!(
            queue.cap,
            RobustnessPreset::Durable.policy(Subsystem::Queue).cap
        );

        assert_eq!(
            resolved.matrix.policy(Subsystem::Webhook).on_conflict,
            ConflictPosture::Permanent
        );
        // A sibling's budget is still the preset's.
        assert_eq!(
            resolved.matrix.policy(Subsystem::Email).budget,
            RobustnessPreset::Durable.policy(Subsystem::Email).budget
        );
    }

    #[test]
    fn an_attempt_count_of_zero_means_never_give_up() {
        let resolved = RetryMatrix::from_lookup(|key| match key {
            "MARULA_RETRY_CONSUMER_ATTEMPTS" => Some("0".into()),
            "MARULA_RETRY_QUEUE_ATTEMPTS" => Some("0".into()),
            _ => None,
        });
        assert!(resolved.warnings.is_empty(), "{:?}", resolved.warnings);
        assert_eq!(
            resolved.matrix.policy(Subsystem::Queue).budget,
            AttemptBudget::Unlimited
        );
        assert!(!resolved
            .matrix
            .policy(Subsystem::Queue)
            .exhausted(1_000_000));
    }

    #[test]
    fn a_refused_override_is_reported_and_left_alone() {
        let resolved = RetryMatrix::from_lookup(|key| match key {
            "MARULA_RETRY_QUEUE_ATTEMPTS" => Some("ten".into()),
            "MARULA_RETRY_QUEUE_BASE_MS" => Some("99999999".into()),
            "MARULA_RETRY_EMAIL_JITTER" => Some("sometimes".into()),
            "MARULA_RETRY_QUEUE_CAP_MS" => Some("1".into()),
            _ => None,
        });
        // All four are refused; none silently changes the policy.
        assert_eq!(resolved.warnings.len(), 4, "{:?}", resolved.warnings);
        assert_eq!(
            resolved.matrix.policy(Subsystem::Queue),
            RetryMatrix::for_preset(RobustnessPreset::Balanced).policy(Subsystem::Queue)
        );
    }

    #[test]
    fn every_lever_has_a_unique_name_from_the_table() {
        let names = env_names();
        assert_eq!(names.len(), 1 + Subsystem::ALL.len() * Lever::ALL.len());
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "duplicate env name");
        assert!(unique.contains(ROBUSTNESS_ENV));
        assert!(unique.contains("MARULA_RETRY_QUEUE_BASE_MS"));
        assert!(unique.contains("MARULA_RETRY_DECISION_CONFLICT"));

        // The table is ordered like the enums, so a lever added to one without
        // the other is caught here rather than becoming a descriptor gap.
        for (row, subsystem) in Subsystem::ALL.into_iter().enumerate() {
            for (column, lever) in Lever::ALL.into_iter().enumerate() {
                let name = env_name(subsystem, lever);
                assert_eq!(name, RETRY_ENV_NAMES[row][column]);
                assert_eq!(
                    name,
                    format!(
                        "MARULA_RETRY_{}_{}",
                        subsystem.as_str().to_ascii_uppercase(),
                        lever.suffix()
                    ),
                    "the static table and the naming convention disagree"
                );
            }
        }
    }

    #[test]
    fn delay_secs_rounds_up_so_a_sub_second_backoff_cannot_spin() {
        let policy = RetryPolicy::new(
            Subsystem::Consumer,
            Duration::from_millis(250),
            2,
            Duration::from_secs(10),
            AttemptBudget::Unlimited,
            Jitter::None,
        );
        assert_eq!(policy.delay_secs(1, 0.0), 1);
        assert_eq!(policy.delay_secs(2, 0.0), 1);
        assert_eq!(policy.delay_secs(6, 0.0), 8);
    }
}
