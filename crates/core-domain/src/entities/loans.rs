//! Client loans + rentals (Wave 32 lane 3, `p32-tools-loans`).
//!
//! A device — sourced from org stock (a serialized lane-1 item + its
//! serial) or from an existing `DeviceAsset` — is issued TO A CLIENT under
//! an agreement. The loan header carries the client, the contract FK where
//! one exists (else the free-form agreement reference), the out date, the
//! expected return, and the return condition. Stock-sourced lines ride the
//! lane-1 append-only ledger on BOTH legs (`loan` out, `loan_return` in);
//! asset-sourced lines reference the asset that was issued.
//!
//! On issue, a stock-sourced device instantiates a `DeviceAsset` in the
//! client's fleet carrying identity keys ({serial}) — the ADR 0003 dedupe
//! path then converges a later M365/RMM feed describing the same phone
//! onto that same row (a serial match is decisive, never a twin).
//!
//! When the loan IS a lease, the lease terms reuse the 0061 rental
//! vocabulary: the loan carries `lease_cycle` / `lease_cost_minor` /
//! `lease_currency`, and the issue leg stamps the asset's `rental_*`
//! fields (ownership `leased`, start = out date, end = expected return).
//! Money is integer minor units + ISO 4217 (ADR 0002); no conversion
//! happens here.

use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

/// Lease billing cycles — identical vocabulary to the 0061 asset rental
/// fields (`rental_cycle`) so an issued lease never needs translation.
pub const LEASE_CYCLES: [&str; 3] = ["monthly", "quarterly", "annual"];

pub fn is_valid_lease_cycle(cycle: &str) -> bool {
    LEASE_CYCLES.contains(&cycle.trim())
}

/// Loan lifecycle. Overdue is derived (`open` + past expected return),
/// never stored.
pub const LOAN_STATUSES: [&str; 2] = ["open", "returned"];

/// A client loan/rental agreement record.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientLoan {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: Uuid,
    /// Agreement anchor: the contract row where one exists.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub contract_id: Option<Uuid>,
    /// Free-form contract/agreement reference when no contract row exists.
    pub agreement_ref: Option<String>,
    pub out_at: DateTime<Utc>,
    pub expected_return_at: NaiveDate,
    pub returned_at: Option<DateTime<Utc>>,
    pub status: String,
    /// True when this loan is a lease; the terms below are then required
    /// and stamped onto the issued asset's rental fields.
    pub is_lease: bool,
    pub lease_cycle: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub lease_cost_minor: Option<i64>,
    pub lease_currency: Option<String>,
    pub notes: Option<String>,
    pub return_notes: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub issued_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ClientLoan {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !LOAN_STATUSES.contains(&self.status.as_str()) {
            return Err(DomainError::Validation(
                "loan status must be open or returned".into(),
            ));
        }
        if (self.status == "returned") != self.returned_at.is_some() {
            return Err(DomainError::Validation(
                "a returned loan carries returned_at; an open one does not".into(),
            ));
        }
        if self.expected_return_at < self.out_at.date_naive() {
            return Err(DomainError::Validation(
                "expected return must not precede the out date".into(),
            ));
        }
        if self.is_lease {
            let cycle = self.lease_cycle.as_deref().unwrap_or("");
            if !is_valid_lease_cycle(cycle) {
                return Err(DomainError::Validation(
                    "a lease needs a cycle of: monthly, quarterly, annual".into(),
                ));
            }
            if self.lease_cost_minor.is_none() || self.lease_currency.is_none() {
                return Err(DomainError::Validation(
                    "a lease needs its cycle cost and currency".into(),
                ));
            }
        } else if self.lease_cycle.is_some()
            || self.lease_cost_minor.is_some()
            || self.lease_currency.is_some()
        {
            return Err(DomainError::Validation(
                "lease terms are only valid when the loan is a lease".into(),
            ));
        }
        if let Some(currency) = &self.lease_currency {
            crate::money::CurrencyCode::new(currency)?;
        }
        for text in [
            self.agreement_ref.as_deref(),
            self.notes.as_deref(),
            self.return_notes.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if text.chars().count() > 500 {
                return Err(DomainError::Validation(
                    "loan text fields must be at most 500 characters".into(),
                ));
            }
        }
        Ok(())
    }

    /// Derived, never stored: overdue while open past the expected return.
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        self.returned_at.is_none() && self.expected_return_at < today
    }
}

/// One device on a loan. Stock-sourced lines carry the stock item + serial
/// and reference both ledger legs; every line references the device's row
/// in the client's fleet (instantiated on issue when the device came from
/// stock).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientLoanLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub loan_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub stock_item_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub serial_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub device_asset_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub movement_out_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub movement_in_id: Option<Uuid>,
    pub returned_at: Option<DateTime<Utc>>,
    pub return_condition: Option<String>,
    pub return_notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ClientLoanLine {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.stock_item_id.is_none() && self.device_asset_id.is_none() {
            return Err(DomainError::Validation(
                "a loan line needs a stock item or a device asset".into(),
            ));
        }
        if self.stock_item_id.is_some() != self.serial_id.is_some() {
            return Err(DomainError::Validation(
                "a stock-sourced loan line always carries its serial".into(),
            ));
        }
        if let Some(condition) = &self.return_condition {
            if !crate::entities::tools::is_valid_return_condition(condition) {
                return Err(DomainError::Validation(
                    "return condition must be one of: good, fair, damaged".into(),
                ));
            }
            if self.returned_at.is_none() {
                return Err(DomainError::Validation(
                    "a return condition requires the device to be returned".into(),
                ));
            }
        }
        if let Some(notes) = &self.return_notes {
            if notes.chars().count() > 500 {
                return Err(DomainError::Validation(
                    "return notes must be at most 500 characters".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn is_returned(&self) -> bool {
        self.returned_at.is_some()
    }

    pub fn from_stock(&self) -> bool {
        self.stock_item_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loan() -> ClientLoan {
        ClientLoan {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            client_id: Uuid::now_v7(),
            contract_id: None,
            agreement_ref: Some("AGR-2026-014".into()),
            out_at: Utc::now(),
            expected_return_at: (Utc::now() + chrono::Duration::days(30)).date_naive(),
            returned_at: None,
            status: "open".into(),
            is_lease: false,
            lease_cycle: None,
            lease_cost_minor: None,
            lease_currency: None,
            notes: None,
            return_notes: None,
            issued_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn plain_loan_shape_holds() {
        loan().validate().unwrap();
    }

    #[test]
    fn lease_terms_are_required_and_exclusive() {
        let mut l = loan();
        l.is_lease = true;
        assert!(l.validate().is_err(), "lease without terms is rejected");
        l.lease_cycle = Some("monthly".into());
        l.lease_cost_minor = Some(4_900);
        l.lease_currency = Some("USD".into());
        l.validate().unwrap();

        let mut l = loan();
        l.lease_cost_minor = Some(4_900);
        assert!(
            l.validate().is_err(),
            "lease terms on a non-lease loan are rejected"
        );
    }

    #[test]
    fn overdue_is_derived_from_expected_return() {
        let mut l = loan();
        let today = l.expected_return_at + chrono::Duration::days(1);
        assert!(
            !l.is_overdue(l.expected_return_at),
            "due today is not overdue"
        );
        assert!(l.is_overdue(today));
        l.returned_at = Some(Utc::now());
        l.status = "returned".into();
        assert!(!l.is_overdue(today), "a returned loan is never overdue");
    }

    #[test]
    fn stock_lines_pair_item_and_serial() {
        let mut line = ClientLoanLine {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            loan_id: Uuid::now_v7(),
            stock_item_id: Some(Uuid::now_v7()),
            serial_id: None,
            device_asset_id: Some(Uuid::now_v7()),
            movement_out_id: None,
            movement_in_id: None,
            returned_at: None,
            return_condition: None,
            return_notes: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(line.validate().is_err(), "item without serial is rejected");
        line.serial_id = Some(Uuid::now_v7());
        line.validate().unwrap();

        let mut bare = ClientLoanLine {
            stock_item_id: None,
            serial_id: None,
            device_asset_id: None,
            ..line
        };
        assert!(bare.validate().is_err(), "no source is rejected");
        bare.device_asset_id = Some(Uuid::now_v7());
        bare.validate().unwrap();
    }
}
