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
//! Wave 9 (native invoicing v1) adds the [`payments`] seam here: the
//! `PaymentProvider` trait with the built-in none-mode provider. Invoice
//! persistence, dunning states, and the run orchestration live in
//! core-db (migration 0036) and core-api (`routes::invoices`).
//!
//! Non-purpose: NO currency conversion: every line carries the contract's
//! own currency.

pub mod payments;
pub mod periods;
pub mod rules;

pub use payments::{
    ChargeReceipt, ChargeRequest, NonePaymentProvider, PaymentDeclinedCode, PaymentError,
    PaymentProvider, StripePaymentProvider,
};
pub use periods::{period_start, periods_starting_in};
pub use rules::{evaluate, BillableEntry, BillingError};
