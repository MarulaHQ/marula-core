//! worker/stream_health — consumer-group health gauges (PEL depth, lag, age).
//!
//! Purpose: the metrics stopped where the operations began. `marula.outbox.lag_ms`
//! covers the outbox relay, and `marula.queue.*` covers the Postgres job
//! queue, but every real-time consumer in this process reads the same Valkey
//! stream through its OWN consumer group, and nothing reported what any of
//! them was holding. A consumer that stops acking (a handler that fails
//! forever, a wedged connection) grows an invisible backlog: the stream looks
//! healthy, the outbox is drained, and tickets simply stop being processed.
//!
//! Three numbers, read straight from Valkey, per group:
//!
//! * `XINFO GROUPS` → `pending` (the PEL: delivered, never acked) and `lag`
//!   (entries the group has not been delivered at all).
//! * `XPENDING <stream> <group>` → `start_id`, the oldest unacked entry. A
//!   stream id is `<unix-ms>-<seq>`, so its first component IS the enqueue
//!   time: the age needs no extra lookup and no clock agreement between
//!   host and server.
//!
//! Non-purpose: no reclamation (`XCLAIM` lives in the consumers), no alert
//! thresholds, no history. This module reads; it never writes to the stream.
//!
//! Failure stance: a probe that cannot reach Valkey emits NOTHING and logs
//! at debug. The absence of a sample is the signal an operator needs, and a
//! fabricated `0` would hide exactly the stall this exists to expose.

use core_events::EVENT_STREAM;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;

/// One consumer group's liveness sample, before it becomes a metric.
///
/// Separate from the emit so the arithmetic is testable without Valkey: the
/// interesting bugs here are "age is negative" and "empty PEL reports a huge
/// age", not the round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupSample {
    /// Entries delivered to the group and never acknowledged.
    pub pel_depth: u64,
    /// Entries the group has not been delivered yet, when the server can
    /// compute it. `None` is reported as absence, never as zero.
    pub lag: Option<u64>,
    /// Age of the oldest unacked entry, in seconds. Zero when the PEL is
    /// empty — nothing is waiting, so nothing is old.
    pub oldest_pending_age_seconds: u64,
}

/// The millisecond component of a Valkey stream id (`<ms>-<seq>`).
///
/// `None` for anything that is not a well-formed id — a malformed id is a
/// reason to skip the age, not to invent one.
#[must_use]
pub fn stream_id_ms(id: &str) -> Option<u64> {
    id.split_once('-')?.0.parse().ok()
}

/// Age in seconds of an entry enqueued at `enqueued_ms`, given `now_ms`.
///
/// Clamped at zero: a clock that ran backwards on the Valkey host must read
/// as "just now", never as a negative age that wraps into an enormous
/// positive one once it is stored in a `u64` gauge.
#[must_use]
pub fn age_seconds(enqueued_ms: u64, now_ms: u64) -> u64 {
    now_ms.saturating_sub(enqueued_ms) / 1000
}

/// Builds the sample from the two replies. Split out so a test can pin the
/// empty-PEL rule and the malformed-id rule without a server.
#[must_use]
fn sample_from(
    pel_depth: u64,
    lag: Option<u64>,
    oldest_pending_id: Option<&str>,
    now_ms: u64,
) -> GroupSample {
    // An empty PEL has no oldest entry, and its honest age is zero. This is
    // the one place zero means something true, so it is stated rather than
    // left to fall out of a missing id.
    let oldest_pending_age_seconds = if pel_depth == 0 {
        0
    } else {
        oldest_pending_id
            .and_then(stream_id_ms)
            .map_or(0, |ms| age_seconds(ms, now_ms))
    };
    GroupSample {
        pel_depth,
        lag,
        oldest_pending_age_seconds,
    }
}

/// Reads one consumer group and emits its gauges.
///
/// Call it from the consumer's own loop, after its batch: the group it
/// reports on is the group that loop owns, so a consumer that dies takes its
/// own series stale instead of leaving a healthy-looking number behind.
pub async fn probe(conn: &mut MultiplexedConnection, group: &str) {
    if let Some(sample) = read_group(conn, group).await {
        emit(group, sample);
    }
}

/// Records one sample. Split from the read so a consumer that hides its
/// connection behind a seam (webhooks-out) can still emit the same shape.
pub fn emit(group: &str, sample: GroupSample) {
    core_otel::metrics::consumer_group_health(
        group,
        sample.pel_depth,
        sample.lag,
        sample.oldest_pending_age_seconds,
    );
}

/// The Valkey half of [`probe`]: two commands, no metric side effects.
pub async fn read_group(conn: &mut MultiplexedConnection, group: &str) -> Option<GroupSample> {
    // XINFO GROUPS: does the group exist, how big is its PEL, and how far
    // behind is it? A group that is not there yet (the consumer has not run
    // its first `XGROUP CREATE`) has no backlog to report.
    let groups: redis::streams::StreamInfoGroupsReply = match conn.xinfo_groups(EVENT_STREAM).await
    {
        Ok(reply) => reply,
        Err(error) => {
            tracing::debug!(%error, group, "consumer health: xinfo groups failed");
            return None;
        }
    };
    let info = groups.groups.iter().find(|g| g.name == group)?;

    // XPENDING summary: the oldest unacked entry's id, whose millisecond
    // component is its enqueue time. Not fetched when the PEL is empty —
    // the reply has no start id then anyway.
    let oldest_pending_id = if info.pending == 0 {
        None
    } else {
        match conn
            .xpending::<&str, &str, redis::streams::StreamPendingReply>(EVENT_STREAM, group)
            .await
        {
            Ok(redis::streams::StreamPendingReply::Data(data)) => Some(data.start_id),
            Ok(redis::streams::StreamPendingReply::Empty) => None,
            Err(error) => {
                tracing::debug!(%error, group, "consumer health: xpending failed");
                None
            }
        }
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    Some(sample_from(
        info.pending as u64,
        info.lag.map(|l| l as u64),
        oldest_pending_id.as_deref(),
        now_ms,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_ids_yield_their_enqueue_millisecond() {
        assert_eq!(stream_id_ms("1700000000000-0"), Some(1_700_000_000_000));
        assert_eq!(stream_id_ms("1700000000000-19"), Some(1_700_000_000_000));
        assert_eq!(stream_id_ms("not-an-id"), None);
        assert_eq!(stream_id_ms(""), None);
        assert_eq!(stream_id_ms("12345"), None);
    }

    #[test]
    fn age_is_derived_from_the_stream_id_not_the_host_clock() {
        // 90 seconds between enqueue and read, regardless of who is running.
        assert_eq!(age_seconds(1_000_000, 1_090_000), 90);
        assert_eq!(age_seconds(1_000_000, 1_000_900), 0);
    }

    #[test]
    fn a_backwards_clock_never_produces_a_negative_age() {
        // Postgres/Valkey clock behind the reader: clamp to zero rather than
        // wrap into ~584 billion years once stored in a u64 gauge.
        assert_eq!(age_seconds(1_090_000, 1_000_000), 0);
    }

    #[test]
    fn an_empty_pel_reports_zero_age_rather_than_a_stale_entry() {
        let sample = sample_from(0, Some(12), None, 1_700_000_000_000);
        assert_eq!(sample.pel_depth, 0);
        assert_eq!(sample.oldest_pending_age_seconds, 0);
        assert_eq!(sample.lag, Some(12), "lag is independent of the PEL");
    }

    #[test]
    fn a_malformed_oldest_id_skips_the_age_instead_of_guessing() {
        let sample = sample_from(3, Some(0), Some("garbage"), 1_700_000_000_000);
        assert_eq!(sample.pel_depth, 3);
        assert_eq!(sample.oldest_pending_age_seconds, 0);
    }

    #[test]
    fn a_concrete_pel_reports_the_oldest_entrys_age() {
        let sample = sample_from(4, Some(0), Some("1699999990000-0"), 1_700_000_000_000);
        assert_eq!(sample.pel_depth, 4);
        assert_eq!(sample.oldest_pending_age_seconds, 10);
    }

    #[test]
    fn unknown_server_lag_stays_absent_instead_of_becoming_zero() {
        let sample = sample_from(0, None, None, 1_700_000_000_000);
        assert_eq!(sample.lag, None);
    }
}
