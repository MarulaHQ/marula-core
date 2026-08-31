//! Typed errors for the report compiler. Every variant is a validation
//! failure surfaced as HTTP 422 — none of them ever carries SQL text.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportsError {
    /// A name outside the static catalog reached a lookup. This is the
    /// fail-closed seam for identifier injection attempts.
    UnknownColumn { view: String, name: String },
    /// Malformed or over-wide date window.
    InvalidWindow(String),
    /// Pagination bounds violation.
    InvalidLimit(i64),
    /// An opaque cursor failed to decode (client error, never a panic).
    InvalidCursor,
    /// A stored report spec violated the typed registry (TASKLIST R3):
    /// unknown slug, a type mismatch between filter operand and catalog
    /// column, an aggregate applied to a non-numeric column, a shape bound
    /// (filter/measure/group-by caps), or a duplicate output name. The
    /// `field` names the spec member at fault; `reason` is the specific
    /// violation. Never carries SQL text.
    InvalidSpec { field: String, reason: String },
}

impl fmt::Display for ReportsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReportsError::UnknownColumn { view, name } => {
                write!(f, "unknown column '{name}' on view '{view}'")
            }
            ReportsError::InvalidWindow(msg) => write!(f, "invalid date window: {msg}"),
            ReportsError::InvalidLimit(limit) => {
                write!(f, "limit must be between 1 and 500, got {limit}")
            }
            ReportsError::InvalidCursor => write!(f, "malformed pagination cursor"),
            ReportsError::InvalidSpec { field, reason } => {
                write!(f, "invalid report spec ({field}): {reason}")
            }
        }
    }
}

impl std::error::Error for ReportsError {}
