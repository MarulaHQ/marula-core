//! The reconnect backoff every stream consumer and listener shares.
//!
//! Purpose: one object that owns "how long do I wait before reconnecting" so
//! the ten consumer loops in this crate stop each carrying their own
//! `100_u64` and their own `(backoff_ms * 2).min(5_000)`. The schedule comes
//! from [`core_domain::retry`]'s consumer and relay policies, which means the
//! numbers, the jitter and the operator overrides all live in one place.
//!
//! Non-purpose: no reconnection, no stream handling. This only computes and
//! performs the sleep; the loop still decides when a connection ended.
//!
//! A reconnect loop has no dead-letter to fall into — stopping means a process
//! that silently stops listening — so its policy is `AttemptBudget::Unlimited`.
//! The cap is what bounds it, not a budget.

use core_domain::retry::{self, RetryPolicy, Subsystem};
use std::time::Duration;

/// A per-loop backoff state machine.
#[derive(Debug, Clone)]
pub struct Reconnect {
    policy: RetryPolicy,
    /// Failures since the last successful connection. 1 after the first.
    attempts: u32,
}

impl Reconnect {
    /// For a stream consumer (`Subsystem::Consumer`).
    #[must_use]
    pub fn consumer() -> Self {
        Self::new(Subsystem::Consumer)
    }

    /// For the outbox relay and the realtime listener (`Subsystem::Relay`).
    #[must_use]
    pub fn relay() -> Self {
        Self::new(Subsystem::Relay)
    }

    #[must_use]
    pub fn new(subsystem: Subsystem) -> Self {
        Self {
            policy: retry::policy(subsystem),
            attempts: 0,
        }
    }

    /// A connection succeeded: the next failure starts at the base delay
    /// again, not where the last outage left off.
    pub fn reset(&mut self) {
        self.attempts = 0;
    }

    /// The delay for the next reconnect, jittered, advancing the counter.
    pub fn next_delay(&mut self) -> Duration {
        self.attempts = self.attempts.saturating_add(1);
        self.policy.delay(self.attempts, retry::sample())
    }

    /// The delay the current attempt would get, without advancing.
    #[must_use]
    pub fn peek_delay(&self) -> Duration {
        self.policy
            .delay(self.attempts.saturating_add(1), retry::sample())
    }

    /// Sleeps the next backoff. The call site's whole retry loop becomes
    /// `backoff.sleep().await`.
    pub async fn sleep(&mut self) {
        tokio::time::sleep(self.next_delay()).await;
    }

    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_reconnect_waits_the_policys_base() {
        let mut reconnect = Reconnect::consumer();
        let base = retry::policy(Subsystem::Consumer).base;
        for _ in 0..16 {
            reconnect.reset();
            assert!(
                reconnect.next_delay() <= base,
                "a first reconnect slept longer than the base"
            );
        }
    }

    #[test]
    fn successive_failures_wait_longer_until_the_cap() {
        let mut reconnect = Reconnect::relay();
        let cap = retry::policy(Subsystem::Relay).cap;
        let mut previous = Duration::ZERO;
        // Compare the unjittered ceilings: sampled values can dip by design,
        // but the window must widen.
        let policy = retry::policy(Subsystem::Relay);
        for attempt in 1..12 {
            let ceiling = policy.delay(attempt, 1.0);
            assert!(
                ceiling >= previous,
                "the window narrowed at attempt {attempt}"
            );
            assert!(ceiling <= cap);
            previous = ceiling;
        }
        for _ in 0..64 {
            assert!(reconnect.next_delay() <= cap);
        }
    }

    #[test]
    fn a_successful_connection_resets_the_schedule() {
        let mut reconnect = Reconnect::consumer();
        for _ in 0..10 {
            let _ = reconnect.next_delay();
        }
        assert_eq!(reconnect.attempts(), 10);
        reconnect.reset();
        assert_eq!(reconnect.attempts(), 0);
        assert!(reconnect.peek_delay() <= retry::policy(Subsystem::Consumer).base);
    }

    /// The policy is jittered, so two replicas that lose the same connection
    /// do not reconnect in lockstep.
    #[test]
    fn two_replicas_do_not_reconnect_in_lockstep() {
        let mut first = Reconnect::consumer();
        let mut second = Reconnect::consumer();
        let a: Vec<Duration> = (0..32).map(|_| first.next_delay()).collect();
        let b: Vec<Duration> = (0..32).map(|_| second.next_delay()).collect();
        assert_ne!(a, b, "the reconnect schedule is not jittered");
    }
}
