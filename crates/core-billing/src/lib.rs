//! core-billing — contracts, recurring billing rules, invoice runs,
//! `PaymentProvider` seam.
//!
//! Purpose (Wave 8b scope): the recurring-billing rules engine. Pure,
//! deterministic evaluation of [`core_domain::entities::Contract`] rows
//! into draft invoice-line candidates ([`rules::evaluate`]): billing
//! period arithmetic over anchor + frequency, the four contract types
//! (recurring / time_and_materials / retainer / fixed_fee), effective
//! dating, and money math in integer minor units. Storage and HTTP live
//! in core-db (`contracts` queries, migration 0029) and core-api
//! (`routes::billing`) respectively; this crate owns only the rules.
//!
//! Non-purpose: NO invoice persistence, payments, dunning, or
//! `PaymentProvider` wiring — those are Wave 9. The engine's output is a
//! preview shape; the invoice run will consume the same evaluation when
//! it lands. No currency conversion: every line carries the contract's
//! own currency.

pub mod periods;
pub mod rules;

pub use periods::{period_start, periods_starting_in};
pub use rules::{evaluate, BillableEntry, BillingError};
