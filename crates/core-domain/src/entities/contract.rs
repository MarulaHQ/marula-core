//! Contracts: the commercial agreements an MSP holds with its clients
//! (the private build tracker Phase 3 — contracts + recurring billing rules).
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
use crate::status::Priority;
use chrono::{NaiveDate, Utc};
use std::str::FromStr;

/// The supported commercial agreement shapes. Wave 39 adds the MSP
/// relationship types beyond the original four: per-unit recurring
/// (per-seat / per-device) and per-incident (per closed ticket) pricing.
/// Prepaid blocks, NTE caps, minimums, and escalators ride the existing
/// types as attached configuration rather than new shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ContractType {
    Recurring,
    TimeAndMaterials,
    Retainer,
    FixedFee,
    PerUnit,
    PerIncident,
}

impl ContractType {
    pub const ALL: [ContractType; 6] = [
        ContractType::Recurring,
        ContractType::TimeAndMaterials,
        ContractType::Retainer,
        ContractType::FixedFee,
        ContractType::PerUnit,
        ContractType::PerIncident,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ContractType::Recurring => "recurring",
            ContractType::TimeAndMaterials => "time_and_materials",
            ContractType::Retainer => "retainer",
            ContractType::FixedFee => "fixed_fee",
            ContractType::PerUnit => "per_unit",
            ContractType::PerIncident => "per_incident",
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
            "per_unit" => Ok(Self::PerUnit),
            "per_incident" => Ok(Self::PerIncident),
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
    /// Per-unit recurring: the price per unit per period, minor units.
    pub unit_price_minor: Option<i64>,
    /// Per-unit recurring: which inventory counts the units — `devices`
    /// (device_assets for the client) or `users` (org users billed to the
    /// client's contract). Defaults to `devices` at evaluation when unset.
    pub unit_basis: Option<UnitBasis>,
    /// Per-incident: the fixed price per closed ticket in the covered
    /// band, minor units.
    pub incident_price_minor: Option<i64>,
    /// Per-incident: which closed tickets price — the four-tier priority
    /// (`p1|p2|p3|p4`), or `any` for every closed ticket. `NULL` = any.
    pub incident_priority: Option<String>,
    /// Per-ticket capped bands (T&M, retainer, per-incident on the same
    /// contract type is fine too): when set, evaluation enforces the cap.
    /// Stored denormalized from `contract_caps` for read-path simplicity
    /// (the cap rows carry the authoritative state).
    pub notes: Option<String>,
    /// Client purchase-order reference printed on the invoices this
    /// contract produces (F: PO number rides the contract so runs carry
    /// it forward without per-invoice entry).
    pub po_number: Option<String>,
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
        money_non_negative(self.unit_price_minor, "unit_price_minor")?;
        money_non_negative(self.incident_price_minor, "incident_price_minor")?;
        if self.po_number.as_deref().map(str::len).unwrap_or(0) > 100 {
            return Err(DomainError::Validation(
                "po_number must be at most 100 characters".into(),
            ));
        }
        if let Some(basis) = &self.unit_basis {
            basis.validate()?;
        }
        if let Some(priority) = &self.incident_priority {
            if priority != "any" && Priority::from_str(priority).is_err() {
                return Err(DomainError::Validation(format!(
                    "invalid incident_priority: {priority}"
                )));
            }
        }
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
            ContractType::PerUnit => {
                recurring_triple()?;
                if self.unit_price_minor.is_none() {
                    return Err(DomainError::Validation(
                        "per_unit contracts require unit_price_minor".into(),
                    ));
                }
            }
            ContractType::PerIncident => {
                if self.incident_price_minor.is_none() {
                    return Err(DomainError::Validation(
                        "per_incident contracts require incident_price_minor".into(),
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
    /// Per-unit recurring: quantity × unit price for one period.
    PerUnitBase,
    /// Per-incident: one line per closed ticket in the covered band.
    PerIncident,
    /// A sold prepaid block's sale price (the hour-bank deposit).
    BlockSale,
    /// Labor drawn from a prepaid block (the draw-down is bookkeeping on
    /// the block; the line documents the consumption at the block rate).
    BlockDrawdown,
    /// Labor beyond the typed cap behavior's `overage_billable` election.
    CapOverage,
    /// Minimum-commitment true-up: tops the period up to the minimum.
    MinimumTrueup,
    /// Change-order amendment billed as its own line.
    ChangeOrder,
    /// SLA service credit: a NEGATIVE adjustment for a missed SLA
    /// (applied at verification, never silent — the description names the
    /// ticket and the missed target).
    SlaCredit,
    /// A scheduled escalator uplift line for the period it applies to.
    Escalator,
    /// A released project milestone or fixed-fee phase billed through
    /// this contract (Wave 39 lane 4). Description-keyed: the release
    /// ledger row's id rides the description in `[id]` brackets, the run
    /// flips the ledger row `billed` with the consuming invoice.
    ProjectRelease,
}

impl BillingLineKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            BillingLineKind::RecurringBase => "recurring_base",
            BillingLineKind::RetainerBase => "retainer_base",
            BillingLineKind::RetainerOverage => "retainer_overage",
            BillingLineKind::TimeMaterials => "time_materials",
            BillingLineKind::FixedFee => "fixed_fee",
            BillingLineKind::PerUnitBase => "per_unit_base",
            BillingLineKind::PerIncident => "per_incident",
            BillingLineKind::BlockSale => "block_sale",
            BillingLineKind::BlockDrawdown => "block_drawdown",
            BillingLineKind::CapOverage => "cap_overage",
            BillingLineKind::MinimumTrueup => "minimum_trueup",
            BillingLineKind::ChangeOrder => "change_order",
            BillingLineKind::SlaCredit => "sla_credit",
            BillingLineKind::Escalator => "escalator",
            BillingLineKind::ProjectRelease => "project_release",
        }
    }

    /// Period-identity kinds: at most one line per (contract, kind,
    /// period_start) ever — the run dedupes on that triple. Everything
    /// else dedupes on (kind, period_start, description).
    pub const fn is_period_kind(self) -> bool {
        matches!(
            self,
            BillingLineKind::RecurringBase
                | BillingLineKind::RetainerBase
                | BillingLineKind::RetainerOverage
                | BillingLineKind::PerUnitBase
                | BillingLineKind::MinimumTrueup
                | BillingLineKind::Escalator
        )
    }
}

impl FromStr for BillingLineKind {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "recurring_base" => Ok(Self::RecurringBase),
            "retainer_base" => Ok(Self::RetainerBase),
            "retainer_overage" => Ok(Self::RetainerOverage),
            "time_materials" => Ok(Self::TimeMaterials),
            "fixed_fee" => Ok(Self::FixedFee),
            "per_unit_base" => Ok(Self::PerUnitBase),
            "per_incident" => Ok(Self::PerIncident),
            "block_sale" => Ok(Self::BlockSale),
            "block_drawdown" => Ok(Self::BlockDrawdown),
            "cap_overage" => Ok(Self::CapOverage),
            "minimum_trueup" => Ok(Self::MinimumTrueup),
            "change_order" => Ok(Self::ChangeOrder),
            "sla_credit" => Ok(Self::SlaCredit),
            "escalator" => Ok(Self::Escalator),
            "project_release" => Ok(Self::ProjectRelease),
            other => Err(DomainError::Validation(format!(
                "invalid billing line kind: {other}"
            ))),
        }
    }
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
    /// NEGATIVE only for `sla_credit` lines — the one line kind whose
    /// signed amount is meaningful; everything else is ≥ 0 and the
    /// engine's i128 total fold accepts the mix.
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
    /// Block draw-down detail (prepaid blocks only; empty otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<BlockConsumption>))]
    pub block_consumption: Option<BlockConsumption>,
    /// Cap warnings (`warn` behavior): one human-readable marker per
    /// (cap, period) whose labor reached the cap. `stop`/`overage_billable`
    /// communicate through their line shapes instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cap_warnings: Vec<String>,
}

// --- Wave 39: MSP relationship-type configuration shapes ----------------------

/// Which org inventory counts a per-unit contract's quantity at
/// evaluation time (C: quantity × unit price per period, unit counts
/// drawn from the org's asset/user inventories at evaluation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum UnitBasis {
    /// One unit per active device asset billed to the contract's client.
    Devices,
    /// One unit per org user (per-seat billing).
    Users,
}

impl UnitBasis {
    pub const fn as_str(self) -> &'static str {
        match self {
            UnitBasis::Devices => "devices",
            UnitBasis::Users => "users",
        }
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            UnitBasis::Devices | UnitBasis::Users => Ok(()),
        }
    }
}

impl std::fmt::Display for UnitBasis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UnitBasis {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "devices" => Ok(Self::Devices),
            "users" => Ok(Self::Users),
            other => Err(DomainError::Validation(format!(
                "invalid unit basis: {other}"
            ))),
        }
    }
}

/// What evaluation does when a T&M contract's cap is reached (B: never a
/// silent cutoff — the behavior is a typed choice on the cap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CapReachedBehavior {
    /// Entries past the cap produce NO lines (work is absorbed).
    Stop,
    /// Entries past the cap still bill, and each capped period also
    /// emits a visible warning marker in the evaluation.
    Warn,
    /// Entries past the cap bill as `cap_overage` lines (usually at the
    /// overage rate rather than the card rate).
    OverageBillable,
}

impl CapReachedBehavior {
    pub const fn as_str(self) -> &'static str {
        match self {
            CapReachedBehavior::Stop => "stop",
            CapReachedBehavior::Warn => "warn",
            CapReachedBehavior::OverageBillable => "overage_billable",
        }
    }
}

impl std::fmt::Display for CapReachedBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CapReachedBehavior {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stop" => Ok(Self::Stop),
            "warn" => Ok(Self::Warn),
            "overage_billable" => Ok(Self::OverageBillable),
            other => Err(DomainError::Validation(format!(
                "invalid cap reached behavior: {other}"
            ))),
        }
    }
}

/// A T&M not-to-exceed cap. `scope` picks the boundary:
/// `period` enforces per billing period, `contract` enforces over the
/// contract's whole effective window.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractCap {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    /// `period` or `contract` (ARP-guarded CHECK in 0112).
    pub scope: String,
    /// The not-to-exceed amount, minor units of billable work per scope.
    pub cap_minor: i64,
    /// What happens when the cap is reached (typed, never silent).
    pub reached_behavior: CapReachedBehavior,
    /// Hourly rate for `overage_billable` beyond the cap; `None` bills
    /// the matched card/default rate.
    pub overage_rate_minor: Option<i64>,
    /// Optional work_type filter: the cap applies only to that bucket.
    pub work_type: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
}

/// A prepaid block (hour bank): sold up front, drawn down on use, with
/// expiry + rollover policies (A). Rows are immutable once sold —
/// rollover MINTS a successor block at expiry, it never edits the old
/// row (additive-ledger doctrine).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractBlock {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    /// Human label ("50-hour prepaid block, March 2026").
    pub label: String,
    /// Total minutes sold.
    pub total_minutes: i32,
    /// Sale price, minor units (usually total_minutes/60 × block rate).
    pub sale_price_minor: i64,
    /// Optional discounted hourly rate for labor priced against this
    /// block; `None` bills draws at the card/default rate with the block
    /// absorbing the minutes (a pure prepaid balance).
    pub block_rate_minor: Option<i64>,
    /// Last day draws may land inside this block.
    pub expires_on: Option<NaiveDate>,
    /// `expire` — unused minutes lapse at expiry; `rollover` — unused
    /// minutes mint a successor block (12 months) at expiry; `none` —
    /// no expiry, draws allowed indefinitely.
    pub rollover_policy: String,
    /// Draw-down ledger: each consumption row subtracts from the block.
    /// The remaining balance is computed, never stored.
    pub created_at: chrono::DateTime<Utc>,
}

/// A scheduled escalator: the contract's per-period price uplifts by
/// `uplift_bps` basis points each anniversary of the billing anchor
/// (E). Pure math at evaluation — no state.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractEscalator {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    /// Uplift per anniversary, basis points (300 = +3.0%).
    pub uplift_bps: i32,
    /// First anniversary that applies (defaults to evaluation at the
    /// anchor itself when `None` — the anchor anniversary).
    pub first_uplift_on: Option<NaiveDate>,
    pub created_at: chrono::DateTime<Utc>,
}

/// An approved change order: a billable amendment with its own lifecycle
/// (E). Riding existing approval/audit patterns: statuses
/// `draft → approved → billed`, rejected/cancelled terminal.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractChangeOrder {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    pub number: String,
    pub title: String,
    pub description: Option<String>,
    pub amount_minor: i64,
    /// `draft|approved|rejected|billed|cancelled`.
    pub status: String,
    pub approved_by: Option<uuid::Uuid>,
    pub approved_at: Option<chrono::DateTime<Utc>>,
    pub billed_invoice_id: Option<uuid::Uuid>,
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// An SLA service credit the admin or verification flow applies for a
/// missed SLA (E): becomes a NEGATIVE `sla_credit` line at the next
/// run window covering `applies_on`. Never silent — the record names
/// the ticket, the policy, and the missed target.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractSlaCredit {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: ContractId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<uuid::Uuid>,
    /// What was missed ("resolution target breached by 2d 4h").
    pub reason: String,
    /// Credit amount, minor units — stored POSITIVE, applied as a
    /// negative line (the ledger records magnitude; direction is the
    /// line kind's meaning).
    pub amount_minor: i64,
    pub currency: String,
    /// The billing window the credit lands in (inclusive-day semantics
    /// at evaluation: `from <= applies_on <= to`).
    pub applies_on: NaiveDate,
    /// `pending|applied|cancelled`.
    pub status: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub applied_invoice_id: Option<uuid::Uuid>,
    pub created_by: Option<uuid::Uuid>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// One block draw-down result the engine reports per draw: block
/// identity, the consuming time entry (annual ledger key), minutes
/// absorbed, and whether the draw spilled (used only by the
/// read-model/preview surfaces; the run persists draws).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BlockDraw {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub block_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub time_entry_id: Option<uuid::Uuid>,
    pub minutes: i32,
    pub spilled_minutes: i32,
    pub expired_unused_minutes: i32,
}

/// The per-period consumption summary a block contract's evaluation
/// carries back: draws in order, blocks that expired unused inside the
/// window (for depletion/expiry alerts), and any minutes that found no
/// open block (`unblocked_minutes` — they bill as T&M).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BlockConsumption {
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub draws: Vec<BlockDraw>,
    pub unblocked_minutes: i64,
}
