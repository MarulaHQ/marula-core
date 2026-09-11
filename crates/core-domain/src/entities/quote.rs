//! Quotes + line items (TASKLIST Phase 3, Wave 12 lane 2).
//!
//! A quote is a client-scoped document with a numbered lifecycle:
//! `draft` → `sent` → `accepted` | `declined` | `expired`. Money is
//! integer minor units + ISO 4217 (floats never touch amounts):
//!
//! * line amount = half-even(quantity_milli × unit_amount_minor / 1000)
//!   — fractions of a unit stay integer-only (`1.5` hours = `1500`).
//! * subtotal = sum(line amounts).
//! * tax = half-even(taxable subtotal × tax_rate_bps / 10000), where the
//!   taxable subtotal sums only lines whose `taxable` flag is set.
//! * total = subtotal + tax.
//!
//! **Immutability once sent:** only a `draft` quote may be edited; the
//! lifecycle legs (send/accept/decline/expire/convert) are the only later
//! writes. Quote lines snapshot their catalog context at write time, so a
//! later catalog change never alters an existing quote.

use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::{NaiveDate, Utc};
use std::str::FromStr;

/// Lifecycle of a quote. `accepted` / `declined` / `expired` are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum QuoteStatus {
    Draft,
    Sent,
    Accepted,
    Declined,
    Expired,
}

impl QuoteStatus {
    pub const ALL: [QuoteStatus; 5] = [
        QuoteStatus::Draft,
        QuoteStatus::Sent,
        QuoteStatus::Accepted,
        QuoteStatus::Declined,
        QuoteStatus::Expired,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            QuoteStatus::Draft => "draft",
            QuoteStatus::Sent => "sent",
            QuoteStatus::Accepted => "accepted",
            QuoteStatus::Declined => "declined",
            QuoteStatus::Expired => "expired",
        }
    }

    /// Only a draft is editable — sending locks the document.
    pub const fn is_editable(self) -> bool {
        matches!(self, QuoteStatus::Draft)
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            QuoteStatus::Accepted | QuoteStatus::Declined | QuoteStatus::Expired
        )
    }

    /// The legal lifecycle edges, as data. Refuses same-state and any
    /// backwards/terminal move with a typed validation error.
    pub fn transition(from: QuoteStatus, to: QuoteStatus) -> Result<(), DomainError> {
        let ok = matches!(
            (from, to),
            (QuoteStatus::Draft, QuoteStatus::Sent)
                | (QuoteStatus::Draft, QuoteStatus::Expired)
                | (QuoteStatus::Sent, QuoteStatus::Accepted)
                | (QuoteStatus::Sent, QuoteStatus::Declined)
                | (QuoteStatus::Sent, QuoteStatus::Expired)
        );
        if ok {
            Ok(())
        } else {
            Err(DomainError::Validation(format!(
                "illegal quote transition {} → {}",
                from.as_str(),
                to.as_str()
            )))
        }
    }
}

impl FromStr for QuoteStatus {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "draft" => Ok(Self::Draft),
            "sent" => Ok(Self::Sent),
            "accepted" => Ok(Self::Accepted),
            "declined" => Ok(Self::Declined),
            "expired" => Ok(Self::Expired),
            other => Err(DomainError::Validation(format!(
                "invalid quote status: {other}"
            ))),
        }
    }
}

/// The rolled-up money on a quote, integer minor units. Returned by
/// [`rollup`] and recomputed by the API on every line edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteRollup {
    pub subtotal_minor: i64,
    pub tax_minor: i64,
    pub total_minor: i64,
}

/// `numerator / denominator` rounded half-even (banker's rounding).
/// Integer-only: no float ever touches an amount.
fn div_round_half_even(numerator: i128, denominator: i128) -> i128 {
    let q = numerator.div_euclid(denominator);
    let r = numerator.rem_euclid(denominator);
    let twice = r * 2;
    if twice > denominator || (twice == denominator && q % 2 != 0) {
        q + 1
    } else {
        q
    }
}

/// Line amount for `quantity_milli` units at `unit_amount_minor` per
/// whole unit, half-even to the nearest minor unit.
pub fn line_amount_minor(quantity_milli: i64, unit_amount_minor: i64) -> Result<i64, DomainError> {
    if quantity_milli <= 0 {
        return Err(DomainError::Validation(
            "line quantity must be positive".into(),
        ));
    }
    if unit_amount_minor < 0 {
        return Err(DomainError::Validation(
            "line unit amount must not be negative".into(),
        ));
    }
    let raw = (quantity_milli as i128) * (unit_amount_minor as i128);
    let rounded = div_round_half_even(raw, 1_000);
    i64::try_from(rounded)
        .map_err(|_| DomainError::MoneyOverflow(raw.to_string(), "line amount".into()))
}

/// Rolls line amounts + flat tax rate into subtotal/tax/total. Tax applies
/// to the taxable subtotal only; `tax_rate_bps` is basis points (800 = 8%).
pub fn rollup(lines: &[QuoteLine], tax_rate_bps: i32) -> Result<QuoteRollup, DomainError> {
    if !(0..=10_000).contains(&tax_rate_bps) {
        return Err(DomainError::Validation(
            "tax rate must be between 0 and 10000 basis points".into(),
        ));
    }
    let mut subtotal_i: i128 = 0;
    let mut taxable_i: i128 = 0;
    for line in lines {
        line.validate()?;
        subtotal_i += line.amount_minor as i128;
        if line.taxable {
            taxable_i += line.amount_minor as i128;
        }
    }
    let tax_i = div_round_half_even(taxable_i * tax_rate_bps as i128, 10_000);
    let subtotal_minor = i64::try_from(subtotal_i)
        .map_err(|_| DomainError::MoneyOverflow(subtotal_i.to_string(), "subtotal".into()))?;
    let tax_minor = i64::try_from(tax_i)
        .map_err(|_| DomainError::MoneyOverflow(tax_i.to_string(), "tax".into()))?;
    let total_minor = subtotal_minor
        .checked_add(tax_minor)
        .ok_or_else(|| DomainError::MoneyOverflow(subtotal_minor.to_string(), "total".into()))?;
    Ok(QuoteRollup {
        subtotal_minor,
        tax_minor,
        total_minor,
    })
}

/// The quote document. `subtotal/tax/total` are persisted roll-ups of the
/// lines, recomputed on every draft edit.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Quote {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    /// Human quote number, unique per org (e.g. "Q-000012").
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub status: QuoteStatus,
    pub currency: String,
    pub issued_on: NaiveDate,
    /// Last day the quote is valid; conversion after this is refused.
    pub valid_until: NaiveDate,
    pub notes: Option<String>,
    pub terms: Option<String>,
    pub subtotal_minor: i64,
    pub tax_rate_bps: i32,
    pub tax_minor: i64,
    pub total_minor: i64,
    pub sent_at: Option<chrono::DateTime<Utc>>,
    /// When the quote reached a terminal accept/decline/expire state.
    pub decided_at: Option<chrono::DateTime<Utc>>,
    /// The contract the convert-to-invoice seam materialized (NULL until
    /// converted).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub converted_contract_id: Option<uuid::Uuid>,
    /// The DRAFT invoice the seam created (NULL until converted).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub converted_invoice_id: Option<uuid::Uuid>,
    pub converted_at: Option<chrono::DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl Quote {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.number.trim().is_empty() {
            return Err(DomainError::Validation("quote number is required".into()));
        }
        crate::money::CurrencyCode::new(&self.currency)?;
        if self.valid_until < self.issued_on {
            return Err(DomainError::Validation(
                "valid_until must not precede issued_on".into(),
            ));
        }
        if self.subtotal_minor < 0 || self.tax_minor < 0 || self.total_minor < 0 {
            return Err(DomainError::Validation(
                "quote amounts must not be negative".into(),
            ));
        }
        if self.total_minor != self.subtotal_minor + self.tax_minor {
            return Err(DomainError::Validation(
                "quote total must equal subtotal + tax".into(),
            ));
        }
        if !(0..=10_000).contains(&self.tax_rate_bps) {
            return Err(DomainError::Validation(
                "tax rate must be between 0 and 10000 basis points".into(),
            ));
        }
        Ok(())
    }

    /// Convertible: accepted, not yet converted, and within validity.
    pub fn convertible_on(&self, today: NaiveDate) -> bool {
        self.status == QuoteStatus::Accepted
            && self.converted_invoice_id.is_none()
            && today <= self.valid_until
    }
}

/// One quote line. `amount_minor` is the half-even product of
/// `quantity_milli × unit_amount_minor / 1000`; the API recomputes it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub quote_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub catalog_item_id: Option<uuid::Uuid>,
    pub description: String,
    pub unit: String,
    /// Quantity in thousandths of a unit (1.5 = 1500).
    pub quantity_milli: i64,
    pub unit_amount_minor: i64,
    pub taxable: bool,
    pub amount_minor: i64,
    pub position: i32,
}

impl QuoteLine {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.description.trim().is_empty() {
            return Err(DomainError::Validation(
                "quote line description is required".into(),
            ));
        }
        if self.quantity_milli <= 0 {
            return Err(DomainError::Validation(
                "quote line quantity must be positive".into(),
            ));
        }
        if self.unit_amount_minor < 0 {
            return Err(DomainError::Validation(
                "quote line unit amount must not be negative".into(),
            ));
        }
        if self.amount_minor < 0 {
            return Err(DomainError::Validation(
                "quote line amount must not be negative".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd_line(qty_milli: i64, unit: i64, taxable: bool) -> QuoteLine {
        QuoteLine {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            quote_id: uuid::Uuid::now_v7(),
            catalog_item_id: None,
            description: "line".into(),
            unit: "each".into(),
            quantity_milli: qty_milli,
            unit_amount_minor: unit,
            taxable,
            amount_minor: line_amount_minor(qty_milli, unit).unwrap(),
            position: 0,
        }
    }

    #[test]
    fn lifecycle_edges_are_data() {
        use QuoteStatus::*;
        assert!(QuoteStatus::transition(Draft, Sent).is_ok());
        assert!(QuoteStatus::transition(Sent, Accepted).is_ok());
        assert!(QuoteStatus::transition(Sent, Declined).is_ok());
        assert!(QuoteStatus::transition(Sent, Expired).is_ok());
        assert!(QuoteStatus::transition(Draft, Expired).is_ok());
        // No skipping send, no reviving terminals, no-op refused.
        assert!(QuoteStatus::transition(Draft, Accepted).is_err());
        assert!(QuoteStatus::transition(Accepted, Sent).is_err());
        assert!(QuoteStatus::transition(Sent, Sent).is_err());
        assert!(QuoteStatus::transition(Declined, Accepted).is_err());
    }

    #[test]
    fn status_strings_round_trip() {
        for s in QuoteStatus::ALL {
            assert_eq!(QuoteStatus::from_str(s.as_str()).unwrap(), s);
            assert_eq!(
                serde_json::to_value(s).unwrap(),
                serde_json::json!(s.as_str())
            );
        }
    }

    #[test]
    fn line_amounts_round_half_even() {
        // 3 × 333 = 999 → 999
        assert_eq!(line_amount_minor(3_000, 333).unwrap(), 999);
        // 0.5 × 1 cent = 0.5 → 0 (even)
        assert_eq!(line_amount_minor(500, 1).unwrap(), 0);
        // 1.5 × 1 cent = 1.5 → 2 (even)
        assert_eq!(line_amount_minor(1_500, 1).unwrap(), 2);
        // 2.5 × 1 cent = 2.5 → 2 (even)
        assert_eq!(line_amount_minor(2_500, 1).unwrap(), 2);
        assert!(line_amount_minor(0, 100).is_err());
        assert!(line_amount_minor(1_000, -1).is_err());
    }

    #[test]
    fn rollup_taxes_only_taxable_lines() {
        let lines = vec![
            usd_line(2_000, 100, true), // 200 taxable
            usd_line(1_000, 50, false), // 50 exempt
        ];
        let r = rollup(&lines, 800).unwrap(); // 8%
        assert_eq!(r.subtotal_minor, 250);
        assert_eq!(r.tax_minor, 16); // 200 * 0.08
        assert_eq!(r.total_minor, 266);
    }

    #[test]
    fn quote_validation_and_convertibility() {
        let today = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let mut q = Quote {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            number: "Q-000001".into(),
            client_id: uuid::Uuid::now_v7(),
            status: QuoteStatus::Draft,
            currency: "USD".into(),
            issued_on: today,
            valid_until: NaiveDate::from_ymd_opt(2026, 2, 10).unwrap(),
            notes: None,
            terms: None,
            subtotal_minor: 250,
            tax_rate_bps: 800,
            tax_minor: 16,
            total_minor: 266,
            sent_at: None,
            decided_at: None,
            converted_contract_id: None,
            converted_invoice_id: None,
            converted_at: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(q.validate().is_ok());
        assert!(!q.convertible_on(today));
        q.status = QuoteStatus::Accepted;
        assert!(q.convertible_on(today));
        assert!(!q.convertible_on(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()));

        q.total_minor = 999;
        assert!(q.validate().is_err());
    }
}
