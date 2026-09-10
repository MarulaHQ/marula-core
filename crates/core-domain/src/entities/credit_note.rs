//! Credit notes: client credits issued, applied against invoices, and
//! refunded (TASKLIST Money+docs backend gap, Wave 11 backend-gaps lane).
//!
//! Money is integer minor units + ISO 4217 throughout — a credit note
//! never converts currency (ADR 0002): it is issued in one currency and
//! its applications/refunds stay in it. The document lifecycle is data
//! (`issued` → `void`); allocation is tracked as sums, not statuses —
//! `applied_minor` + `refunded_minor` against `total_minor` with
//! `remaining()` ≥ 0 validated on every write. A void is only legal
//! while nothing has moved.

use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::Utc;
use std::str::FromStr;

/// Document lifecycle of a credit note. `applied` / `refunded` /
/// `partially applied` are DERIVED from the counters, never stored —
/// allocation moves continuously; only a void is a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CreditNoteStatus {
    /// Open: remaining credit is available to apply or refund.
    Issued,
    /// Withdrawn; terminal. Only legal while applied + refunded == 0.
    Void,
}

impl CreditNoteStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            CreditNoteStatus::Issued => "issued",
            CreditNoteStatus::Void => "void",
        }
    }
}

impl FromStr for CreditNoteStatus {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "issued" => Ok(Self::Issued),
            "void" => Ok(Self::Void),
            other => Err(DomainError::Validation(format!(
                "invalid credit note status: {other}"
            ))),
        }
    }
}

/// The credit document. `issued_against_invoice_id` is the issuing
/// context (may be `None` for a free-standing client credit);
/// applications may target any issued invoice of the same client and
/// currency.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreditNote {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// Human credit-note number, unique per org (e.g. "CRN-000012").
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub issued_against_invoice_id: Option<uuid::Uuid>,
    pub status: CreditNoteStatus,
    /// ISO 4217 code; every amount on the credit uses it.
    pub currency: String,
    /// Credit face value, minor units (> 0).
    pub total_minor: i64,
    /// Sum moved onto invoices, minor units.
    pub applied_minor: i64,
    /// Sum refunded back out, minor units.
    pub refunded_minor: i64,
    pub reason: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl CreditNote {
    /// Still-available credit, minor units.
    pub fn remaining_minor(&self) -> i64 {
        self.total_minor - self.applied_minor - self.refunded_minor
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.number.trim().is_empty() {
            return Err(DomainError::Validation(
                "credit note number is required".into(),
            ));
        }
        crate::money::CurrencyCode::new(&self.currency)?;
        if self.total_minor <= 0 {
            return Err(DomainError::Validation(
                "credit note total must be positive".into(),
            ));
        }
        if self.applied_minor < 0 || self.refunded_minor < 0 {
            return Err(DomainError::Validation(
                "credit note allocations must not be negative".into(),
            ));
        }
        if self.remaining_minor() < 0 {
            return Err(DomainError::Validation(
                "credit note over-allocated: applied + refunded exceed total".into(),
            ));
        }
        if self.status == CreditNoteStatus::Void
            && (self.applied_minor > 0 || self.refunded_minor > 0)
        {
            return Err(DomainError::Validation(
                "a credit note with allocations cannot be voided".into(),
            ));
        }
        Ok(())
    }
}

/// One allocation of credit-note value onto an invoice. The money move
/// itself is the paired invoice payment receipt (provider `credit_note`);
/// this row is the credit-side ledger.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreditNoteApplication {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub credit_note_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: uuid::Uuid,
    pub amount_minor: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub payment_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> CreditNote {
        CreditNote {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            number: "CRN-000001".into(),
            client_id: uuid::Uuid::now_v7(),
            issued_against_invoice_id: None,
            status: CreditNoteStatus::Issued,
            currency: "USD".into(),
            total_minor: 10_000,
            applied_minor: 0,
            refunded_minor: 0,
            reason: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn remaining_is_total_minus_allocations() {
        let mut n = note();
        assert_eq!(n.remaining_minor(), 10_000);
        n.applied_minor = 4_000;
        n.refunded_minor = 1_000;
        assert_eq!(n.remaining_minor(), 5_000);
        assert!(n.validate().is_ok());
    }

    #[test]
    fn over_allocation_is_invalid() {
        let mut n = note();
        n.applied_minor = 6_000;
        n.refunded_minor = 6_000;
        assert!(n.validate().is_err());
    }

    #[test]
    fn void_with_allocations_is_invalid() {
        let mut n = note();
        n.applied_minor = 2_000;
        n.status = CreditNoteStatus::Void;
        assert!(n.validate().is_err());
        n.applied_minor = 0;
        assert!(n.validate().is_ok());
    }
}
