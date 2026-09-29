//! Multi-timer with segments, single-active-timer workflow, and time entries.
//! Timer state is server-authoritative: clients render optimistically and
//! reconcile, but every segment boundary is recorded here with its reason.

use crate::error::DomainError;
use crate::ids::{OrgId, PhaseId, TicketId, TimeEntryId, TimerSessionId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Why a timer segment ended. Preserved forever — the pause reason is
/// billable-work metadata, not a transient UI state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// Approval lifecycle of a time entry (Wave 14 lane 1): `draft` entries
/// are still being worked on, `submitted` waits in the approver queue,
/// `approved` entries are the only ones billing reads may consume, and
/// `rejected` entries carry a typed rejection reason until reopened.
/// Entries consumed by an invoice are additionally LOCKED (enforced by
/// the persistence layer) regardless of status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Draft,
    Submitted,
    Approved,
    Rejected,
}

impl ApprovalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            ApprovalStatus::Draft => "draft",
            ApprovalStatus::Submitted => "submitted",
            ApprovalStatus::Approved => "approved",
            ApprovalStatus::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "draft" => ApprovalStatus::Draft,
            "submitted" => ApprovalStatus::Submitted,
            "approved" => ApprovalStatus::Approved,
            "rejected" => ApprovalStatus::Rejected,
            _ => return None,
        })
    }
}

/// Closed rejection vocabulary (Wave 14 lane 1): approvers reject with a
/// machine-readable reason code, never prose-only. `other` may carry the
/// free-text explanation on the decision record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TimeRejectionReason {
    MissingNotes,
    WrongTicket,
    IncorrectDuration,
    DuplicateEntry,
    NonBillableWork,
    PolicyViolation,
    Other,
}

impl TimeRejectionReason {
    pub const ALL: &'static [TimeRejectionReason] = &[
        TimeRejectionReason::MissingNotes,
        TimeRejectionReason::WrongTicket,
        TimeRejectionReason::IncorrectDuration,
        TimeRejectionReason::DuplicateEntry,
        TimeRejectionReason::NonBillableWork,
        TimeRejectionReason::PolicyViolation,
        TimeRejectionReason::Other,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            TimeRejectionReason::MissingNotes => "missing_notes",
            TimeRejectionReason::WrongTicket => "wrong_ticket",
            TimeRejectionReason::IncorrectDuration => "incorrect_duration",
            TimeRejectionReason::DuplicateEntry => "duplicate_entry",
            TimeRejectionReason::NonBillableWork => "non_billable_work",
            TimeRejectionReason::PolicyViolation => "policy_violation",
            TimeRejectionReason::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        TimeRejectionReason::ALL
            .iter()
            .copied()
            .find(|r| r.as_str() == s)
    }
}

/// One approval-lifecycle transition, applied by
/// [`TimeEntry::apply_approval_action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAction {
    Submit,
    Approve,
    Reject,
    Reopen,
}

/// The audit-trail action of a [`TimeEntryApprovalRecord`] (append-only
/// decision trail; differs from [`ApprovalAction`] in that it names the
/// recorded past, not the requested transition).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TimeApprovalAction {
    Submitted,
    Approved,
    Rejected,
    Reopened,
}

impl TimeApprovalAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            TimeApprovalAction::Submitted => "submitted",
            TimeApprovalAction::Approved => "approved",
            TimeApprovalAction::Rejected => "rejected",
            TimeApprovalAction::Reopened => "reopened",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "submitted" => TimeApprovalAction::Submitted,
            "approved" => TimeApprovalAction::Approved,
            "rejected" => TimeApprovalAction::Rejected,
            "reopened" => TimeApprovalAction::Reopened,
            _ => return None,
        })
    }
}

/// One append-only approval decision (the audit row behind the queue's
/// history view). `seq` is monotonic per entry — the client's ordering key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeEntryApprovalRecord {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub time_entry_id: TimeEntryId,
    pub seq: i32,
    pub action: TimeApprovalAction,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub actor_id: UserId,
    /// Typed [`TimeRejectionReason`] wire code on rejections, else None.
    pub reason_code: Option<String>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A queue/history projection: the entry plus its approval metadata
/// (who/when submitted, who/when decided).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeEntryQueueItem {
    // The wire carries the FULL TimeEntry (not just its uuid — the earlier
    // value_type = uuid::Uuid annotation lied; fixed Wave 16 lane 9).
    pub entry: TimeEntry,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub submitted_by: Option<UserId>,
    pub submitted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub decided_by: Option<UserId>,
    pub decided_at: Option<DateTime<Utc>>,
    /// True once the entry is consumed by an invoice — frozen forever.
    pub locked: bool,
}

impl ApprovalStatus {
    /// The pure approval state machine: draft → submitted → approved |
    /// rejected → (reopen) → draft. Approved is terminal until an invoice
    /// consumes the entry (the lock, enforced by the persistence layer).
    pub fn transition(self, action: ApprovalAction) -> Result<ApprovalStatus, DomainError> {
        use ApprovalAction::*;
        let next = match (self, action) {
            (ApprovalStatus::Draft, Submit) | (ApprovalStatus::Rejected, Submit) => {
                ApprovalStatus::Submitted
            }
            (ApprovalStatus::Submitted, Approve) => ApprovalStatus::Approved,
            (ApprovalStatus::Submitted, Reject) => ApprovalStatus::Rejected,
            (ApprovalStatus::Rejected, Reopen) => ApprovalStatus::Draft,
            (ApprovalStatus::Approved, _) => {
                return Err(DomainError::Validation(
                    "entry is approved and locked".into(),
                ))
            }
            (ApprovalStatus::Submitted, ApprovalAction::Reopen)
            | (ApprovalStatus::Draft, _)
            | (ApprovalStatus::Submitted, ApprovalAction::Submit)
            | (ApprovalStatus::Rejected, ApprovalAction::Approve)
            | (ApprovalStatus::Rejected, ApprovalAction::Reject) => {
                return Err(DomainError::Validation(format!(
                    "cannot {:?} an entry in state {}",
                    action,
                    self.as_str()
                )))
            }
        };
        Ok(next)
    }
}

/// One user's timer against one ticket. At most one session per user is
/// ever running — enforced by [`TimerSession::start_switch`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerSession {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: TimerSessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
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
            return Err(DomainError::InvalidDuration(
                (now - seg.started_at).num_seconds(),
            ));
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
                return Err(DomainError::InvalidDuration(
                    (now - seg.started_at).num_seconds(),
                ));
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
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: TimeEntryId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// The ticket this entry bills through. `None` ONLY on project-scoped
    /// entries (Wave 39 lane 4, report P5): the entry scopes to
    /// [`phase_id`](Self::phase_id) instead and bills through the phase
    /// project's linked contract — project work pays without a ticket
    /// detour. Exactly one of (ticket_id, phase_id) is present; the DB
    /// CHECK mirrors it.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    /// The delivery phase this entry scopes to (project-scoped entries
    /// only; implies the project). `None` on ticket-scoped entries.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub phase_id: Option<PhaseId>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Worked seconds — authoritative, derived on write, never recomputed.
    pub minutes: i64,
    pub billable: bool,
    pub notes: Option<String>,
    pub work_type: Option<String>,
    /// Origin: timer-derived entries keep their session for audit.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub timer_session_id: Option<TimerSessionId>,
    /// Approval lifecycle state. `None` on legacy snapshots (pre-approval
    /// rows and old event payloads) and treated as [`ApprovalStatus::Draft`]
    /// by the workflow; new writes always carry the status.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub approval_status: Option<ApprovalStatus>,
    /// Typed rejection reason (the [`TimeRejectionReason`] wire code) when
    /// the entry was rejected; cleared on resubmit.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub approval_reason_code: Option<String>,
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
            ticket_id: Some(session.ticket_id),
            phase_id: None,
            user_id: session.user_id,
            started_at: session.started_at,
            ended_at: now,
            minutes: (total + 59) / 60, // round up to whole minutes
            billable,
            notes,
            work_type: None,
            timer_session_id: Some(session.id),
            approval_status: Some(ApprovalStatus::Draft),
            approval_reason_code: None,
            created_at: now,
        })
    }

    /// The effective approval status: legacy `None` reads as draft.
    pub fn approval(&self) -> ApprovalStatus {
        self.approval_status.unwrap_or(ApprovalStatus::Draft)
    }

    /// Applies one approval transition to this entry, keeping the typed
    /// rejection reason in sync (set on reject, cleared on submit/approve/
    /// reopen). Pure — persistence and audit rows belong to the caller.
    pub fn apply_approval_action(
        &mut self,
        action: ApprovalAction,
        reason_code: Option<TimeRejectionReason>,
    ) -> Result<ApprovalStatus, DomainError> {
        if action == ApprovalAction::Reject && reason_code.is_none() {
            return Err(DomainError::Validation(
                "a rejection requires a typed reason code".into(),
            ));
        }
        if let Some(code) = reason_code {
            if action != ApprovalAction::Reject {
                return Err(DomainError::Validation(
                    "reason codes only apply to rejections".into(),
                ));
            }
            let _ = code;
        }
        let next = self.approval().transition(action)?;
        self.approval_status = Some(next);
        self.approval_reason_code = match action {
            ApprovalAction::Reject => reason_code.map(|c| c.as_str().to_string()),
            _ => None,
        };
        Ok(next)
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
        s.pause(
            t0 + Duration::minutes(10),
            Some("on call with vendor".into()),
        )
        .unwrap();
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

    #[test]
    fn approval_lifecycle_happy_path() {
        let (t0, org, user, ticket) = base();
        let mut s = TimerSession::start(org, user, ticket, t0);
        s.stop(t0 + Duration::minutes(30), None).unwrap();
        let mut e = TimeEntry::from_session(&s, t0 + Duration::minutes(30), None, true).unwrap();
        assert_eq!(e.approval(), ApprovalStatus::Draft);
        e.apply_approval_action(ApprovalAction::Submit, None)
            .unwrap();
        assert_eq!(e.approval(), ApprovalStatus::Submitted);
        e.apply_approval_action(ApprovalAction::Approve, None)
            .unwrap();
        assert_eq!(e.approval(), ApprovalStatus::Approved);
        // Approved is terminal — no further transition is legal.
        assert!(e
            .apply_approval_action(ApprovalAction::Submit, None)
            .is_err());
        assert!(e
            .apply_approval_action(ApprovalAction::Reject, Some(TimeRejectionReason::Other))
            .is_err());
        assert!(e
            .apply_approval_action(ApprovalAction::Reopen, None)
            .is_err());
    }

    #[test]
    fn rejection_requires_typed_reason_and_reopen_clears_it() {
        let (t0, org, user, ticket) = base();
        let mut s = TimerSession::start(org, user, ticket, t0);
        s.stop(t0 + Duration::minutes(10), None).unwrap();
        let mut e = TimeEntry::from_session(&s, t0 + Duration::minutes(10), None, true).unwrap();
        e.apply_approval_action(ApprovalAction::Submit, None)
            .unwrap();
        // A rejection without a typed reason code is refused.
        assert!(e
            .apply_approval_action(ApprovalAction::Reject, None)
            .is_err());
        e.apply_approval_action(
            ApprovalAction::Reject,
            Some(TimeRejectionReason::MissingNotes),
        )
        .unwrap();
        assert_eq!(e.approval(), ApprovalStatus::Rejected);
        assert_eq!(e.approval_reason_code.as_deref(), Some("missing_notes"));
        // Reason codes never ride non-rejection actions.
        assert!(e
            .apply_approval_action(ApprovalAction::Reopen, Some(TimeRejectionReason::Other))
            .is_err());
        e.apply_approval_action(ApprovalAction::Reopen, None)
            .unwrap();
        assert_eq!(e.approval(), ApprovalStatus::Draft);
        assert!(e.approval_reason_code.is_none());
    }

    #[test]
    fn rejected_entry_can_be_resubmitted_directly() {
        let (t0, org, user, ticket) = base();
        let mut s = TimerSession::start(org, user, ticket, t0);
        s.stop(t0 + Duration::minutes(5), None).unwrap();
        let mut e = TimeEntry::from_session(&s, t0 + Duration::minutes(5), None, true).unwrap();
        e.apply_approval_action(ApprovalAction::Submit, None)
            .unwrap();
        e.apply_approval_action(
            ApprovalAction::Reject,
            Some(TimeRejectionReason::WrongTicket),
        )
        .unwrap();
        e.apply_approval_action(ApprovalAction::Submit, None)
            .unwrap();
        assert_eq!(e.approval(), ApprovalStatus::Submitted);
        assert!(e.approval_reason_code.is_none());
    }

    #[test]
    fn rejection_reason_vocabulary_is_closed() {
        assert_eq!(
            TimeRejectionReason::parse("missing_notes"),
            Some(TimeRejectionReason::MissingNotes)
        );
        assert_eq!(TimeRejectionReason::parse("nonsense"), None);
        assert_eq!(
            ApprovalStatus::parse("approved"),
            Some(ApprovalStatus::Approved)
        );
        assert_eq!(ApprovalStatus::parse("approved "), None);
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
