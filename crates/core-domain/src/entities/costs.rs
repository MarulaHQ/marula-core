//! Cost vocabulary: how a cost or a rental charge is reported, which record a
//! money line cites, how an organisation treats landed cost, and the recharge
//! rule it applies to a purchase made on a client's behalf (ADR 0035 decisions
//! 3, 6 and 7).
//!
//! Types and validation only. The columns these describe are nullable and
//! additive (migration 0234); a line that was written before the chain existed
//! stays uncited rather than being given an invented source.

use crate::error::DomainError;
use crate::money::CurrencyCode;
use uuid::Uuid;

/// Reporting classes (ADR 0035 §6.4). A charge has one class and posts once.
/// `inventory` is the only capitalised class: freight booked into the cost
/// basis is inventory, and the same freight in invoice-values mode is
/// `freight_in` on the project or ticket.
pub const REPORTING_CLASSES: [&str; 8] = [
    "inventory",
    "freight_in",
    "duty",
    "service_cost",
    "rental_expense",
    "ewaste_cost",
    "overhead",
    "cogs",
];

pub fn is_valid_reporting_class(class: &str) -> bool {
    REPORTING_CLASSES.contains(&class.trim())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReportingClass {
    Inventory,
    FreightIn,
    Duty,
    ServiceCost,
    RentalExpense,
    EwasteCost,
    Overhead,
    Cogs,
}

impl ReportingClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReportingClass::Inventory => "inventory",
            ReportingClass::FreightIn => "freight_in",
            ReportingClass::Duty => "duty",
            ReportingClass::ServiceCost => "service_cost",
            ReportingClass::RentalExpense => "rental_expense",
            ReportingClass::EwasteCost => "ewaste_cost",
            ReportingClass::Overhead => "overhead",
            ReportingClass::Cogs => "cogs",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "inventory" => Ok(Self::Inventory),
            "freight_in" => Ok(Self::FreightIn),
            "duty" => Ok(Self::Duty),
            "service_cost" => Ok(Self::ServiceCost),
            "rental_expense" => Ok(Self::RentalExpense),
            "ewaste_cost" => Ok(Self::EwasteCost),
            "overhead" => Ok(Self::Overhead),
            "cogs" => Ok(Self::Cogs),
            other => Err(DomainError::Validation(format!(
                "invalid reporting class: {other}"
            ))),
        }
    }

    /// Only `inventory` adds to a unit's cost basis. Everything else is an
    /// expense the accountant sees under its own class.
    pub const fn is_capitalised(self) -> bool {
        matches!(self, ReportingClass::Inventory)
    }
}

/// What a money line cites (ADR 0035 §9). `manual` is the honest source for a
/// line a person typed; every other kind names a row and carries its id.
pub const SOURCE_KINDS: [&str; 8] = [
    "stock_movement",
    "purchase_receipt_line",
    "ticket_cost_line",
    "invoice_line",
    "credit_line",
    "acquisition_cost",
    "rental_charge",
    "manual",
];

pub fn is_valid_source_kind(kind: &str) -> bool {
    SOURCE_KINDS.contains(&kind.trim())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    StockMovement,
    PurchaseReceiptLine,
    TicketCostLine,
    InvoiceLine,
    CreditLine,
    AcquisitionCost,
    RentalCharge,
    Manual,
}

impl SourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            SourceKind::StockMovement => "stock_movement",
            SourceKind::PurchaseReceiptLine => "purchase_receipt_line",
            SourceKind::TicketCostLine => "ticket_cost_line",
            SourceKind::InvoiceLine => "invoice_line",
            SourceKind::CreditLine => "credit_line",
            SourceKind::AcquisitionCost => "acquisition_cost",
            SourceKind::RentalCharge => "rental_charge",
            SourceKind::Manual => "manual",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "stock_movement" => Ok(Self::StockMovement),
            "purchase_receipt_line" => Ok(Self::PurchaseReceiptLine),
            "ticket_cost_line" => Ok(Self::TicketCostLine),
            "invoice_line" => Ok(Self::InvoiceLine),
            "credit_line" => Ok(Self::CreditLine),
            "acquisition_cost" => Ok(Self::AcquisitionCost),
            "rental_charge" => Ok(Self::RentalCharge),
            "manual" => Ok(Self::Manual),
            other => Err(DomainError::Validation(format!(
                "invalid source kind: {other}"
            ))),
        }
    }

    pub const fn is_manual(self) -> bool {
        matches!(self, SourceKind::Manual)
    }
}

/// The citation a money row carries: the kind of record that caused it and that
/// record's id. A `manual` line has no id; every other kind must have one.
/// An uncited row is `None` at the call site, not an empty source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MoneyLineSource {
    pub kind: SourceKind,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub id: Option<Uuid>,
}

impl MoneyLineSource {
    pub const fn manual() -> Self {
        Self {
            kind: SourceKind::Manual,
            id: None,
        }
    }

    pub const fn citing(kind: SourceKind, id: Uuid) -> Self {
        Self { kind, id: Some(id) }
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        match (self.kind.is_manual(), self.id) {
            (true, None) => Ok(()),
            (true, Some(_)) => Err(DomainError::Validation(
                "a manual source carries no source id".into(),
            )),
            (false, Some(_)) => Ok(()),
            (false, None) => Err(DomainError::Validation(
                "a cited source needs the id of the record it names".into(),
            )),
        }
    }
}

/// How an organisation treats landed cost (ADR 0035 §3). Switching modes
/// applies to new receipts; a historical layer is never rewritten.
pub const LANDED_COST_MODES: [&str; 2] = ["invoice_values", "landed_costs"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LandedCostMode {
    /// The receipt's invoice value is the cost. Freight posts as an expense.
    /// The default preserves today's behaviour: invoice values, no layers.
    #[default]
    InvoiceValues,
    /// Receipt plus capitalised acquisition-cost layers is the cost.
    LandedCosts,
}

impl LandedCostMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            LandedCostMode::InvoiceValues => "invoice_values",
            LandedCostMode::LandedCosts => "landed_costs",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "invoice_values" => Ok(Self::InvoiceValues),
            "landed_costs" => Ok(Self::LandedCosts),
            other => Err(DomainError::Validation(format!(
                "invalid landed cost mode: {other}"
            ))),
        }
    }
}

/// How a client purchase is recharged (settled decision 1, ADR 0035 §2).
/// Percentage and fixed are both built; percentage is the default.
pub const RECHARGE_RULE_KINDS: [&str; 4] = ["at_cost", "markup_bps", "fixed", "none"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RechargeRuleKind {
    AtCost,
    /// The default kind: a percentage markup, at zero until configured.
    #[default]
    MarkupBps,
    Fixed,
    None,
}

impl RechargeRuleKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            RechargeRuleKind::AtCost => "at_cost",
            RechargeRuleKind::MarkupBps => "markup_bps",
            RechargeRuleKind::Fixed => "fixed",
            RechargeRuleKind::None => "none",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        match raw.trim() {
            "at_cost" => Ok(Self::AtCost),
            "markup_bps" => Ok(Self::MarkupBps),
            "fixed" => Ok(Self::Fixed),
            "none" => Ok(Self::None),
            other => Err(DomainError::Validation(format!(
                "invalid recharge rule kind: {other}"
            ))),
        }
    }

    pub const fn is_default(self) -> bool {
        matches!(self, RechargeRuleKind::MarkupBps)
    }
}

/// The recharge rule in force for one line. The resolved rule is stored beside
/// the charge it produced, so a later policy change cannot rewrite an issued
/// recharge. All money is integer minor units and the percentage is basis
/// points, never a float.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RechargeRule {
    pub kind: RechargeRuleKind,
    /// `markup_bps` only: 2000 = 20% above cost. 0 means at cost.
    pub markup_bps: Option<i32>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub fixed_minor: Option<i64>,
    pub currency: Option<String>,
}

impl RechargeRule {
    /// The organisation default: percentage at zero, which recharges at cost
    /// until someone configures a markup.
    pub fn default_markup() -> Self {
        Self {
            kind: RechargeRuleKind::MarkupBps,
            markup_bps: Some(0),
            fixed_minor: None,
            currency: None,
        }
    }

    pub fn at_cost() -> Self {
        Self {
            kind: RechargeRuleKind::AtCost,
            markup_bps: None,
            fixed_minor: None,
            currency: None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self.kind, RechargeRuleKind::None)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        match self.kind {
            RechargeRuleKind::AtCost | RechargeRuleKind::None => {
                if self.markup_bps.is_some()
                    || self.fixed_minor.is_some()
                    || self.currency.is_some()
                {
                    return Err(DomainError::Validation(
                        "an at_cost or none recharge rule carries no amount".into(),
                    ));
                }
            }
            RechargeRuleKind::MarkupBps => {
                let bps = self.markup_bps.ok_or_else(|| {
                    DomainError::Validation("a percentage recharge needs its basis points".into())
                })?;
                if !(0..=100_000).contains(&bps) {
                    return Err(DomainError::Validation(
                        "recharge markup must be between 0 and 100000 basis points".into(),
                    ));
                }
                if self.fixed_minor.is_some() {
                    return Err(DomainError::Validation(
                        "a percentage recharge has no fixed amount".into(),
                    ));
                }
            }
            RechargeRuleKind::Fixed => {
                let amount = self.fixed_minor.ok_or_else(|| {
                    DomainError::Validation("a fixed recharge needs its amount".into())
                })?;
                if amount < 0 {
                    return Err(DomainError::Validation(
                        "a fixed recharge amount must not be negative".into(),
                    ));
                }
                let currency = self.currency.as_deref().ok_or_else(|| {
                    DomainError::Validation("a fixed recharge needs its currency".into())
                })?;
                CurrencyCode::new(currency)?;
                if self.markup_bps.is_some() {
                    return Err(DomainError::Validation(
                        "a fixed recharge has no percentage".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Default for RechargeRule {
    fn default() -> Self {
        Self::default_markup()
    }
}

/// The organisation's inventory money policy, read from the `orgs` columns
/// migration 0234 adds. The defaults are the migration defaults, so an org that
/// never configured anything reads the same thing the database holds.
///
/// The stored fields are the raw columns, not a resolved rule: switching the
/// kind does not clear the other kind's value, and the resolved rule is what a
/// purchase produces. That is why the percentage is always present and the
/// fixed amount and currency are optional.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InventoryPolicy {
    pub landed_cost_mode: LandedCostMode,
    pub recharge_rule_kind: RechargeRuleKind,
    /// Basis points; 0 means at cost. Stored even while another kind is active.
    pub recharge_markup_bps: i32,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub recharge_fixed_minor: Option<i64>,
    pub recharge_currency: Option<String>,
}

impl InventoryPolicy {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !(0..=100_000).contains(&self.recharge_markup_bps) {
            return Err(DomainError::Validation(
                "recharge markup must be between 0 and 100000 basis points".into(),
            ));
        }
        if self.recharge_fixed_minor.is_some_and(|amount| amount < 0) {
            return Err(DomainError::Validation(
                "a fixed recharge amount must not be negative".into(),
            ));
        }
        if let Some(currency) = &self.recharge_currency {
            CurrencyCode::new(currency)?;
        }
        if self.recharge_rule_kind == RechargeRuleKind::Fixed
            && (self.recharge_fixed_minor.is_none() || self.recharge_currency.is_none())
        {
            return Err(DomainError::Validation(
                "a fixed recharge default needs its amount and currency".into(),
            ));
        }
        Ok(())
    }

    /// The rule a client purchase resolves to when no ticket or client
    /// agreement overrides it. The other kind's stored value is dropped here,
    /// so the resolved rule carries exactly one amount.
    pub fn resolved_recharge_rule(&self) -> Result<RechargeRule, DomainError> {
        self.validate()?;
        match self.recharge_rule_kind {
            RechargeRuleKind::MarkupBps => Ok(RechargeRule {
                kind: RechargeRuleKind::MarkupBps,
                markup_bps: Some(self.recharge_markup_bps),
                fixed_minor: None,
                currency: None,
            }),
            RechargeRuleKind::Fixed => Ok(RechargeRule {
                kind: RechargeRuleKind::Fixed,
                markup_bps: None,
                fixed_minor: self.recharge_fixed_minor,
                currency: self.recharge_currency.clone(),
            }),
            kind @ (RechargeRuleKind::AtCost | RechargeRuleKind::None) => Ok(RechargeRule {
                kind,
                markup_bps: None,
                fixed_minor: None,
                currency: None,
            }),
        }
    }
}

impl Default for InventoryPolicy {
    /// The migration defaults: invoice values and a percentage at zero.
    fn default() -> Self {
        Self {
            landed_cost_mode: LandedCostMode::default(),
            recharge_rule_kind: RechargeRuleKind::default(),
            recharge_markup_bps: 0,
            recharge_fixed_minor: None,
            recharge_currency: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reporting_class_round_trips_and_only_inventory_capitalises() {
        for class in REPORTING_CLASSES {
            let parsed = ReportingClass::parse(class).expect("known class parses");
            assert_eq!(parsed.as_str(), class);
            assert!(is_valid_reporting_class(class));
        }
        assert_eq!(
            REPORTING_CLASSES
                .iter()
                .filter(|c| ReportingClass::parse(c).unwrap().is_capitalised())
                .copied()
                .collect::<Vec<_>>(),
            vec!["inventory"]
        );
        assert!(ReportingClass::parse("capex").is_err());
    }

    #[test]
    fn a_source_is_either_manual_or_cites_an_id() {
        MoneyLineSource::manual().validate().unwrap();
        let id = Uuid::now_v7();
        MoneyLineSource::citing(SourceKind::StockMovement, id)
            .validate()
            .unwrap();
        assert!(!MoneyLineSource::citing(SourceKind::StockMovement, id)
            .kind
            .is_manual());
        assert!(MoneyLineSource {
            kind: SourceKind::StockMovement,
            id: None,
        }
        .validate()
        .is_err());
        assert!(MoneyLineSource {
            kind: SourceKind::Manual,
            id: Some(id),
        }
        .validate()
        .is_err());
    }

    #[test]
    fn landed_cost_defaults_to_invoice_values() {
        assert_eq!(LandedCostMode::default(), LandedCostMode::InvoiceValues);
        for mode in LANDED_COST_MODES {
            assert_eq!(LandedCostMode::parse(mode).unwrap().as_str(), mode);
        }
        assert!(LandedCostMode::parse("fifo").is_err());
    }

    #[test]
    fn the_percentage_recharge_is_the_default() {
        let rule = RechargeRule::default();
        assert_eq!(rule.kind, RechargeRuleKind::MarkupBps);
        assert_eq!(rule.markup_bps, Some(0));
        rule.validate().unwrap();
        assert!(rule.kind.is_default());
    }

    #[test]
    fn each_recharge_kind_requires_its_own_amount() {
        RechargeRule::at_cost().validate().unwrap();

        let mut fixed = RechargeRule {
            kind: RechargeRuleKind::Fixed,
            markup_bps: None,
            fixed_minor: Some(2_500),
            currency: Some("USD".into()),
        };
        fixed.validate().unwrap();

        fixed.currency = None;
        assert!(fixed.validate().is_err(), "a fixed amount needs a currency");

        let percentage = RechargeRule {
            kind: RechargeRuleKind::MarkupBps,
            markup_bps: Some(2_000),
            fixed_minor: None,
            currency: None,
        };
        percentage.validate().unwrap();

        let over_charged = RechargeRule {
            kind: RechargeRuleKind::MarkupBps,
            markup_bps: Some(100_001),
            fixed_minor: None,
            currency: None,
        };
        assert!(over_charged.validate().is_err());

        let contradictory = RechargeRule {
            kind: RechargeRuleKind::AtCost,
            markup_bps: Some(500),
            fixed_minor: None,
            currency: None,
        };
        assert!(
            contradictory.validate().is_err(),
            "at_cost cannot carry a markup"
        );
    }

    #[test]
    fn the_default_policy_is_invoice_values_and_at_cost() {
        let policy = InventoryPolicy::default();
        policy.validate().unwrap();
        assert_eq!(policy.landed_cost_mode, LandedCostMode::InvoiceValues);
        assert_eq!(policy.recharge_markup_bps, 0);

        let resolved = policy.resolved_recharge_rule().unwrap();
        assert_eq!(resolved.kind, RechargeRuleKind::MarkupBps);
        assert_eq!(resolved.markup_bps, Some(0));
        assert_eq!(resolved.fixed_minor, None);
    }

    #[test]
    fn a_fixed_policy_default_resolves_to_one_amount_only() {
        let policy = InventoryPolicy {
            recharge_rule_kind: RechargeRuleKind::Fixed,
            recharge_markup_bps: 1_500,
            recharge_fixed_minor: Some(2_000),
            recharge_currency: Some("USD".into()),
            ..InventoryPolicy::default()
        };
        policy.validate().unwrap();
        let resolved = policy.resolved_recharge_rule().unwrap();
        assert_eq!(resolved.kind, RechargeRuleKind::Fixed);
        assert_eq!(resolved.fixed_minor, Some(2_000));
        assert_eq!(
            resolved.markup_bps, None,
            "the stored percentage is not part of a resolved fixed rule"
        );

        let incomplete = InventoryPolicy {
            recharge_currency: None,
            ..policy
        };
        assert!(incomplete.validate().is_err());
    }
}
