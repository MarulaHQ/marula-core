//! SLA clock engine — pure escalation-ladder evaluation and budget math.
//!
//! Purpose: the decision core the worker's periodic sweep calls: where does
//! a ticket's SLA clock stand (consumed vs budget, in BUSINESS time), and
//! which ladder steps are due RIGHT NOW given what already fired. Fire-once
//! idempotency is a function input (the fired set), so repeated evaluations
//! under retries/clock skew can never produce a duplicate verdict.
//!
//! Non-purpose: no I/O, no clocks read from the host, no notification
//! creation — verdicts become worker jobs that flow through core-api.

use crate::entities::{BusinessCalendar, TicketSlaState};
use chrono::{DateTime, Utc};
use std::collections::HashSet;

/// Which SLA clock a ladder step or verdict refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlaTarget {
    FirstResponse,
    Resolution,
}

impl SlaTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstResponse => "first_response",
            Self::Resolution => "resolution",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "first_response" => Some(Self::FirstResponse),
            "resolution" => Some(Self::Resolution),
            _ => None,
        }
    }

    /// The due-date column this target reads from.
    pub fn due_at(self, state: &TicketSlaState) -> Option<DateTime<Utc>> {
        match self {
            Self::FirstResponse => state.first_response_due,
            Self::Resolution => state.resolution_due,
        }
    }
}

/// Ladder step trigger, bound to notification-center kinds that already
/// exist (0007/0012) so preference + quiet-hours machinery applies as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepTrigger {
    Risk,
    Breach,
    Escalate,
}

impl StepTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Risk => "risk",
            Self::Breach => "breach",
            Self::Escalate => "escalate",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "risk" => Some(Self::Risk),
            "breach" => Some(Self::Breach),
            "escalate" => Some(Self::Escalate),
            _ => None,
        }
    }

    /// The notification kind this trigger fans out into.
    pub const fn notification_kind(self) -> crate::entities::NotificationKind {
        match self {
            Self::Risk => crate::entities::NotificationKind::SlaRisk,
            Self::Breach => crate::entities::NotificationKind::SlaBreach,
            Self::Escalate => crate::entities::NotificationKind::Escalation,
        }
    }
}

/// One enabled ladder step, storage-shaped ids attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LadderStep {
    pub id: uuid::Uuid,
    pub step_index: i32,
    pub trigger: StepTrigger,
    /// Fires when consumed budget reaches this percent.
    pub at_pct: i64,
}

/// A verdict the evaluator owes the outside world: fire this step for this
/// target exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub step_id: uuid::Uuid,
    pub step_index: i32,
    pub trigger: StepTrigger,
    pub target: SlaTarget,
    pub at_pct: i64,
}

/// Where one SLA clock stands at an instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockPosition {
    /// Business seconds actually consumed (pauses banked away).
    pub consumed_secs: i64,
    /// Total business-seconds budget (`None` due ⇒ no position at all).
    pub budget_secs: i64,
    pub pct: i64,
}

/// Computes business-time consumption for one clock.
///
/// `banked_paused_secs` are completed Waiting stretches; while a pause is
/// OPEN (`paused_since` set) its elapsed span is excluded on top. Both
/// subtractions use the same calendar so maintenance windows inside pauses
/// behave identically to those inside runs.
pub fn position(
    calendar: &BusinessCalendar,
    created_at: DateTime<Utc>,
    now: DateTime<Utc>,
    due: Option<DateTime<Utc>>,
    banked_paused_secs: i64,
    paused_since: Option<DateTime<Utc>>,
) -> Option<ClockPosition> {
    let due = due?;
    let total = calendar.elapsed_business_secs(created_at, now);
    let open_pause = paused_since
        .filter(|s| *s < now)
        .map(|s| calendar.elapsed_business_secs(s, now))
        .unwrap_or(0);
    let consumed_secs = (total - banked_paused_secs - open_pause).max(0);
    let budget_secs = calendar.elapsed_business_secs(created_at, due).max(0);
    let pct = if budget_secs <= 0 {
        // Degenerate budget (all downtime): any consumption is breach.
        i64::from(consumed_secs > 0) * 100
    } else {
        (consumed_secs.saturating_mul(100)) / budget_secs
    };
    Some(ClockPosition {
        consumed_secs,
        budget_secs,
        pct,
    })
}

/// Evaluates the ladder: every enabled step whose threshold is met for a
/// live target and NOT present in `already_fired` yields exactly one
/// verdict, ordered by threshold then index then target (deterministic so
/// downstream jobs enqueue in a stable order).
pub fn evaluate_ladder(
    steps: &[LadderStep],
    already_fired: &HashSet<(uuid::Uuid, SlaTarget)>,
    first_response: Option<ClockPosition>,
    resolution: Option<ClockPosition>,
) -> Vec<Verdict> {
    let mut out = Vec::new();
    for step in steps {
        for (target, position) in [
            (SlaTarget::FirstResponse, first_response),
            (SlaTarget::Resolution, resolution),
        ] {
            let Some(pos) = position else {
                continue;
            };
            if pos.pct < step.at_pct {
                continue;
            }
            if already_fired.contains(&(step.id, target)) {
                continue;
            }
            out.push(Verdict {
                step_id: step.id,
                step_index: step.step_index,
                trigger: step.trigger,
                target,
                at_pct: step.at_pct,
            });
        }
    }
    out.sort_by_key(|v| (v.at_pct, v.step_index, v.target.as_str() == "resolution"));
    out
}

/// Convenience: positions for both targets of a ticket's SLA state under
/// its policy calendar. First response drops out once responded.
/// `created_at` anchors both budgets (the worker carries it from tickets).
pub fn ticket_positions(
    state: &TicketSlaState,
    calendar: &BusinessCalendar,
    created_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> (Option<ClockPosition>, Option<ClockPosition>) {
    let fr = if state.first_responded_at.is_some() {
        None
    } else {
        position(
            calendar,
            created_at,
            now,
            state.first_response_due,
            state.paused_business_secs,
            state.paused_since,
        )
    };
    let resolution = position(
        calendar,
        created_at,
        now,
        state.resolution_due,
        state.paused_business_secs,
        state.paused_since,
    );
    (fr, resolution)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::BusinessCalendar;
    use crate::status::TicketStatusCategory;
    use chrono::Duration;
    use proptest::prelude::*;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
            .and_utc()
    }

    fn weekday_calendar() -> BusinessCalendar {
        BusinessCalendar {
            name: "9to5 M-F".into(),
            timezone: "UTC".into(),
            windows: [
                Some(("09:00".into(), "17:00".into())),
                Some(("09:00".into(), "17:00".into())),
                Some(("09:00".into(), "17:00".into())),
                Some(("09:00".into(), "17:00".into())),
                Some(("09:00".into(), "17:00".into())),
                None,
                None,
            ],
            holidays: vec![],
            maintenance: vec![],
        }
    }

    fn ladder() -> Vec<LadderStep> {
        vec![
            LadderStep {
                id: uuid::Uuid::from_u128(1),
                step_index: 0,
                trigger: StepTrigger::Risk,
                at_pct: 75,
            },
            LadderStep {
                id: uuid::Uuid::from_u128(2),
                step_index: 1,
                trigger: StepTrigger::Breach,
                at_pct: 100,
            },
            LadderStep {
                id: uuid::Uuid::from_u128(3),
                step_index: 2,
                trigger: StepTrigger::Escalate,
                at_pct: 125,
            },
        ]
    }

    // --- ladder fire-once / idempotency ------------------------------------

    #[test]
    fn ladder_fires_only_above_threshold_and_never_twice() {
        let steps = ladder();
        let pos80 = ClockPosition {
            consumed_secs: 80,
            budget_secs: 100,
            pct: 80,
        };
        let fired = HashSet::new();
        let v = evaluate_ladder(&steps, &fired, None, Some(pos80));
        assert_eq!(v.len(), 1);
        assert_eq!(
            (v[0].step_id, v[0].target),
            (uuid::Uuid::from_u128(1), SlaTarget::Resolution)
        );

        // Re-evaluating WITH the verdict recorded fires nothing.
        let mut fired: HashSet<(uuid::Uuid, SlaTarget)> =
            v.iter().map(|x| (x.step_id, x.target)).collect();
        assert!(evaluate_ladder(&steps, &fired, None, Some(pos80)).is_empty());

        // Crossing breach + escalate yields both, ordered by threshold.
        let pos130 = ClockPosition {
            consumed_secs: 130,
            budget_secs: 100,
            pct: 130,
        };
        let v = evaluate_ladder(&steps, &fired, None, Some(pos130));
        assert_eq!(
            v.iter().map(|x| x.at_pct).collect::<Vec<_>>(),
            vec![100, 125]
        );
        fired.extend(v.iter().map(|x| (x.step_id, x.target)));
        assert!(evaluate_ladder(&steps, &fired, None, Some(pos130)).is_empty());
    }

    #[test]
    fn first_response_target_drops_out_after_response() {
        let pos150 = ClockPosition {
            consumed_secs: 150,
            budget_secs: 100,
            pct: 150,
        };
        // FR position present ⇒ steps evaluate against BOTH targets.
        let fr = evaluate_ladder(&ladder(), &HashSet::new(), Some(pos150), None);
        assert!(fr.iter().all(|v| v.target == SlaTarget::FirstResponse));
        // Responded ⇒ caller passes None ⇒ nothing fires even at 150%.
        assert!(evaluate_ladder(&ladder(), &HashSet::new(), None, None).is_empty());
    }

    #[test]
    fn trigger_maps_to_existing_notification_kinds() {
        assert_eq!(
            StepTrigger::Risk.notification_kind(),
            crate::entities::NotificationKind::SlaRisk
        );
        assert_eq!(
            StepTrigger::Breach.notification_kind(),
            crate::entities::NotificationKind::SlaBreach
        );
        assert_eq!(
            StepTrigger::Escalate.notification_kind(),
            crate::entities::NotificationKind::Escalation
        );
    }

    // --- position / skew ----------------------------------------------------

    #[test]
    fn open_pause_freezes_consumption_exactly() {
        let cal = weekday_calendar();
        let created = utc(2026, 5, 18, 9, 0); // Mon 09:00
        let due = Some(utc(2026, 5, 18, 17, 0)); // same-day 8h budget
                                                 // Runs one hour, pauses at 10:00, evaluated at 16:00.
        let running = position(&cal, created, utc(2026, 5, 18, 10, 0), due, 0, None).unwrap();
        assert_eq!(running.consumed_secs, 3600);
        let paused = position(
            &cal,
            created,
            utc(2026, 5, 18, 16, 0),
            due,
            0,
            Some(utc(2026, 5, 18, 10, 0)),
        )
        .unwrap();
        assert_eq!(
            paused.consumed_secs, 3600,
            "an open pause must freeze the clock"
        );
    }

    #[test]
    fn degenerate_budget_breaches_on_any_consumption() {
        let cal = weekday_calendar();
        // Due INSIDE downtime: budget computes to zero.
        let due = Some(utc(2026, 5, 18, 8, 0)); // pre-business
        let p = position(
            &cal,
            utc(2026, 5, 18, 9, 0),
            utc(2026, 5, 18, 9, 1),
            due,
            0,
            None,
        );
        assert_eq!(p.map(|x| x.pct), Some(100));
        let p0 = position(
            &cal,
            utc(2026, 5, 18, 9, 0),
            utc(2026, 5, 18, 9, 0),
            due,
            0,
            None,
        );
        assert_eq!(p0.map(|x| x.pct), Some(0));
    }

    // --- pause/resume interleavings vs an independent oracle ----------------

    struct Oracle {
        bank: i64,
        span_start: Option<DateTime<Utc>>,
    }

    impl Oracle {
        fn transition(&mut self, cal: &BusinessCalendar, waiting: bool, now: DateTime<Utc>) {
            if waiting {
                if self.span_start.is_none() {
                    self.span_start = Some(now);
                }
            } else if let Some(s) = self.span_start.take() {
                self.bank += cal.elapsed_business_secs(s, now);
            }
        }

        fn consumed(
            &self,
            cal: &BusinessCalendar,
            created: DateTime<Utc>,
            now: DateTime<Utc>,
        ) -> i64 {
            let open = self
                .span_start
                .map(|s| cal.elapsed_business_secs(s, now))
                .unwrap_or(0);
            cal.elapsed_business_secs(created, now) - self.bank - open
        }
    }

    proptest! {
        /// Clock-skew tolerance: consumption between two evaluations moves
        /// by EXACTLY the business time between the two read clocks while
        /// running — a skewed evaluator can neither invent nor destroy
        /// progress beyond that bound.
        #[test]
        fn consumption_moves_only_by_business_time_between_reads(
            base_min in 1i64..=300,
            delta_min in -45i64..=45,
        ) {
            let cal = weekday_calendar();
            let created = utc(2026, 5, 18, 9, 0);
            let due = Some(utc(2026, 5, 19, 17, 0));
            let n1 = created + Duration::minutes(base_min);
            let n2 = n1 + Duration::minutes(delta_min);

            let read = |now: DateTime<Utc>| {
                position(&cal, created, now, due, 0, None).map(|p| p.consumed_secs)
            };

            if n2 > n1 {
                let expected_gain = cal.elapsed_business_secs(n1, n2);
                prop_assert_eq!(read(n2).unwrap() - read(n1).unwrap(), expected_gain);
            } else if n2 < n1 {
                // A host reading an EARLIER clock sees less-or-equal progress
                // (monotone in the DB clock), never negative consumption.
                prop_assert!(read(n2).unwrap() <= read(n1).unwrap());
            } else {
                prop_assert_eq!(read(n1), read(n2));
            }
        }

        /// While a pause is OPEN, any two reads observe identical progress
        /// regardless of how far apart they are.
        #[test]
        fn open_pause_makes_reads_skew_immune(base_min in 1i64..=120, gap_min in 0i64..=240) {
            let cal = weekday_calendar();
            let created = utc(2026, 5, 18, 9, 0);
            let due = Some(utc(2026, 5, 20, 17, 0));
            let pause_from = created + Duration::minutes(base_min);
            let n1 = pause_from + Duration::minutes(gap_min.min(60));
            let n2 = pause_from + Duration::minutes(gap_min + 240);
            let read = |now: DateTime<Utc>| {
                position(&cal, created, now, due, 0, Some(pause_from)).unwrap().consumed_secs
            };
            prop_assert_eq!(read(n1), read(n2));
        }

        /// Random Waiting/running interleavings: the entity's banked seconds
        /// and the engine's consumption match a directly-computed oracle at
        /// EVERY step, and consumption never goes negative.
        #[test]
        fn pause_resume_interleavings_conserve_budget(
            ops in proptest::collection::vec((proptest::bool::ANY, 7u32..600), 1..24),
        ) {
            let cal = weekday_calendar();
            let created = utc(2026, 5, 18, 9, 0);
            let due = Some(utc(2026, 5, 29, 17, 0));
            let mut state_paused_since: Option<DateTime<Utc>> = None;
            let mut state_banked: i64 = 0;
            let mut oracle = Oracle { bank: 0, span_start: None };
            let mut cursor = created;

            for (waiting, minutes) in ops {
                cursor += Duration::minutes(i64::from(minutes));
                let _category = if waiting {
                    TicketStatusCategory::Waiting
                } else {
                    TicketStatusCategory::InProgress
                };
                // Mirror TicketSlaState::apply_category semantics.
                if waiting && state_paused_since.is_none() {
                    state_paused_since = Some(cursor);
                } else if !waiting {
                    if let Some(s) = state_paused_since.take() {
                        state_banked += cal.elapsed_business_secs(s, cursor);
                    }
                }
                oracle.transition(&cal, waiting, cursor);

                let pos = position(
                    &cal,
                    created,
                    cursor,
                    due,
                    state_banked,
                    state_paused_since,
                )
                .unwrap();
                prop_assert_eq!(
                    pos.consumed_secs,
                    oracle.consumed(&cal, created, cursor),
                    "engine vs oracle divergence"
                );
                prop_assert_eq!(state_banked, oracle.bank);
                prop_assert!(pos.consumed_secs >= 0);
            }
        }
    }
}
