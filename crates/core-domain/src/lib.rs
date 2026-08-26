//! core-domain — entities, policies, and domain services for Meridian.
//!
//! Purpose: the pure, framework-free heart of the product. Every rule that must
//! hold regardless of HTTP, queues, or storage lives here, heavily tested.
//!
//! Non-purpose: no I/O of any kind. No SQL, no HTTP, no Redis, no clocks read
//! from the environment — time and effects are always injected as parameters.

pub mod automations;
pub mod entities;
pub mod error;
pub mod events;
pub mod ids;
pub mod mentions;
pub mod money;
pub mod notes;
pub mod notifications;
pub mod policy;
pub mod sla_engine;
pub mod status;
pub mod workflow;

pub use entities::*;
pub use error::DomainError;
pub use ids::*;
pub use money::Money;
pub use policy::{Permission, PolicyDecision};
pub use status::{Priority, TicketStatusCategory};
pub use workflow::{TransitionEdge, TransitionRejection};

/// Semantic categories every custom ticket status must bind to (§5).
/// SLA timers, pause logic, automations, billing rules, and reports bind to
/// these categories — never to display labels.
pub const STATUS_CATEGORIES: [TicketStatusCategory; 4] = [
    TicketStatusCategory::Open,
    TicketStatusCategory::InProgress,
    TicketStatusCategory::Waiting,
    TicketStatusCategory::Closed,
];

/// Version of the domain event envelope schema emitted by this build.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_round_trips() {
        for c in STATUS_CATEGORIES {
            assert_eq!(c.as_str().parse::<TicketStatusCategory>().unwrap(), c);
        }
    }
}
