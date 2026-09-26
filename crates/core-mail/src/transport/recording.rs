//! Test/dev transport doubles: a recorder asserting exactly-once delivery
//! and a flaky wrapper injecting deterministic failures to prove the
//! retry/backoff path records each message exactly once.
//!
//! Purpose: the recording adapter backs the CI mail harness (acceptance
//! anchor "two simulated weeks, zero lost messages") and unit tests; the
//! noop transport is the disabled-deployment stand-in that logs instead of
//! sending.
//!
//! Non-purpose: neither double performs real delivery — never point a
//! production deployment at them.

use crate::transport::{MailTransport, OutboundEmail, SendReceipt, TransportError};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

/// Records every send in memory. The harness asserts one entry per queued
/// message — the zero-loss invariant.
#[derive(Default)]
pub struct RecordingTransport {
    sent: Mutex<Vec<OutboundEmail>>,
}

impl RecordingTransport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshots everything recorded so far (arrival order preserved).
    pub async fn recorded(&self) -> Vec<OutboundEmail> {
        self.sent.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub async fn count(&self) -> usize {
        self.sent.lock().map(|g| g.len()).unwrap_or_default()
    }
}

#[async_trait]
impl MailTransport for RecordingTransport {
    fn name(&self) -> &'static str {
        "recording"
    }

    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        if let Ok(mut guard) = self.sent.lock() {
            guard.push(message.clone());
        }
        Ok(SendReceipt {
            provider_message_id: Some(format!("rec-{}", message.rfc_message_id)),
        })
    }
}

/// Fails the first `failures` attempts of each distinct message (keyed by
/// RFC Message-ID), then delegates. Deterministic — no randomness.
pub struct FlakyTransport<T> {
    pub inner: T,
    pub failures: usize,
    attempts: Mutex<HashMap<String, usize>>,
}

impl<T> FlakyTransport<T> {
    #[must_use]
    pub fn new(inner: T, failures: usize) -> Self {
        Self {
            inner,
            failures,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    /// Attempts counted so far per Message-ID — proves backoff really
    /// retried rather than silently dropping.
    pub async fn attempt_count(&self, rfc_message_id: &str) -> usize {
        self.attempts
            .lock()
            .map(|m| m.get(rfc_message_id).copied().unwrap_or(0))
            .unwrap_or(0)
    }
}

#[async_trait]
impl<T: MailTransport + Sync> MailTransport for FlakyTransport<T> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        let n = {
            let mut map = self
                .attempts
                .lock()
                .map_err(|_| TransportError::Unavailable("flaky state poisoned".to_string()))?;
            let e = map.entry(message.rfc_message_id.clone()).or_insert(0);
            *e += 1;
            *e
        };
        if n <= self.failures {
            return Err(TransportError::Unavailable(format!(
                "injected failure {n}/{}",
                self.failures
            )));
        }
        self.inner.send(message).await
    }
}

/// Logs-and-discards transport for deployments with sending disabled:
/// reports success so the pipeline completes without side effects.
///
/// None-mode note: this IS the outbound none mode — visible, logged, and
/// never touching a real provider. The ledger marks such messages sent
/// with transport label `none`, making the no-op explicit in data.
pub struct NoopTransport;

/// Blanket forwarding impls so wrappers can borrow shared transports.
#[async_trait]
impl<T: MailTransport + ?Sized + Sync> MailTransport for std::sync::Arc<T> {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        (**self).send(message).await
    }
}

#[async_trait]
impl<T: MailTransport + ?Sized + Sync> MailTransport for &T {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        (**self).send(message).await
    }
}

#[async_trait]
impl MailTransport for NoopTransport {
    fn name(&self) -> &'static str {
        "none"
    }

    async fn send(&self, _message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        tracing::info!(subject = %_message.subject, "email transport=none: not sent");
        Ok(SendReceipt {
            provider_message_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str) -> OutboundEmail {
        OutboundEmail {
            from_address: "f@a.test".into(),
            from_name: None,
            reply_to: None,
            to_addresses: vec!["t@a.test".into()],
            subject: "s".into(),
            body_text: "b".into(),
            rfc_message_id: id.into(),
            in_reply_to: None,
            references: vec![],
        }
    }

    #[tokio::test]
    async fn recording_counts_exactly_one_per_send() {
        let rec = RecordingTransport::new();
        rec.send(&msg("<a@x>")).await.expect("send");
        rec.send(&msg("<b@x>")).await.expect("send");
        assert_eq!(rec.count().await, 2);
        let all = rec.recorded().await;
        assert_eq!(all[0].rfc_message_id, "<a@x>");
    }

    #[tokio::test]
    async fn flaky_fails_then_succeeds_and_counts_attempts() {
        let rec = RecordingTransport::new();
        let flaky = FlakyTransport::new(&rec, 2);
        let m = msg("<retry@x>");
        assert!(flaky.send(&m).await.is_err());
        assert!(flaky.send(&m).await.is_err());
        assert_eq!(rec.count().await, 0);
        flaky.send(&m).await.expect("third attempt succeeds");
        assert_eq!(flaky.attempt_count("<retry@x>").await, 3);
        assert_eq!(rec.count().await, 1);
    }
}
