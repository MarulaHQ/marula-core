//! Prebuilt report kinds (the private build tracker R1) — the closed enumeration of v1
//! operational reports, each bound to exactly one catalog view and one
//! validated parameter shape.
//!
//! Non-purpose: no ad-hoc view/column selection (that is the R3/R6
//! builder's job on top of this same catalog); no scheduling; no storage.

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use crate::catalog::ViewId;

/// The eight v1 report kinds. Slugs are snake_case URL path segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportKind {
    TimeByUser,
    TimeByClient,
    TimeByTicket,
    SlaAttainment,
    SlaBreaches,
    TicketThroughput,
    TicketAging,
    FirstResponse,
}

impl ReportKind {
    pub const ALL: [ReportKind; 8] = [
        ReportKind::TimeByUser,
        ReportKind::TimeByClient,
        ReportKind::TimeByTicket,
        ReportKind::SlaAttainment,
        ReportKind::SlaBreaches,
        ReportKind::TicketThroughput,
        ReportKind::TicketAging,
        ReportKind::FirstResponse,
    ];

    /// Stable URL/API slug. Constants only.
    pub const fn slug(self) -> &'static str {
        match self {
            ReportKind::TimeByUser => "time_by_user",
            ReportKind::TimeByClient => "time_by_client",
            ReportKind::TimeByTicket => "time_by_ticket",
            ReportKind::SlaAttainment => "sla_attainment",
            ReportKind::SlaBreaches => "sla_breaches",
            ReportKind::TicketThroughput => "ticket_throughput",
            ReportKind::TicketAging => "ticket_aging",
            ReportKind::FirstResponse => "first_response",
        }
    }

    /// The single read-model view this kind consumes (ADR 0001: views
    /// only, never physical tables).
    pub const fn view(self) -> ViewId {
        match self {
            ReportKind::TimeByUser | ReportKind::TimeByClient | ReportKind::TimeByTicket => {
                ViewId::TimeEntries
            }
            ReportKind::SlaAttainment | ReportKind::SlaBreaches => ViewId::SlaOutcomes,
            ReportKind::TicketThroughput | ReportKind::TicketAging => ViewId::Tickets,
            ReportKind::FirstResponse => ViewId::FirstResponse,
        }
    }

    /// Strict parser for the `{kind}` path segment. Anything unknown fails
    /// closed.
    pub fn parse(slug: &str) -> Option<ReportKind> {
        Self::ALL.into_iter().find(|k| k.slug() == slug)
    }

    /// Whether the kind supports `?user_id=` narrowing at all.
    pub const fn has_user_dimension(self) -> bool {
        matches!(
            self,
            ReportKind::TimeByUser
                | ReportKind::TimeByClient
                | ReportKind::TimeByTicket
                | ReportKind::SlaAttainment
                | ReportKind::SlaBreaches
                | ReportKind::FirstResponse
                | ReportKind::TicketThroughput
                | ReportKind::TicketAging
        )
    }
}

/// Shared date-window inputs. Semantics resolve in [`crate::params`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowParams {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

/// Parameters shared by the three time rollups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeReportParams {
    pub window: WindowParams,
    /// Optional narrowing to a single staff member (`None` = all rows in
    /// scope). This is also the slot RBAC self-scoping is forced into.
    pub user_id: Option<Uuid>,
}

/// TimeByUser extra: billable filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeByUserParams {
    pub base: TimeReportParams,
    pub billable: Option<bool>,
}

impl From<&TimeReportParams> for TimeByUserParams {
    fn from(base: &TimeReportParams) -> Self {
        Self {
            base: base.clone(),
            billable: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlaOutcomesParams {
    pub window: WindowParams,
    pub policy_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThroughputParams {
    pub window: WindowParams,
    pub user_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgingParams {
    pub user_id: Option<Uuid>,
    /// Snapshot instant the ages are computed against. Fixed explicitly by
    /// tests for determinism; the API passes `Utc::now()`.
    pub as_of: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstResponseParams {
    pub window: WindowParams,
    pub user_id: Option<Uuid>,
}

/// Everything the compiler needs per request. The API layer maps query
/// parameters into these after validation.
#[derive(Debug, Clone, PartialEq)]
pub enum KindParams {
    TimeByUser(TimeByUserParams),
    TimeByClient(TimeReportParams),
    TimeByTicket(TimeReportParams),
    SlaAttainment(SlaOutcomesParams),
    SlaBreaches(SlaOutcomesParams),
    TicketThroughput(ThroughputParams),
    TicketAging(AgingParams),
    FirstResponse(FirstResponseParams),
}

impl KindParams {
    pub fn window(&self) -> &WindowParams {
        match self {
            KindParams::TimeByUser(p) => &p.base.window,
            KindParams::TimeByClient(p) => &p.window,
            KindParams::TimeByTicket(p) => &p.window,
            KindParams::SlaAttainment(p) => &p.window,
            KindParams::SlaBreaches(p) => &p.window,
            KindParams::TicketThroughput(p) => &p.window,
            KindParams::TicketAging(_) => unreachable!("aging takes no window"),
            KindParams::FirstResponse(p) => &p.window,
        }
    }
}

/// Matches params to their kind at compile-time shape level. Any mismatch
/// is an internal invariant violation surfaced loudly.
pub fn validate_kind(kind: ReportKind, params: &KindParams) {
    match (kind, params) {
        (ReportKind::TimeByUser, KindParams::TimeByUser(_))
        | (ReportKind::TimeByClient, KindParams::TimeByClient(_))
        | (ReportKind::TimeByTicket, KindParams::TimeByTicket(_))
        | (ReportKind::SlaAttainment, KindParams::SlaAttainment(_))
        | (ReportKind::SlaBreaches, KindParams::SlaBreaches(_))
        | (ReportKind::TicketThroughput, KindParams::TicketThroughput(_))
        | (ReportKind::TicketAging, KindParams::TicketAging(_))
        | (ReportKind::FirstResponse, KindParams::FirstResponse(_)) => {}
        _ => panic!("kind/params mismatch: {kind:?} given {params:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_round_trip_strictly() {
        for kind in ReportKind::ALL {
            assert_eq!(ReportKind::parse(kind.slug()), Some(kind));
        }
    }

    #[test]
    fn unknown_or_adversarial_slugs_fail_closed() {
        for bad in [
            "",
            "..",
            "TIME_BY_USER",
            "time-by-user",
            "time_by_user%00",
            "' OR 1=1 --",
            "../../etc/passwd",
            "sla_attainment\x00DROP TABLE tickets;",
        ] {
            assert_eq!(ReportKind::parse(bad), None, "slug {bad:?} must not parse");
        }
    }

    #[test]
    fn every_kind_maps_to_a_versioned_view() {
        for kind in ReportKind::ALL {
            let view = kind.view().qualified_name();
            assert!(view.starts_with("ops.v1_"), "{kind:?} escaped the layer");
        }
    }
}
