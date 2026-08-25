//! Typed domain errors. No anyhow inside the domain — every failure mode is
//! enumerable and machine-readable.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("invalid status category: {0}")]
    InvalidStatusCategory(String),
    #[error("invalid priority: {0}")]
    InvalidPriority(String),
    #[error("invalid note kind: {0}")]
    InvalidNoteKind(String),
    #[error("money overflow: {0} + {1}")]
    MoneyOverflow(String, String),
    #[error("currency mismatch: {0} vs {1}")]
    CurrencyMismatch(String, String),
    #[error("invalid duration: {0} seconds")]
    InvalidDuration(i64),
    #[error("checklist incomplete: {0} unchecked item(s)")]
    ChecklistIncomplete(usize),
    #[error("transition not allowed: {0} → {1}")]
    InvalidTransition(String, String),
    #[error("validation failed: {0}")]
    Validation(String),
}
