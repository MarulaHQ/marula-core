//! Typed errors for the report compiler. Every variant is a validation
//! failure surfaced as HTTP 422 — none of them ever carries SQL text.

use std::fmt;

use crate::card::{CardFailure, CardShape};

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
    /// A stored report spec violated the typed registry (the private build tracker R3):
    /// unknown slug, a type mismatch between filter operand and catalog
    /// column, an aggregate applied to a non-numeric column, a shape bound
    /// (filter/measure/group-by caps), or a duplicate output name. The
    /// `field` names the spec member at fault; `reason` is the specific
    /// violation. Never carries SQL text.
    InvalidSpec { field: String, reason: String },
    /// A card definition violated its own contract (Wave 46 lane 1): an
    /// out-of-range window or refresh, or a shape it cannot fill statically.
    /// `field` names the card member at fault.
    InvalidCard { field: String, reason: String },
    /// The card displays a wider window than one refresh reads, so the older
    /// part of the card is never recomputed. Carries both numbers so a repair
    /// loop can widen the reach or narrow the window without guessing.
    WindowWiderThanRefresh { window_days: u32, reach_days: u32 },
    /// The card refreshes less often than the window it displays, so it
    /// shows a span it never revisits.
    RefreshSlowerThanWindow {
        every_minutes: u32,
        window_days: u32,
    },
    /// The card's own data cannot fill its shape: no rows for a chart or a
    /// table, no projected columns for a table's headers, no scalar for a
    /// stat. Typed (`shape` + `failure`) because "empty" and "broken" are
    /// different answers and a caller must be able to act on the second.
    EmptyCard {
        shape: CardShape,
        failure: CardFailure,
    },
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
            ReportsError::InvalidCard { field, reason } => {
                write!(f, "invalid card ({field}): {reason}")
            }
            ReportsError::WindowWiderThanRefresh {
                window_days,
                reach_days,
            } => write!(
                f,
                "card shows a {window_days}-day window but one refresh reads \
                 {reach_days} days, so the rest is never recomputed"
            ),
            ReportsError::RefreshSlowerThanWindow {
                every_minutes,
                window_days,
            } => write!(
                f,
                "card refreshes every {every_minutes} minutes but shows a \
                 {window_days}-day window"
            ),
            ReportsError::EmptyCard { shape, failure } => match failure {
                CardFailure::NoRows => {
                    write!(f, "{} card has no rows at this window", shape.slug())
                }
                CardFailure::NoHeaders => {
                    write!(f, "{} card has no columns to use as headers", shape.slug())
                }
                CardFailure::NoScalar { column } => write!(
                    f,
                    "{} card has no scalar for '{}' in the run summary",
                    shape.slug(),
                    column
                ),
            },
        }
    }
}

impl std::error::Error for ReportsError {}
