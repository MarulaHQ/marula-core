//! Contracts: the commercial agreements an MSP holds with its clients
//! (TASKLIST Phase 3 — contracts + recurring billing rules).
//!
//! Four contract types, each carrying the billing configuration its
//! recurring-billing rules need:
//!
//! * [`ContractType::Recurring`] — a fixed price per billing period
//!   (managed services). Requires `amount_minor`, `billing_frequency`,
//!   `billing_anchor`.
//! * [`ContractType::TimeAndMaterials`] — labor billed from time entries
//!   at contract rates. Requires `hourly_rate_minor` (the default rate);
//!   the per-`work_type` rate card lives in [`ContractRate`] rows.
//! * [`ContractType::Retainer`] — a fixed price per period that includes a
//!   block of hours; consumption beyond the block bills at the overage
//!   rate. Requires the recurring triple plus `included_minutes` and
//!   `overage_rate_minor`.
//! * [`ContractType::FixedFee`] — a dated fee schedule; each
//!   [`ContractFixedFee`] becomes a candidate line when its `due_on` falls
//!   in the evaluation window.
//!
//! Effective dating: `effective_from` / `effective_until` bound when the
//! agreement generates billable candidates; billing periods derive
//! deterministically from `billing_anchor` + `billing_frequency`
//! (see core-billing's rules engine — period math is pure, invoice
//! persistence is deliberately absent until Wave 9).

use crate::error::DomainError;
use crate::ids::{ClientId, ContractId, OrgId};
use chrono::{NaiveDate, Utc};
use std::str::FromStr;

/// The four supported commercial agreement shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ContractType {
    Recurring,
    TimeAndMaterials,
    Retainer,
    FixedFee,
}

impl ContractType {
    pub const ALL: [ContractType; 4] = [
        ContractType::Recurring,
        ContractType::TimeAndMaterials,
        ContractType::Retainer,
        ContractType::FixedFee,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ContractType::Recurring => "recurring",
            ContractType::TimeAndMaterials => "time_and_materials",
            ContractType::Retainer => "retainer",
            ContractType::FixedFee => "fixed_fee",
        }
    }
}

impl std::fmt::Display for ContractType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContractType {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "recurring" => Ok(Self::Recurring),
            "time_and_materials" => Ok(Self::TimeAndMaterials),
            "retainer" => Ok(Self::Retainer),
            "fixed_fee" => Ok(Self::FixedFee),
            other => Err(DomainError::Validation(format!(
                "invalid contract type: {other}"
            ))),
        }
    }
}

/// Lifecycle. `draft` contracts evaluate in previews (plan before
/// activation) but Wave 9's invoice runs will refuse them; `cancelled`
/// contracts never evaluate. `ended` keeps evaluating candidates whose
/// dates fall inside the (closed) effective window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    Draft,
    Active,
    Ended,
    Cancelled,
}

impl ContractStatus {
    pub const ALL: [ContractStatus; 4] = [
        ContractStatus::Draft,
        ContractStatus::Active,
        ContractStatus::Ended,
        ContractStatus::Cancelled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ContractStatus::Draft => "draft",
            ContractStatus::Active => "active",
            ContractStatus::Ended => "ended",
            ContractStatus::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for ContractStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContractStatus {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "ended" => Ok(Self::Ended),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(DomainError::Validation(format!(
                "invalid contract status: {other}"
            ))),
        }
    }
}

/// Billing period length. Period boundaries derive from the contract's
/// `billing_anchor`: the anchor is the first period's start date and each
/// subsequent period starts one frequency step later (calendar-aware
/// clamping lives in core-billing's period arithmetic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BillingFrequency {
    Weekly,
    Monthly,
    Quarterly,
    Semiannual,
    Annual,
}

impl BillingFrequency {
    pub const ALL: [BillingFrequency; 5] = [
        BillingFrequency::Weekly,
        BillingFrequency::Monthly,
        BillingFrequency::Quarterly,
        BillingFrequency::Semiannual,
        BillingFrequency::Annual,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            BillingFrequency::Weekly => "weekly",
            BillingFrequency::Monthly => "monthly",
            BillingFrequency::Quarterly => "quarterly",
            BillingFrequency::Semiannual => "semiannual",
            BillingFrequency::Annual => "annual",
        }
    }

    /// How many months one period spans (0 for weekly, which is day-based).
    pub const fn months(self) -> u32 {
        match self {
            BillingFrequency::Weekly => 0,
            BillingFrequency::Monthly => 1,
            BillingFrequency::Quarterly => 3,
            BillingFrequency::Semiannual => 6,
            BillingFrequency::Annual => 12,
        }
    }
}

impl std::fmt::Display for BillingFrequency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BillingFrequency {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "weekly" => Ok(Self::Weekly),
            "monthly" => Ok(Self::Monthly),
            "quarterly" => Ok(Self::Quarterly),
            "semiannual" => Ok(Self::Semiannual),
            "annual" => Ok(Self::Annual),
            other => Err(DomainError::Validation(format!(
                "invalid billing frequency: {other}"
            ))),
        }
    }
}

/// A commercial agreement with one client. Money fields are integer minor
/// units (cents for USD) — floats are forbidden near amounts (§5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Contract {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: ContractId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    /// Human contract number, unique per org (e.g. "C-2026-014").
    pub number: String,
    pub name: String,
    pub contract_type: ContractType,
    pub status: ContractStatus,
    /// ISO 4217 code; every amount on the contract uses this currency.
    pub currency: String,
    /// First day the agreement can generate billable candidates.
    pub effective_from: NaiveDate,
    /// Last day it can; `None` = open-ended.
    pub effective_until: Option<NaiveDate>,
    /// Required for recurring/retainer; the period cadence.
    pub billing_frequency: Option<BillingFrequency>,
    /// Required for recurring/retainer; the first billing period's start.
    pub billing_anchor: Option<NaiveDate>,
    /// Per-period price (recurring/retainer), minor units.
    pub amount_minor: Option<i64>,
    /// Default hourly rate for T&M when no rate card row matches the
    /// entry's `work_type`, minor units per hour.
    pub hourly_rate_minor: Option<i64>,
    /// Retainer: labor minutes included in each period's base price.
    pub included_minutes: Option<i32>,
    /// Retainer: hourly rate for minutes beyond the included block.
    pub overage_rate_minor: Option<i64>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl Contract {
    /// The type-conditional invariants every persisted contract must hold.
    /// Called by the API layer before create/update; deliberately pure so
    /// imports and future agents reuse the same rules.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.number.trim().is_empty() {
            return Err(DomainError::Validation(
                "contract number is required".into(),
            ));
        }
        if self.number.len() > 100 {
            return Err(DomainError::Validation(
                "contract number must be at most 100 characters".into(),
            ));
        }
        if self.name.trim().is_empty() {
            return Err(DomainError::Validation("contract name is required".into()));
        }
        crate::money::CurrencyCode::new(&self.currency)?;
        if let Some(until) = self.effective_until {
            if until < self.effective_from {
                return Err(DomainError::Validation(
                    "effective_until must not precede effective_from".into(),
                ));
            }
        }
        let money_non_negative = |v: Option<i64>, field: &str| -> Result<(), DomainError> {
            if let Some(amount) = v {
                if amount < 0 {
                    return Err(DomainError::Validation(format!(
                        "{field} must not be negative"
                    )));
                }
            }
            Ok(())
        };
        money_non_negative(self.amount_minor, "amount_minor")?;
        money_non_negative(self.hourly_rate_minor, "hourly_rate_minor")?;
        money_non_negative(self.overage_rate_minor, "overage_rate_minor")?;
        if let Some(minutes) = self.included_minutes {
            if minutes < 0 {
                return Err(DomainError::Validation(
                    "included_minutes must not be negative".into(),
                ));
            }
        }
        let recurring_triple = || -> Result<(), DomainError> {
            if self.billing_frequency.is_none() || self.billing_anchor.is_none() {
                return Err(DomainError::Validation(format!(
                    "{} contracts require billing_frequency and billing_anchor",
                    self.contract_type
                )));
            }
            Ok(())
        };
        match self.contract_type {
            ContractType::Recurring => {
                recurring_triple()?;
                if self.amount_minor.is_none() {
                    return Err(DomainError::Validation(
                        "recurring contracts require amount_minor".into(),
                    ));
                }
            }
            ContractType::Retainer => {
                recurring_triple()?;
                if self.amount_minor.is_none()
                    || self.included_minutes.is_none()
                    || self.overage_rate_minor.is_none()
                {
                    return Err(DomainError::Validation(
                        "retainer contracts require amount_minor, included_minutes and overage_rate_minor"
                            .into(),
                    ));
                }
            }
            ContractType::TimeAndMaterials => {
                if self.hourly_rate_minor.is_none() {
                    return Err(DomainError::Validation(
                        "time_and_materials contracts require hourly_rate_minor".into(),
                    ));
                }
            }
            ContractType::FixedFee => {}
        }
        Ok(())
    }
}

/// T&M rate card row: the hourly rate for time entries whose `work_type`
/// matches `work_type`. Entries with no matching row bill at the
/// contract's default `hourly_rate_minor`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractRate {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    pub work_type: String,
    /// Hourly rate in minor units.
    pub rate_minor: i64,
    pub created_at: chrono::DateTime<Utc>,
}

/// Fixed-fee schedule entry: becomes a candidate line when `due_on` falls
/// inside the evaluation window (and the effective window).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractFixedFee {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    pub label: String,
    pub amount_minor: i64,
    pub due_on: NaiveDate,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
}

// --- recurring-billing rule-engine wire types --------------------------------
// These are the shapes the preview endpoints return; Wave 9's invoice run
// will persist lines derived from the same engine output.

/// Which billing rule produced a candidate line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BillingLineKind {
    /// Recurring: the per-period base price.
    RecurringBase,
    /// Retainer: the per-period base price (included block untouched).
    RetainerBase,
    /// Retainer: consumption beyond the included block at the overage rate.
    RetainerOverage,
    /// T&M labor, one line per `work_type` bucket.
    TimeMaterials,
    /// Fixed fee whose due date fell in the window.
    FixedFee,
}

/// One draft invoice-line candidate. This is a PREVIEW shape: nothing
/// persists until Wave 9's invoice run.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BillingCandidateLine {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    pub contract_number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    pub kind: BillingLineKind,
    /// The billing period (recurring/retainer) or the effective slice of
    /// the evaluation window (T&M) or the fee's due date mirrored into
    /// both ends (fixed fee). Inclusive start, exclusive end for
    /// period-derived lines.
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub description: String,
    /// Labor minutes on the line (T&M/overage); 0 for amount lines.
    pub quantity_minutes: i64,
    /// Unit price in minor units: the hourly rate for labor lines, the
    /// full amount otherwise.
    pub unit_amount_minor: i64,
    /// Line total in minor units (currency on the parent evaluation).
    pub amount_minor: i64,
}

/// The result of evaluating one contract's billing rules over a window.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BillingEvaluation {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    pub contract_number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    pub contract_type: ContractType,
    pub currency: String,
    /// The evaluation window (inclusive bounds).
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub lines: Vec<BillingCandidateLine>,
    /// Sum of line amounts, minor units.
    pub total_minor: i64,
}
