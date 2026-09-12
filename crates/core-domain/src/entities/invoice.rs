//! Invoices: the persisted billing documents an invoice run creates from
//! billable contracts (TASKLIST Phase 3 — native invoicing v1, Wave 9).
//!
//! Every invoice derives from exactly one contract and carries the
//! contract's currency; money is integer minor units throughout. The two
//! state machines are plain data (validated by [`InvoiceStatus`] and
//! [`DunningState`], checked in the DB by CHECK constraints):
//!
//! * [`InvoiceStatus`] — document lifecycle: `issued` → `paid` | `void`.
//!   `draft` is reserved for a future manual-invoice flow; runs always
//!   issue directly.
//! * [`DunningState`] — collections posture: `none` → `reminder_sent` →
//!   `overdue` → `final_notice` → `collections`, forward-only.
//!
//! [`InvoiceLine`] rows persist the core-billing rule engine's candidate
//! output (kind mirrors `BillingLineKind`); [`InvoiceTimeEntryLink`]
//! records which time entries an invoice consumed, so every entry bills
//! at most once, ever.

use crate::error::DomainError;
use crate::ids::{ClientId, ContractId, OrgId};
use chrono::{NaiveDate, Utc};
use std::str::FromStr;

/// Document lifecycle of an invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    /// Reserved for a future manual-invoice flow; runs issue directly.
    Draft,
    Issued,
    /// Fully paid (`total_paid_minor >= total_minor`); terminal.
    Paid,
    /// Withdrawn; terminal. Releases nothing (v1: consumption markers are
    /// permanent so re-runs can never double-bill).
    Void,
}

impl InvoiceStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            InvoiceStatus::Draft => "draft",
            InvoiceStatus::Issued => "issued",
            InvoiceStatus::Paid => "paid",
            InvoiceStatus::Void => "void",
        }
    }
}

impl FromStr for InvoiceStatus {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "draft" => Ok(Self::Draft),
            "issued" => Ok(Self::Issued),
            "paid" => Ok(Self::Paid),
            "void" => Ok(Self::Void),
            other => Err(DomainError::Validation(format!(
                "invalid invoice status: {other}"
            ))),
        }
    }
}

/// Collections posture of an issued invoice. Forward-only: states are
/// data moved by explicit API writes (events + write trail), never a
/// background side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DunningState {
    None,
    ReminderSent,
    Overdue,
    FinalNotice,
    Collections,
}

impl DunningState {
    pub const ALL: [DunningState; 5] = [
        DunningState::None,
        DunningState::ReminderSent,
        DunningState::Overdue,
        DunningState::FinalNotice,
        DunningState::Collections,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            DunningState::None => "none",
            DunningState::ReminderSent => "reminder_sent",
            DunningState::Overdue => "overdue",
            DunningState::FinalNotice => "final_notice",
            DunningState::Collections => "collections",
        }
    }

    /// Position in the ladder; [`DunningState::transition`] refuses moves
    /// that go backwards.
    pub const fn rank(self) -> u8 {
        match self {
            DunningState::None => 0,
            DunningState::ReminderSent => 1,
            DunningState::Overdue => 2,
            DunningState::FinalNotice => 3,
            DunningState::Collections => 4,
        }
    }

    /// The forward-only transition rule: any strictly-forward move is
    /// legal (skipping rungs is allowed — an org can go straight to
    /// collections); same-state is a no-op; backwards is refused.
    pub fn transition(from: DunningState, to: DunningState) -> Result<(), DomainError> {
        if to.rank() > from.rank() {
            Ok(())
        } else if to == from {
            Err(DomainError::Validation(format!(
                "dunning state is already {to:?}"
            )))
        } else {
            Err(DomainError::Validation(format!(
                "dunning state cannot move backwards from {from:?} to {to:?}"
            )))
        }
    }
}

impl std::fmt::Display for DunningState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DunningState {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "reminder_sent" => Ok(Self::ReminderSent),
            "overdue" => Ok(Self::Overdue),
            "final_notice" => Ok(Self::FinalNotice),
            "collections" => Ok(Self::Collections),
            other => Err(DomainError::Validation(format!(
                "invalid dunning state: {other}"
            ))),
        }
    }
}

/// The billing document. Derives from exactly one contract; `currency`
/// and `total_minor` come from the contract's evaluation (line sum).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Invoice {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// The run that created this invoice (`None` for future manual invoices).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub run_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    /// Human invoice number, unique per org (e.g. "INV-000042").
    pub number: String,
    pub status: InvoiceStatus,
    /// ISO 4217 code copied from the contract; every amount uses it.
    pub currency: String,
    /// The billed period, when the contract type has one (recurring /
    /// retainer / T&M window slice); `None` for fixed-fee schedules.
    pub period_from: Option<NaiveDate>,
    pub period_to: Option<NaiveDate>,
    pub issued_at: NaiveDate,
    pub due_on: NaiveDate,
    /// Sum of line amounts, minor units.
    pub total_minor: i64,
    /// Recorded payment total, minor units.
    pub total_paid_minor: i64,
    /// FX conversion snapshot (ADR 0002 lock moment: invoice issue) —
    /// locked once at the issue leg when currency ≠ org base; immutable
    /// afterwards. `base_total_minor` is the issue-time base equivalent.
    pub fx_rate_used: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fx_rate_date: Option<NaiveDate>,
    pub fx_source: Option<String>,
    pub base_total_minor: Option<i64>,
    pub dunning_state: DunningState,
    pub dunning_updated_at: Option<chrono::DateTime<Utc>>,
    pub dunning_note: Option<String>,
    pub notes: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl Invoice {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.number.trim().is_empty() {
            return Err(DomainError::Validation("invoice number is required".into()));
        }
        crate::money::CurrencyCode::new(&self.currency)?;
        if self.total_minor < 0 || self.total_paid_minor < 0 {
            return Err(DomainError::Validation(
                "invoice amounts must not be negative".into(),
            ));
        }
        if self.due_on < self.issued_at {
            return Err(DomainError::Validation(
                "due_on must not precede issued_at".into(),
            ));
        }
        Ok(())
    }
}

/// One persisted billing line; mirrors the engine's `BillingCandidateLine`
/// (kind reuse via [`crate::entities::BillingLineKind`]) minus the
/// denormalized contract/client names.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvoiceLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    pub kind: crate::entities::BillingLineKind,
    pub description: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    /// Labor minutes on the line (T&M / overage); 0 for amount lines.
    pub quantity_minutes: i64,
    pub unit_amount_minor: i64,
    pub amount_minor: i64,
    /// FX conversion snapshot (ADR 0002 lock moment: time-entry costing —
    /// the run pricing labor into this line) when currency ≠ org base.
    pub fx_rate_used: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fx_rate_date: Option<NaiveDate>,
    pub fx_source: Option<String>,
    pub base_amount_minor: Option<i64>,
    pub position: i32,
}

/// A payment receipt recorded through the `PaymentProvider` seam. The
/// provider stays a string (`none` in v1 — the built-in provider approves
/// locally without external PSP calls).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvoicePayment {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: uuid::Uuid,
    pub amount_minor: i64,
    pub currency: String,
    /// FX conversion snapshot (ADR 0002 lock moment: payment settlement) —
    /// locked once at receipt when currency ≠ org base; immutable after.
    pub fx_rate_used: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fx_rate_date: Option<NaiveDate>,
    pub fx_source: Option<String>,
    pub base_amount_minor: Option<i64>,
    pub provider: String,
    pub provider_ref: Option<String>,
    pub notes: Option<String>,
    pub received_at: chrono::DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
}

/// The invoice-run batch record: one evaluation sweep over a window and
/// the roll-up of the invoices it created.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvoiceRun {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// Human run number, unique per org (e.g. "RUN-000007").
    pub number: String,
    pub period_from: NaiveDate,
    pub period_to: NaiveDate,
    pub invoice_count: i32,
    pub total_minor: i64,
    /// Currency of the summed total; `None` when the run created no
    /// invoices (mixed currencies are rejected before any write).
    pub currency: Option<String>,
    pub due_in_days: i32,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dunning_moves_forward_only() {
        // Every strictly-forward move is legal, including skipping rungs.
        assert!(DunningState::transition(DunningState::None, DunningState::Collections).is_ok());
        assert!(DunningState::transition(DunningState::None, DunningState::ReminderSent).is_ok());
        assert!(
            DunningState::transition(DunningState::ReminderSent, DunningState::Overdue).is_ok()
        );
        assert!(DunningState::transition(DunningState::Overdue, DunningState::FinalNotice).is_ok());
        assert!(
            DunningState::transition(DunningState::FinalNotice, DunningState::Collections).is_ok()
        );

        // Same-state and backwards moves are refused.
        assert!(DunningState::transition(DunningState::Overdue, DunningState::Overdue).is_err());
        assert!(
            DunningState::transition(DunningState::FinalNotice, DunningState::Overdue).is_err()
        );
        assert!(DunningState::transition(DunningState::Collections, DunningState::None).is_err());
    }

    #[test]
    fn dunning_strings_round_trip() {
        for state in DunningState::ALL {
            assert_eq!(DunningState::from_str(state.as_str()).unwrap(), state);
            assert_eq!(state.to_string(), state.as_str());
        }
        assert!(DunningState::from_str("friendly email").is_err());
    }

    #[test]
    fn invoice_validation_basics() {
        let mut invoice = Invoice {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            run_id: None,
            client_id: uuid::Uuid::now_v7(),
            contract_id: uuid::Uuid::now_v7(),
            number: "INV-000001".into(),
            status: InvoiceStatus::Issued,
            currency: "USD".into(),
            period_from: None,
            period_to: None,
            issued_at: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            due_on: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            total_minor: 1_000,
            total_paid_minor: 0,
            fx_rate_used: None,
            fx_rate_date: None,
            fx_source: None,
            base_total_minor: None,
            dunning_state: DunningState::None,
            dunning_updated_at: None,
            dunning_note: None,
            notes: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(invoice.validate().is_ok());

        invoice.due_on = invoice.issued_at;
        assert!(invoice.validate().is_ok()); // due the day of issue is fine
        invoice.due_on = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
        assert!(invoice.validate().is_err()); // due before issue is not

        invoice.due_on = invoice.issued_at;
        invoice.total_paid_minor = -1;
        assert!(invoice.validate().is_err());

        invoice.total_paid_minor = 0;
        invoice.currency = "doubloons".into();
        assert!(invoice.validate().is_err());

        invoice.currency = "USD".into();
        invoice.number = "  ".into();
        assert!(invoice.validate().is_err());
    }

    #[test]
    fn status_strings_round_trip() {
        for (text, status) in [
            ("draft", InvoiceStatus::Draft),
            ("issued", InvoiceStatus::Issued),
            ("paid", InvoiceStatus::Paid),
            ("void", InvoiceStatus::Void),
        ] {
            assert_eq!(status.as_str(), text);
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::json!(text)
            );
        }
    }
}
