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

/// The Valkey half of the pair: [`read_group`] reads, [`emit`] publishes.
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

// ---------------------------------------------------------------------------
// The whole-stream view (Wave 52 lane R4 `p52-reconciliation-jobs`)
// ---------------------------------------------------------------------------
//
// Why this exists beside `read_group`: that probe answers "how is MY group
// doing" and runs inside that group's own consumer loop, so a group whose
// consumer is not running produces no sample at all — the one stall an operator
// most needs to see. This pair answers "how does every group on the stream
// stand", from outside any consumer, so a stopped consumer's backlog is still
// visible and a group nobody in this build knows about cannot hide.
//
// The second thing it adds is the floor. Redis reports `lag = null` once a
// group's position has fallen behind entries that were trimmed away, and
// `read_group` reports that as absence — which is correct for a lag gauge and
// wrong for the question "did this group lose events". So the floor comparison
// is explicit: a position older than the stream's first surviving entry means
// entries that group never processed are gone, and no backfill of the stream can
// restore them. That is the number `reconcile::search_index_backfill` exists to
// answer for the index.

/// How far one group is from the stream's bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupGap {
    /// The group name.
    pub group: String,
    /// Entries after the group's position that are still in the stream. The
    /// server's own count when it states one, otherwise a bounded `XRANGE`
    /// count of the surviving tail (so a trimmed stream still yields a number
    /// rather than silence).
    pub gap_entries: u64,
    /// True when the group's position is behind the stream's first surviving
    /// entry: it never processed entries that have since been trimmed away.
    /// Those are unrecoverable from the stream — only a rebuild repairs them.
    pub lost: bool,
}

/// The ceiling on a fallback `XRANGE` count.
///
/// A gap larger than this is reported as this number. A gauge that says "at
/// least a thousand behind" is honest; one that walks a million entries to say
/// "a million" is an outage of its own. Chosen to be an order of magnitude above
/// any lag worth alerting on.
pub const GAP_COUNT_CAP: u64 = 1_000;

/// The id Valkey gives a group that has never been delivered anything.
const NIL_GROUP_ID: &str = "0-0";

/// The `(ms, seq)` pair a stream id encodes.
fn id_parts(id: &str) -> Option<(u64, u64)> {
    let (ms, seq) = id.split_once('-')?;
    Some((ms.parse().ok()?, seq.parse().ok()?))
}

/// Whether `a` is strictly older than `b`.
///
/// Compared as the two integers the id encodes, never as text: `10-0` sorts
/// before `9-0` as a string, and a lexicographic comparison here would report a
/// group as caught up exactly when it is far behind. An unparseable id returns
/// `false` — an unknown position is not evidence of loss.
fn id_is_older(a: &str, b: &str) -> bool {
    match (id_parts(a), id_parts(b)) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    }
}

/// The gap arithmetic, split from the round trips so the two bugs that matter —
/// "a trimmed tail reads as caught up" and "a never-read group reads as lost" —
/// are pinned by tests that need no server.
///
/// `lag` is what `XINFO GROUPS` stated. `surviving_after` is the caller's
/// `XRANGE` count, only consulted when the server declined to compute a lag.
#[must_use]
pub fn classify(
    lag: Option<u64>,
    last_delivered_id: &str,
    first_entry_id: Option<&str>,
    surviving_after: u64,
) -> (u64, bool) {
    // A group that has never been delivered anything is behind the floor by
    // construction — it was created at 0 and has not read yet. That is lag, not
    // loss, and calling it loss would page someone about a group that is about
    // to work.
    let lost = lag.is_none()
        && last_delivered_id != NIL_GROUP_ID
        && first_entry_id.is_some_and(|first| id_is_older(last_delivered_id, first));
    let gap = lag.unwrap_or(surviving_after);
    (gap.min(GAP_COUNT_CAP), lost)
}

/// Every group on the stream, measured against the stream's bounds.
///
/// One `XINFO STREAM` and one `XINFO GROUPS` per call, plus an `XRANGE` per
/// group whose lag the server cannot compute. `None` when the stream cannot be
/// read at all — the absence rule `read_group` already follows: a probe that
/// cannot reach Valkey emits nothing rather than a fabricated zero.
pub async fn read_gaps(conn: &mut MultiplexedConnection) -> Option<Vec<GroupGap>> {
    let stream: redis::streams::StreamInfoStreamReply = match conn.xinfo_stream(EVENT_STREAM).await
    {
        Ok(reply) => reply,
        Err(error) => {
            tracing::debug!(%error, "consumer gap: xinfo stream failed");
            return None;
        }
    };
    let first_entry = (!stream.first_entry.id.is_empty()).then_some(stream.first_entry.id.as_str());

    let groups: redis::streams::StreamInfoGroupsReply = match conn.xinfo_groups(EVENT_STREAM).await
    {
        Ok(reply) => reply,
        Err(error) => {
            tracing::debug!(%error, "consumer gap: xinfo groups failed");
            return None;
        }
    };

    let mut gaps = Vec::with_capacity(groups.groups.len());
    for info in groups.groups {
        let lag = info.lag.map(|l| l as u64);
        // Only pay for the count when the server withheld one: the common case
        // is a server that states its lag.
        let surviving_after = if lag.is_some() {
            0
        } else {
            match conn
                .xrange_count::<&str, String, &str, u64, redis::streams::StreamRangeReply>(
                    EVENT_STREAM,
                    format!("({}", info.last_delivered_id),
                    "+",
                    GAP_COUNT_CAP + 1,
                )
                .await
            {
                Ok(reply) => reply.ids.len() as u64,
                // A malformed exclusive start (a group id that is not an id)
                // would fail the whole probe; the group then reports no gap
                // rather than a guessed one.
                Err(error) => {
                    tracing::debug!(%error, group = %info.name, "consumer gap: xrange failed");
                    0
                }
            }
        };
        let (gap_entries, lost) =
            classify(lag, &info.last_delivered_id, first_entry, surviving_after);
        gaps.push(GroupGap {
            group: info.name,
            gap_entries,
            lost,
        });
    }
    gaps.sort_by(|a, b| a.group.cmp(&b.group));
    Some(gaps)
}

/// Emits one group's gap and loss flag. Zero is a true reading here — a group
/// with nothing undelivered is caught up — which is why the series is emitted
/// unconditionally per group rather than only when non-zero.
pub fn emit_gap(gap: &GroupGap) {
    core_otel::metrics::consumer_gap(&gap.group, gap.gap_entries, gap.lost);
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

    // --- the whole-stream gap view (Wave 52 lane R4) ----------------------

    #[test]
    fn a_server_stated_lag_is_used_as_is_and_costs_no_extra_read() {
        // No XRANGE count was taken, so the surviving count is 0; the stated lag
        // wins anyway.
        assert_eq!(classify(Some(42), "100-0", Some("50-0"), 0), (42, false));
    }

    /// The defect this pass exists for: Redis withholds `lag` once the group's
    /// position is under the stream floor, and the probe that reports absence
    /// leaves the worst case invisible.
    #[test]
    fn a_position_under_the_floor_is_flagged_lost_and_still_counts_its_gap() {
        // Group last delivered 10-0; the stream now starts at 50-0, so entries
        // 11..49 were trimmed before the group ever saw them.
        assert_eq!(
            classify(None, "10-0", Some("50-0"), 7),
            (7, true),
            "the surviving tail is the gap and the flag says the rest is gone"
        );
    }

    /// A group created at 0 (a test fixture, or an operator's `XGROUP CREATE …
    /// 0`) is behind the floor by construction but has lost nothing: it simply
    /// has not read yet.
    #[test]
    fn a_never_delivered_group_is_behind_the_floor_without_having_lost_anything() {
        assert_eq!(classify(None, "0-0", Some("50-0"), 12), (12, false));
    }

    /// Ids compare as the integers they encode, not as text: as strings `10-0`
    /// sorts before `9-0`, which would report a group that is thousands of
    /// entries behind as caught up.
    #[test]
    fn ids_compare_by_their_encoded_milliseconds_not_lexicographically() {
        assert!(id_is_older("9-0", "10-0"));
        assert!(!id_is_older("10-0", "9-0"));
        assert!(id_is_older("10-0", "10-1"), "the sequence breaks the tie");
        assert!(!id_is_older("10-0", "10-0"));
    }

    #[test]
    fn an_unparseable_position_is_not_evidence_of_loss() {
        assert_eq!(classify(None, "garbage", Some("50-0"), 3), (3, false));
        assert_eq!(classify(None, "10-0", None, 3), (3, false));
    }

    #[test]
    fn a_huge_fallback_count_is_reported_at_the_cap_rather_than_walked() {
        assert_eq!(
            classify(None, "10-0", Some("50-0"), 10_000),
            (GAP_COUNT_CAP, true)
        );
    }
}
