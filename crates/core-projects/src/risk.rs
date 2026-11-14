//! Deterministic risk rules for a project plan.
//!
//! Every signal here is computed from stored facts and nothing else: no model,
//! no heuristic weighting, no clock beyond the `today` the caller passes. The
//! rules are the source of a signal's severity (ADR 0034 decision 11), so an AI
//! surface can rank and narrate them but can never invent one or upgrade one.
//!
//! Each rule names the object that caused it, so a surface can link to the task,
//! phase or project rather than describing it in prose.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// The kind of object a signal points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Project,
    Phase,
    Task,
}

/// How much a signal matters. Deterministic rules assign it; nothing else may.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Critical,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Critical => "critical",
        }
    }
}

/// A rule's stable identifier. Stable because a surface stores dismissals
/// against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rule {
    /// A dated task whose end has passed and which is not completed.
    TaskOverdue,
    /// A dated task whose start has passed and which has not begun.
    TaskNotStarted,
    /// Dated work starting soon with nobody assigned.
    UnassignedImminent,
    /// A phase has spent more of its budget than it has finished of its work.
    BurnAheadOfProgress,
    /// An active project with no update inside the window.
    NoRecentUpdate,
    /// A milestone whose date has passed and which is still open.
    MilestoneOverdue,
    /// A task that has sat blocked beyond the window.
    BlockedTooLong,
    /// A task starting later than its baseline by more than the tolerance.
    SlippedAgainstBaseline,
    /// An active project with no dated work at all.
    PlanUndated,
}

impl Rule {
    pub const fn code(self) -> &'static str {
        match self {
            Self::TaskOverdue => "task_overdue",
            Self::TaskNotStarted => "task_not_started",
            Self::UnassignedImminent => "unassigned_imminent",
            Self::BurnAheadOfProgress => "burn_ahead_of_progress",
            Self::NoRecentUpdate => "no_recent_update",
            Self::MilestoneOverdue => "milestone_overdue",
            Self::BlockedTooLong => "blocked_too_long",
            Self::SlippedAgainstBaseline => "slipped_against_baseline",
            Self::PlanUndated => "plan_undated",
        }
    }

    /// Every rule, so a caller can assert the set rather than discover it.
    pub const ALL: [Rule; 9] = [
        Self::TaskOverdue,
        Self::TaskNotStarted,
        Self::UnassignedImminent,
        Self::BurnAheadOfProgress,
        Self::NoRecentUpdate,
        Self::MilestoneOverdue,
        Self::BlockedTooLong,
        Self::SlippedAgainstBaseline,
        Self::PlanUndated,
    ];
}

/// One computed signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    pub rule: Rule,
    pub severity: Severity,
    pub subject: Subject,
    pub subject_id: Uuid,
    /// A sentence a person can act on, without internal vocabulary.
    pub message: String,
}

/// How the rules read a project.
#[derive(Debug, Clone)]
pub struct ProjectFacts {
    pub project_id: Uuid,
    pub project_status: String,
    pub project_due_date: Option<chrono::NaiveDate>,
    /// The most recent status report or update, when one exists.
    pub last_update_at: Option<DateTime<Utc>>,
    pub phases: Vec<PhaseFacts>,
    pub tasks: Vec<TaskFacts>,
}

#[derive(Debug, Clone)]
pub struct PhaseFacts {
    pub id: Uuid,
    pub status: String,
    pub is_milestone: bool,
    /// The phase's end date, when it has one.
    pub end_date: Option<chrono::NaiveDate>,
    /// Budget and consumed money, both in minor units, when a budget exists.
    pub budget_minor: Option<i64>,
    pub consumed_minor: Option<i64>,
    /// Tasks in the phase that are finished, and the total.
    pub tasks_done: i64,
    pub tasks_total: i64,
}

#[derive(Debug, Clone)]
pub struct TaskFacts {
    pub id: Uuid,
    pub status: String,
    pub phase_id: Option<Uuid>,
    pub assigned: bool,
    pub start_at: Option<DateTime<Utc>>,
    pub duration_minutes: Option<i32>,
    pub end_at: Option<DateTime<Utc>>,
    pub baseline_start_at: Option<DateTime<Utc>>,
    /// When the task entered `blocked`, approximated by its last update.
    pub blocked_since: Option<DateTime<Utc>>,
}

/// The windows the rules use. Defaults are the product's posture; an
/// organisation may widen them later without changing a rule's meaning.
#[derive(Debug, Clone, Copy)]
pub struct RiskWindows {
    /// How far ahead "imminent" looks, in days.
    pub imminent_days: i64,
    /// How long a project may go without an update.
    pub stale_update_days: i64,
    /// How long a task may sit blocked.
    pub blocked_days: i64,
    /// How many points ahead of progress burn may run before it warns.
    pub burn_gap_points: i64,
    /// How many days late a task may start against its baseline.
    pub baseline_tolerance_days: i64,
}

impl Default for RiskWindows {
    fn default() -> Self {
        Self {
            imminent_days: 7,
            stale_update_days: 14,
            blocked_days: 5,
            burn_gap_points: 15,
            baseline_tolerance_days: 3,
        }
    }
}

/// Computes every signal a project's facts support, most severe first.
pub fn evaluate(facts: &ProjectFacts, today: DateTime<Utc>, windows: RiskWindows) -> Vec<Signal> {
    let mut signals = Vec::new();
    let terminal = |status: &str| matches!(status, "completed" | "cancelled");

    for task in &facts.tasks {
        if terminal(&task.status) {
            continue;
        }
        if let Some(end) = task.end_at {
            if end < today {
                signals.push(Signal {
                    rule: Rule::TaskOverdue,
                    severity: Severity::Critical,
                    subject: Subject::Task,
                    subject_id: task.id,
                    message: format!("Work that was due on {} is still open.", end.date_naive()),
                });
            } else if let Some(start) = task.start_at {
                if start < today && task.status == "pending" {
                    signals.push(Signal {
                        rule: Rule::TaskNotStarted,
                        severity: Severity::Warn,
                        subject: Subject::Task,
                        subject_id: task.id,
                        message: format!(
                            "This work was due to start on {} and has not begun.",
                            start.date_naive()
                        ),
                    });
                }
            }
        }
        if !task.assigned {
            if let Some(start) = task.start_at {
                let horizon = today + chrono::Duration::days(windows.imminent_days);
                if start <= horizon && start >= today {
                    signals.push(Signal {
                        rule: Rule::UnassignedImminent,
                        severity: Severity::Warn,
                        subject: Subject::Task,
                        subject_id: task.id,
                        message: format!(
                            "Nobody is assigned to work starting on {}.",
                            start.date_naive()
                        ),
                    });
                }
            }
        }
        if task.status == "blocked" {
            if let Some(since) = task.blocked_since {
                if today - since > chrono::Duration::days(windows.blocked_days) {
                    signals.push(Signal {
                        rule: Rule::BlockedTooLong,
                        severity: Severity::Warn,
                        subject: Subject::Task,
                        subject_id: task.id,
                        message: format!(
                            "This work has been blocked since {}.",
                            since.date_naive()
                        ),
                    });
                }
            }
        }
        if let (Some(start), Some(baseline)) = (task.start_at, task.baseline_start_at) {
            let slip = (start - baseline).num_days();
            if slip > windows.baseline_tolerance_days {
                signals.push(Signal {
                    rule: Rule::SlippedAgainstBaseline,
                    severity: Severity::Info,
                    subject: Subject::Task,
                    subject_id: task.id,
                    message: format!("This work now starts {slip} days later than agreed."),
                });
            }
        }
    }

    for phase in &facts.phases {
        if terminal(&phase.status) {
            continue;
        }
        if phase.is_milestone {
            if let Some(end) = phase.end_date {
                if end < today.date_naive() {
                    signals.push(Signal {
                        rule: Rule::MilestoneOverdue,
                        severity: Severity::Critical,
                        subject: Subject::Phase,
                        subject_id: phase.id,
                        message: format!("The milestone promised for {end} is still open."),
                    });
                }
            }
        }
        if let (Some(budget), Some(consumed)) = (phase.budget_minor, phase.consumed_minor) {
            if budget > 0 && phase.tasks_total > 0 {
                let spent = (consumed as f64 / budget as f64) * 100.0;
                let done = (phase.tasks_done as f64 / phase.tasks_total as f64) * 100.0;
                if spent - done > windows.burn_gap_points as f64 {
                    signals.push(Signal {
                        rule: Rule::BurnAheadOfProgress,
                        severity: Severity::Warn,
                        subject: Subject::Phase,
                        subject_id: phase.id,
                        message: format!(
                            "This phase has used {:.0}% of its budget and finished {:.0}% of its work.",
                            spent, done
                        ),
                    });
                }
            }
        }
    }

    if facts.project_status == "active" {
        let stale = match facts.last_update_at {
            Some(at) => today - at > chrono::Duration::days(windows.stale_update_days),
            None => true,
        };
        if stale {
            signals.push(Signal {
                rule: Rule::NoRecentUpdate,
                severity: Severity::Warn,
                subject: Subject::Project,
                subject_id: facts.project_id,
                message: "The client has had no update inside the last two weeks.".into(),
            });
        }
        let dated = facts
            .tasks
            .iter()
            .filter(|t| t.start_at.is_some() && !terminal(&t.status))
            .count();
        if dated == 0 && !facts.tasks.is_empty() {
            signals.push(Signal {
                rule: Rule::PlanUndated,
                severity: Severity::Info,
                subject: Subject::Project,
                subject_id: facts.project_id,
                message: "None of this project's work has a date, so the plan cannot show when it happens."
                    .into(),
            });
        }
    }

    signals.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.rule.code().cmp(b.rule.code()))
            .then_with(|| a.subject_id.cmp(&b.subject_id))
    });
    signals
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 9, 0, 0)
            .single()
            .unwrap()
    }

    fn task(id: u128) -> TaskFacts {
        TaskFacts {
            id: Uuid::from_u128(id),
            status: "pending".into(),
            phase_id: None,
            assigned: true,
            start_at: None,
            duration_minutes: None,
            end_at: None,
            baseline_start_at: None,
            blocked_since: None,
        }
    }

    fn facts(tasks: Vec<TaskFacts>) -> ProjectFacts {
        ProjectFacts {
            project_id: Uuid::from_u128(99),
            project_status: "active".into(),
            project_due_date: None,
            last_update_at: Some(at(14)),
            phases: Vec::new(),
            tasks,
        }
    }

    #[test]
    fn the_rule_set_is_nine_and_stable() {
        assert_eq!(Rule::ALL.len(), 9);
        let codes: Vec<&str> = Rule::ALL.iter().map(|r| r.code()).collect();
        assert!(codes.contains(&"task_overdue"));
        assert!(codes.contains(&"plan_undated"));
    }

    #[test]
    fn an_overdue_task_is_critical() {
        let mut t = task(1);
        t.start_at = Some(at(1));
        t.end_at = Some(at(2));
        let signals = evaluate(&facts(vec![t]), at(10), RiskWindows::default());
        assert_eq!(signals[0].rule, Rule::TaskOverdue);
        assert_eq!(signals[0].severity, Severity::Critical);
    }

    #[test]
    fn a_completed_task_never_signals() {
        let mut t = task(1);
        t.status = "completed".into();
        t.start_at = Some(at(1));
        t.end_at = Some(at(2));
        let signals = evaluate(&facts(vec![t]), at(10), RiskWindows::default());
        assert!(signals.iter().all(|s| s.rule != Rule::TaskOverdue));
    }

    #[test]
    fn unassigned_work_inside_the_horizon_warns() {
        let mut t = task(2);
        t.assigned = false;
        t.start_at = Some(at(16));
        t.duration_minutes = Some(60);
        let signals = evaluate(&facts(vec![t]), at(14), RiskWindows::default());
        assert!(signals.iter().any(|s| s.rule == Rule::UnassignedImminent));
    }

    #[test]
    fn unassigned_work_beyond_the_horizon_is_quiet() {
        let mut t = task(2);
        t.assigned = false;
        t.start_at = Some(at(28));
        t.duration_minutes = Some(60);
        let signals = evaluate(&facts(vec![t]), at(14), RiskWindows::default());
        assert!(signals.iter().all(|s| s.rule != Rule::UnassignedImminent));
    }

    #[test]
    fn burn_ahead_of_progress_warns_on_the_phase() {
        let mut f = facts(vec![]);
        f.phases.push(PhaseFacts {
            id: Uuid::from_u128(5),
            status: "active".into(),
            is_milestone: false,
            end_date: None,
            budget_minor: Some(10_000),
            consumed_minor: Some(9_000),
            tasks_done: 1,
            tasks_total: 10,
        });
        let signals = evaluate(&f, at(14), RiskWindows::default());
        assert!(signals
            .iter()
            .any(|s| s.rule == Rule::BurnAheadOfProgress && s.subject == Subject::Phase));
    }

    #[test]
    fn a_project_with_no_update_inside_the_window_warns() {
        let mut f = facts(vec![]);
        f.last_update_at = Some(at(1));
        let signals = evaluate(&f, at(20), RiskWindows::default());
        assert!(signals.iter().any(|s| s.rule == Rule::NoRecentUpdate));
    }

    #[test]
    fn an_active_project_with_no_dated_work_says_so() {
        let mut t = task(3);
        t.start_at = None;
        let signals = evaluate(&facts(vec![t]), at(14), RiskWindows::default());
        assert!(signals.iter().any(|s| s.rule == Rule::PlanUndated));
    }

    #[test]
    fn most_severe_signals_come_first() {
        let mut overdue = task(1);
        overdue.start_at = Some(at(1));
        overdue.end_at = Some(at(2));
        let mut slipped = task(2);
        slipped.start_at = Some(at(14));
        slipped.baseline_start_at = Some(at(1));
        let signals = evaluate(
            &facts(vec![overdue, slipped]),
            at(14),
            RiskWindows::default(),
        );
        assert_eq!(signals[0].severity, Severity::Critical);
        assert!(signals.windows(2).all(|w| w[0].severity >= w[1].severity));
    }

    #[test]
    fn a_blocked_task_past_the_window_warns() {
        let mut t = task(4);
        t.status = "blocked".into();
        t.blocked_since = Some(at(1));
        let signals = evaluate(&facts(vec![t]), at(14), RiskWindows::default());
        assert!(signals.iter().any(|s| s.rule == Rule::BlockedTooLong));
        assert!(Duration::days(13) > Duration::days(5));
    }
}
