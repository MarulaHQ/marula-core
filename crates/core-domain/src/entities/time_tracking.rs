//! Multi-timer with segments, single-active-timer workflow, and time entries.
//! Timer state is server-authoritative: clients render optimistically and
//! reconcile, but every segment boundary is recorded here with its reason.

use crate::error::DomainError;
use crate::ids::{OrgId, TicketId, TimeEntryId, TimerSessionId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Why a timer segment ended. Preserved forever — the pause reason is
/// billable-work metadata, not a transient UI state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SegmentEnd {
    Stopped,
    Paused,
    /// Auto-paused because the user switched to another ticket
    /// (single-active-timer workflow).
    Switched,
}

impl SegmentEnd {
    pub const fn as_str(self) -> &'static str {
        match self {
            SegmentEnd::Stopped => "stopped",
            SegmentEnd::Paused => "paused",
            SegmentEnd::Switched => "switched",
        }
    }
}

/// A contiguous running interval within a timer session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerSegment {
    pub started_at: DateTime<Utc>,
    /// None while this segment is running.
    pub ended_at: Option<DateTime<Utc>>,
    pub end_kind: Option<SegmentEnd>,
    /// Free-text reason recorded at pause time ("on call with vendor").
    pub reason: Option<String>,
}

impl TimerSegment {
    pub fn elapsed_secs(&self, now: DateTime<Utc>) -> i64 {
        let end = self.ended_at.unwrap_or(now);
        (end - self.started_at).num_seconds().max(0)
    }
}

/// One user's timer against one ticket. At most one session per user is
/// ever running — enforced by [`TimerSession::start_switch`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerSession {
    pub id: TimerSessionId,
    pub org_id: OrgId,
    pub user_id: UserId,
    pub ticket_id: TicketId,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub segments: Vec<TimerSegment>,
}

impl TimerSession {
    /// Starts a new session; `now` must be >= epoch sanity.
    pub fn start(org_id: OrgId, user_id: UserId, ticket_id: TicketId, now: DateTime<Utc>) -> Self {
        Self {
            id: uuid::Uuid::now_v7(),
            org_id,
            user_id,
            ticket_id,
            started_at: now,
            stopped_at: None,
            segments: vec![TimerSegment {
                started_at: now,
                ended_at: None,
                end_kind: None,
                reason: None,
            }],
        }
    }

    pub fn is_running(&self) -> bool {
        self.stopped_at.is_none() && self.segments.last().is_some_and(|s| s.ended_at.is_none())
    }

    /// Pauses the running segment, recording the reason.
    pub fn pause(&mut self, now: DateTime<Utc>, reason: Option<String>) -> Result<(), DomainError> {
        if !self.is_running() {
            return Err(DomainError::Validation("timer is not running".into()));
        }
        let seg = self.segments.last_mut().expect("running segment");
        if seg.started_at > now {
            return Err(DomainError::InvalidDuration((now - seg.started_at).num_seconds()));
        }
        seg.ended_at = Some(now);
        seg.end_kind = Some(SegmentEnd::Paused);
        seg.reason = reason;
        Ok(())
    }

    /// Resumes with a fresh segment after a pause.
    pub fn resume(&mut self, now: DateTime<Utc>) -> Result<(), DomainError> {
        if self.stopped_at.is_some() {
            return Err(DomainError::Validation("timer already stopped".into()));
        }
        if self.is_running() {
            return Err(DomainError::Validation("timer already running".into()));
        }
        self.segments.push(TimerSegment {
            started_at: now,
            ended_at: None,
            end_kind: None,
            reason: None,
        });
        Ok(())
    }

    /// Stops the session for good, closing any running segment.
    pub fn stop(&mut self, now: DateTime<Utc>, reason: Option<String>) -> Result<(), DomainError> {
        if self.stopped_at.is_some() {
            return Err(DomainError::Validation("timer already stopped".into()));
        }
        if self.is_running() {
            let seg = self.segments.last_mut().expect("running segment");
            if seg.started_at > now {
                return Err(DomainError::InvalidDuration((now - seg.started_at).num_seconds()));
            }
            seg.ended_at = Some(now);
            seg.end_kind = Some(SegmentEnd::Stopped);
            if seg.reason.is_none() {
                seg.reason = reason;
            }
        }
        self.stopped_at = Some(now);
        Ok(())
    }

    /// Total accumulated seconds across all segments up to `now`.
    pub fn total_elapsed_secs(&self, now: DateTime<Utc>) -> i64 {
        self.segments.iter().map(|s| s.elapsed_secs(now)).sum()
    }

    /// The single-active-timer rule: starting work on `new_ticket` while
    /// `self` runs auto-pauses this session with [`SegmentEnd::Switched`].
    /// The paused session remains in the user's docked working set.
    pub fn start_switch(
        current: &mut Self,
        org_id: OrgId,
        user_id: UserId,
        new_ticket: TicketId,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if current.is_running() {
            current.pause(now, None)?;
            if let Some(seg) = current.segments.last_mut() {
                seg.end_kind = Some(SegmentEnd::Switched);
            }
        }
        Ok(Self::start(org_id, user_id, new_ticket, now))
    }
}

/// A durable, reviewable record of worked time. Created from a timer session
/// or entered manually; subject to bulk review and approval workflows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeEntry {
    pub id: TimeEntryId,
    pub org_id: OrgId,
    pub ticket_id: TicketId,
    pub user_id: UserId,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Worked seconds — authoritative, derived on write, never recomputed.
    pub minutes: i64,
    pub billable: bool,
    pub notes: Option<String>,
    pub work_type: Option<String>,
    /// Origin: timer-derived entries keep their session for audit.
    pub timer_session_id: Option<TimerSessionId>,
    pub created_at: DateTime<Utc>,
}

impl TimeEntry {
    pub fn from_session(
        session: &TimerSession,
        now: DateTime<Utc>,
        notes: Option<String>,
        billable: bool,
    ) -> Result<Self, DomainError> {
        let total = session.total_elapsed_secs(now);
        if total <= 0 {
            return Err(DomainError::InvalidDuration(total));
        }
        Ok(Self {
            id: uuid::Uuid::now_v7(),
            org_id: session.org_id,
            ticket_id: session.ticket_id,
            user_id: session.user_id,
            started_at: session.started_at,
            ended_at: now,
            minutes: (total + 59) / 60, // round up to whole minutes
            billable,
            notes,
            work_type: None,
            timer_session_id: Some(session.id),
            created_at: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use proptest::prelude::*;

    fn base() -> (DateTime<Utc>, OrgId, UserId, TicketId) {
        (
            Utc::now(),
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
        )
    }

    #[test]
    fn single_active_timer_switch_preserves_elapsed() {
        let (t0, org, user, ticket_a) = base();
        let ticket_b = uuid::Uuid::now_v7();
        let mut a = TimerSession::start(org, user, ticket_a, t0);
        let t1 = t0 + Duration::minutes(30);
        let b = TimerSession::start_switch(&mut a, org, user, ticket_b, t1).unwrap();
        assert!(!a.is_running());
        assert_eq!(a.segments[0].end_kind, Some(SegmentEnd::Switched));
        assert_eq!(a.total_elapsed_secs(t1), 30 * 60);
        assert!(b.is_running());
        assert_eq!(b.ticket_id, ticket_b);
    }

    #[test]
    fn pause_reason_preserved_across_resume() {
        let (t0, org, user, ticket) = base();
        let mut s = TimerSession::start(org, user, ticket, t0);
        s.pause(t0 + Duration::minutes(10), Some("on call with vendor".into())).unwrap();
        s.resume(t0 + Duration::minutes(15)).unwrap();
        assert_eq!(s.segments[0].reason.as_deref(), Some("on call with vendor"));
        assert_eq!(s.total_elapsed_secs(t0 + Duration::minutes(20)), 15 * 60);
    }

    #[test]
    fn cannot_pause_twice_or_resume_running() {
        let (t0, org, user, ticket) = base();
        let mut s = TimerSession::start(org, user, ticket, t0);
        assert!(s.resume(t0).is_err());
        s.pause(t0 + Duration::minutes(1), None).unwrap();
        assert!(s.pause(t0 + Duration::minutes(2), None).is_err());
    }

    #[test]
    fn time_entry_rounds_up_to_minutes() {
        let (t0, org, user, ticket) = base();
        let mut s = TimerSession::start(org, user, ticket, t0);
        s.stop(t0 + Duration::seconds(61), None).unwrap();
        let e = TimeEntry::from_session(&s, t0 + Duration::seconds(61), None, true).unwrap();
        assert_eq!(e.minutes, 2);
    }

    proptest! {
        #[test]
        fn elapsed_is_monotonic(mins in 1..600_i64) {
            let (t0, org, user, ticket) = base();
            let s = TimerSession::start(org, user, ticket, t0);
            let later = t0 + Duration::minutes(mins);
            prop_assert!(s.total_elapsed_secs(later) >= s.total_elapsed_secs(t0));
        }
    }
}
