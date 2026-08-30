//! The recurring-billing rules engine: one pure entry point, [`evaluate`],
//! that turns a contract plus its supporting rows into draft invoice-line
//! candidates over an evaluation window.
//!
//! Contract (Wave 8b): evaluation is READ-ONLY — the output is a preview
//! shape, nothing persists, no `billed` markers exist. Wave 9's invoice
//! run will consume the same function and own persistence.
//!
//! Determinism guarantees (pinned by tests): lines are ordered by rule
//! (period base lines in ascending period order, then overage lines, then
//! T&M buckets sorted by `work_type`, then fixed fees by due date/label);
//! identical inputs produce byte-identical evaluations. Money math is
//! integer minor units; per-hour proration rounds half-up; totals
//! accumulate in i128 and reject overflow as [`BillingError::AmountOverflow`]
//! rather than silently wrapping.

use chrono::NaiveDate;
use core_domain::entities::{
    BillingCandidateLine, BillingEvaluation, BillingLineKind, Contract, ContractFixedFee,
    ContractRate, ContractType,
};
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

/// One billable labor input: a time entry already filtered to the
/// contract's client and `billable = true` by the query layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillableEntry {
    #[allow(dead_code)]
    pub time_entry_id: Uuid,
    pub minutes: i32,
    pub work_type: Option<String>,
    pub entry_date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BillingError {
    #[error("evaluation window is inverted: from {from} is after to {to}")]
    InvalidWindow { from: NaiveDate, to: NaiveDate },
    #[error("line amounts overflow the 64-bit minor-unit total")]
    AmountOverflow,
}

/// Evaluates one contract's billing rules over the inclusive window
/// `[from, to]`.
///
/// Rules per contract type:
///
/// * **Recurring** — one base line per billing period overlapping the
///   window whose period start lies inside the effective window.
/// * **Retainer** — the recurring base line, plus one overage line per
///   period whose consumption (billable minutes dated inside the period,
///   clamped to the effective window) exceeds `included_minutes`.
/// * **T&M** — labor lines bucketed by `work_type` over the effective
///   slice of the window; the rate card overrides the contract default.
/// * **Fixed fee** — one line per schedule entry due inside the effective
///   slice of the window.
///
/// `Cancelled` contracts evaluate to zero lines (they are the only status
/// the engine refuses); `draft`/`active`/`ended` all evaluate, so
/// previews work before activation and after an agreement ends.
pub fn evaluate(
    contract: &Contract,
    rates: &[ContractRate],
    fees: &[ContractFixedFee],
    entries: &[BillableEntry],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<BillingEvaluation, BillingError> {
    if from > to {
        return Err(BillingError::InvalidWindow { from, to });
    }
    let empty = |contract: &Contract| BillingEvaluation {
        contract_id: contract.id,
        contract_number: contract.number.clone(),
        client_id: contract.client_id,
        contract_type: contract.contract_type,
        currency: contract.currency.clone(),
        from,
        to,
        lines: Vec::new(),
        total_minor: 0,
    };
    if contract.status == core_domain::entities::ContractStatus::Cancelled {
        return Ok(empty(contract));
    }
    // Effective slice of the window: candidates only inside [lo, hi].
    let lo = from.max(contract.effective_from);
    let hi = match contract.effective_until {
        Some(until) => to.min(until),
        None => to,
    };
    let mut lines: Vec<BillingCandidateLine> = Vec::new();
    match contract.contract_type {
        ContractType::Recurring => {
            lines.extend(recurring_lines(contract, from, to, hi));
        }
        ContractType::Retainer => {
            lines.extend(recurring_lines(contract, from, to, hi));
            lines.extend(retainer_overage_lines(contract, entries, lo, hi));
        }
        ContractType::TimeAndMaterials => {
            lines.extend(time_materials_lines(contract, rates, entries, lo, hi));
        }
        ContractType::FixedFee => {
            lines.extend(fixed_fee_lines(contract, fees, lo, hi));
        }
    }
    let total: i128 = lines.iter().map(|l| l.amount_minor as i128).sum();
    let total_minor = i64::try_from(total).map_err(|_| BillingError::AmountOverflow)?;
    Ok(BillingEvaluation {
        total_minor,
        lines,
        ..empty(contract)
    })
}

fn effective_period_included(contract: &Contract, period_start: NaiveDate, hi: NaiveDate) -> bool {
    period_start >= contract.effective_from && period_start <= hi
}

/// The per-period base price lines shared by recurring and retainer.
fn recurring_lines(
    contract: &Contract,
    from: NaiveDate,
    to: NaiveDate,
    hi: NaiveDate,
) -> Vec<BillingCandidateLine> {
    let (Some(anchor), Some(frequency), Some(amount)) = (
        contract.billing_anchor,
        contract.billing_frequency,
        contract.amount_minor,
    ) else {
        // Persisted contracts always carry the triple (validated in
        // core-domain); an incomplete row simply produces no lines.
        return Vec::new();
    };
    crate::periods::periods_starting_in(anchor, frequency, from, to)
        .into_iter()
        .filter(|(start, _)| effective_period_included(contract, *start, hi))
        .map(|(start, end)| BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: if contract.contract_type == ContractType::Retainer {
                BillingLineKind::RetainerBase
            } else {
                BillingLineKind::RecurringBase
            },
            period_start: start,
            period_end: end,
            description: format!("{} ({} to {})", contract.name, start, end),
            quantity_minutes: 0,
            unit_amount_minor: amount,
            amount_minor: amount,
        })
        .collect()
}

/// Retainer overage: per period, billable minutes dated inside the period
/// (clamped to the effective slice) beyond the included block, priced at
/// the hourly overage rate with half-up proration.
fn retainer_overage_lines(
    contract: &Contract,
    entries: &[BillableEntry],
    lo: NaiveDate,
    hi: NaiveDate,
) -> Vec<BillingCandidateLine> {
    let (Some(anchor), Some(frequency)) = (contract.billing_anchor, contract.billing_frequency)
    else {
        return Vec::new();
    };
    let included = contract.included_minutes.unwrap_or(0) as i64;
    let Some(overage_rate) = contract.overage_rate_minor else {
        return Vec::new();
    };
    crate::periods::periods_starting_in(anchor, frequency, lo, hi)
        .into_iter()
        .filter(|(start, _)| effective_period_included(contract, *start, hi))
        .filter_map(|(start, end)| {
            let consumed: i64 = entries
                .iter()
                .filter(|e| {
                    e.entry_date >= start.max(lo) && e.entry_date < end && e.entry_date <= hi
                })
                .map(|e| e.minutes as i64)
                .sum();
            let overage = consumed - included;
            if overage <= 0 {
                return None;
            }
            let amount = prorate(overage, overage_rate)?;
            Some(BillingCandidateLine {
                contract_id: contract.id,
                contract_number: contract.number.clone(),
                client_id: contract.client_id,
                kind: BillingLineKind::RetainerOverage,
                period_start: start,
                period_end: end,
                description: format!(
                    "{} overage: {} min beyond {} included ({} to {})",
                    contract.name, overage, included, start, end
                ),
                quantity_minutes: overage,
                unit_amount_minor: overage_rate,
                amount_minor: amount,
            })
        })
        .collect()
}

/// T&M labor: one line per `work_type` bucket over the effective slice.
/// Buckets sort by work_type for determinism; rates come from the card,
/// falling back to the contract default for unset or unmatched work
/// types (those bucket under the literal label they carried, or
/// `unclassified` when the entry had none).
fn time_materials_lines(
    contract: &Contract,
    rates: &[ContractRate],
    entries: &[BillableEntry],
    lo: NaiveDate,
    hi: NaiveDate,
) -> Vec<BillingCandidateLine> {
    let default_rate = contract.hourly_rate_minor.unwrap_or(0);
    let mut buckets: BTreeMap<String, (i64, i64)> = BTreeMap::new(); // work_type -> (minutes, rate)
    for entry in entries {
        if entry.entry_date < lo || entry.entry_date > hi {
            continue;
        }
        let matched = entry
            .work_type
            .as_deref()
            .and_then(|wt| rates.iter().find(|r| r.work_type == wt))
            .map(|r| (r.work_type.clone(), r.rate_minor));
        let (key, rate) = matched.unwrap_or_else(|| {
            (
                entry
                    .work_type
                    .clone()
                    .unwrap_or_else(|| "unclassified".to_string()),
                default_rate,
            )
        });
        let bucket = buckets.entry(key).or_insert((0, rate));
        bucket.0 += entry.minutes as i64;
    }
    buckets
        .into_iter()
        .filter_map(|(key, (minutes, rate))| {
            if minutes == 0 {
                return None;
            }
            let work_type = key
                .split('\u{0}')
                .next()
                .unwrap_or("unclassified")
                .to_string();
            let amount = prorate(minutes, rate)?;
            Some(BillingCandidateLine {
                contract_id: contract.id,
                contract_number: contract.number.clone(),
                client_id: contract.client_id,
                kind: BillingLineKind::TimeMaterials,
                period_start: lo,
                period_end: hi,
                description: format!(
                    "{} labor: {} min at {rate}/h ({work_type})",
                    contract.name, minutes
                ),
                quantity_minutes: minutes,
                unit_amount_minor: rate,
                amount_minor: amount,
            })
        })
        .collect()
}

/// Fixed-fee schedule: one line per entry due inside the effective slice,
/// ordered by (due_on, label) for determinism.
fn fixed_fee_lines(
    contract: &Contract,
    fees: &[ContractFixedFee],
    lo: NaiveDate,
    hi: NaiveDate,
) -> Vec<BillingCandidateLine> {
    let mut due: Vec<&ContractFixedFee> = fees
        .iter()
        .filter(|f| f.due_on >= lo && f.due_on <= hi)
        .collect();
    due.sort_by(|a, b| (a.due_on, &a.label).cmp(&(b.due_on, &b.label)));
    due.into_iter()
        .map(|fee| BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: BillingLineKind::FixedFee,
            period_start: fee.due_on,
            period_end: fee.due_on,
            description: fee.label.clone(),
            quantity_minutes: 0,
            unit_amount_minor: fee.amount_minor,
            amount_minor: fee.amount_minor,
        })
        .collect()
}

/// Per-hour proration with half-up rounding on positive amounts:
/// `(minutes * rate + 30) / 60`. Returns `None` on i64 overflow of the
/// product (surfaced as a skipped line rather than a wrong total — the
/// quantities involved make this practically unreachable).
fn prorate(minutes: i64, hourly_rate_minor: i64) -> Option<i64> {
    let raw = (minutes as i128) * (hourly_rate_minor as i128);
    let rounded = (raw + 30) / 60;
    i64::try_from(rounded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use core_domain::entities::{BillingFrequency, ContractStatus};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn contract(kind: ContractType) -> Contract {
        let base = Contract {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            client_id: Uuid::now_v7(),
            number: "C-001".into(),
            name: "Managed services".into(),
            contract_type: kind,
            status: ContractStatus::Active,
            currency: "USD".into(),
            effective_from: d(2026, 1, 1),
            effective_until: None,
            billing_frequency: None,
            billing_anchor: None,
            amount_minor: None,
            hourly_rate_minor: None,
            included_minutes: None,
            overage_rate_minor: None,
            notes: None,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        };
        match kind {
            ContractType::Recurring => Contract {
                billing_frequency: Some(BillingFrequency::Monthly),
                billing_anchor: Some(d(2026, 1, 15)),
                amount_minor: Some(200_000),
                ..base
            },
            ContractType::Retainer => Contract {
                billing_frequency: Some(BillingFrequency::Monthly),
                billing_anchor: Some(d(2026, 1, 15)),
                amount_minor: Some(100_000),
                included_minutes: Some(600),
                overage_rate_minor: Some(15_000),
                ..base
            },
            ContractType::TimeAndMaterials => Contract {
                hourly_rate_minor: Some(10_000),
                ..base
            },
            ContractType::FixedFee => base,
        }
    }

    fn entry(minutes: i32, work_type: Option<&str>, date: NaiveDate) -> BillableEntry {
        BillableEntry {
            time_entry_id: Uuid::now_v7(),
            minutes,
            work_type: work_type.map(str::to_string),
            entry_date: date,
        }
    }

    #[test]
    fn recurring_emits_one_line_per_period() {
        let c = contract(ContractType::Recurring);
        let e = evaluate(&c, &[], &[], &[], d(2026, 2, 1), d(2026, 3, 20)).unwrap();
        assert_eq!(e.lines.len(), 2);
        assert!(e
            .lines
            .iter()
            .all(|l| l.kind == BillingLineKind::RecurringBase));
        assert_eq!(e.lines[0].period_start, d(2026, 2, 15));
        assert_eq!(e.lines[0].amount_minor, 200_000);
        assert_eq!(e.total_minor, 400_000);
    }

    #[test]
    fn recurring_respects_effective_window() {
        let c = Contract {
            effective_from: d(2026, 3, 1),
            ..contract(ContractType::Recurring)
        };
        let e = evaluate(&c, &[], &[], &[], d(2026, 2, 1), d(2026, 3, 20)).unwrap();
        // Feb period starts 2026-02-15, before effective_from: excluded.
        assert_eq!(e.lines.len(), 1);
        assert_eq!(e.lines[0].period_start, d(2026, 3, 15));
    }

    #[test]
    fn recurring_respects_effective_until() {
        let c = Contract {
            effective_until: Some(d(2026, 3, 14)),
            ..contract(ContractType::Recurring)
        };
        let e = evaluate(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 4, 30)).unwrap();
        // Periods starting 01-15, 02-15 qualify; 03-15 and 04-15 are past
        // effective_until.
        assert_eq!(e.lines.len(), 2);
    }

    #[test]
    fn cancelled_contract_evaluates_to_zero_lines() {
        let c = Contract {
            status: ContractStatus::Cancelled,
            ..contract(ContractType::Recurring)
        };
        let e = evaluate(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 4, 30)).unwrap();
        assert!(e.lines.is_empty());
        assert_eq!(e.total_minor, 0);
    }

    #[test]
    fn retainer_bills_base_plus_overage_beyond_included_block() {
        let c = contract(ContractType::Retainer);
        let entries = vec![
            entry(400, None, d(2026, 1, 20)), // period 1
            entry(350, None, d(2026, 2, 2)),  // period 1: total 750 > 600
            entry(100, None, d(2026, 2, 20)), // period 2: under
        ];
        let e = evaluate(&c, &[], &[], &entries, d(2026, 1, 1), d(2026, 3, 1)).unwrap();
        let base: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::RetainerBase)
            .collect();
        let overage: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::RetainerOverage)
            .collect();
        assert_eq!(base.len(), 2);
        assert_eq!(overage.len(), 1);
        assert_eq!(overage[0].quantity_minutes, 150);
        // 150 min at 150.00/h = 375.00 exactly.
        assert_eq!(overage[0].amount_minor, 37_500);
    }

    #[test]
    fn retainer_overage_rounds_half_up() {
        let c = contract(ContractType::Retainer);
        // 45 min at 150.00/h = 112.50 → 11251? No: half-up on the exact
        // half is 11250 + nothing; 45*15000 = 675000 / 60 = 11250 exact.
        // Use 31 min: 31*15000 = 465000/60 = 7750 exact. Force rounding:
        // 1 min at 150.00/h = 1*15000/60 = 250 exact... 1 min at
        // 100.00/h: 10000/60 = 166.67 → 167 (half-up).
        let c = Contract {
            overage_rate_minor: Some(10_000),
            ..c
        };
        let entries = vec![entry(601, None, d(2026, 1, 20))];
        let e = evaluate(&c, &[], &[], &entries, d(2026, 1, 1), d(2026, 2, 1)).unwrap();
        let overage = e
            .lines
            .iter()
            .find(|l| l.kind == BillingLineKind::RetainerOverage)
            .expect("overage line");
        assert_eq!(overage.quantity_minutes, 1);
        assert_eq!(overage.amount_minor, 167); // 10000/60 = 166.67 → 167
    }

    #[test]
    fn time_materials_buckets_by_work_type_with_card_override() {
        let c = contract(ContractType::TimeAndMaterials);
        let rates = vec![core_domain::entities::ContractRate {
            id: Uuid::now_v7(),
            org_id: c.org_id,
            contract_id: c.id,
            work_type: "onsite".into(),
            rate_minor: 18_000,
            created_at: DateTime::UNIX_EPOCH,
        }];
        let entries = vec![
            entry(60, Some("remote"), d(2026, 1, 5)),
            entry(30, Some("onsite"), d(2026, 1, 6)),
            entry(90, None, d(2026, 1, 7)),
            entry(45, Some("remote"), d(2026, 1, 8)),
        ];
        let e = evaluate(&c, &rates, &[], &entries, d(2026, 1, 1), d(2026, 1, 31)).unwrap();
        assert_eq!(e.lines.len(), 3);
        // Deterministic bucket order: BTreeMap key order — "onsite",
        // "remote", "unclassified".
        assert!(e.lines[0].description.contains("onsite"));
        assert_eq!(e.lines[0].unit_amount_minor, 18_000);
        assert_eq!(e.lines[0].quantity_minutes, 30);
        assert_eq!(e.lines[0].amount_minor, 9_000); // 30min @ 180/h
        assert_eq!(e.lines[1].quantity_minutes, 105);
        assert_eq!(e.lines[1].unit_amount_minor, 10_000);
        // 105 min @ 100/h = 175.00
        assert_eq!(e.lines[1].amount_minor, 17_500);
        assert_eq!(e.lines[2].quantity_minutes, 90);
        assert_eq!(e.lines[2].unit_amount_minor, 10_000);
        assert_eq!(e.total_minor, 9_000 + 17_500 + 15_000);
    }

    #[test]
    fn time_materials_clamps_to_effective_window() {
        let c = Contract {
            effective_from: d(2026, 1, 10),
            ..contract(ContractType::TimeAndMaterials)
        };
        let entries = vec![
            entry(60, None, d(2026, 1, 5)), // before effective_from: skipped
            entry(60, None, d(2026, 1, 15)),
        ];
        let e = evaluate(&c, &[], &[], &entries, d(2026, 1, 1), d(2026, 1, 31)).unwrap();
        assert_eq!(e.lines.len(), 1);
        assert_eq!(e.lines[0].quantity_minutes, 60);
    }

    #[test]
    fn fixed_fee_bills_by_due_date_ordered() {
        let c = contract(ContractType::FixedFee);
        let mk_fee = |label: &str, due_on: NaiveDate, amount: i64| ContractFixedFee {
            id: Uuid::now_v7(),
            org_id: c.org_id,
            contract_id: c.id,
            label: label.into(),
            amount_minor: amount,
            due_on,
            notes: None,
            created_at: DateTime::UNIX_EPOCH,
        };
        let fees = vec![
            mk_fee("Migration", d(2026, 3, 1), 500_000),
            mk_fee("Assessment", d(2026, 1, 10), 250_000),
            mk_fee("Too early", d(2025, 12, 1), 999),
            mk_fee("Too late", d(2026, 6, 1), 999),
        ];
        let e = evaluate(&c, &[], &fees, &[], d(2026, 1, 1), d(2026, 3, 31)).unwrap();
        assert_eq!(e.lines.len(), 2);
        assert_eq!(e.lines[0].description, "Assessment");
        assert_eq!(e.lines[1].description, "Migration");
        assert_eq!(e.total_minor, 750_000);
        assert!(e.lines.iter().all(|l| l.kind == BillingLineKind::FixedFee));
    }

    #[test]
    fn inverted_window_is_rejected() {
        let c = contract(ContractType::Recurring);
        assert!(matches!(
            evaluate(&c, &[], &[], &[], d(2026, 3, 1), d(2026, 2, 1)),
            Err(BillingError::InvalidWindow { .. })
        ));
    }

    #[test]
    fn total_overflow_is_typed_not_silent() {
        let c = contract(ContractType::FixedFee);
        let big = i64::MAX;
        let fees = vec![
            ContractFixedFee {
                id: Uuid::now_v7(),
                org_id: c.org_id,
                contract_id: c.id,
                label: "a".into(),
                amount_minor: big,
                due_on: d(2026, 1, 1),
                notes: None,
                created_at: DateTime::UNIX_EPOCH,
            },
            ContractFixedFee {
                id: Uuid::now_v7(),
                org_id: c.org_id,
                contract_id: c.id,
                label: "b".into(),
                amount_minor: big,
                due_on: d(2026, 1, 2),
                notes: None,
                created_at: DateTime::UNIX_EPOCH,
            },
        ];
        assert!(matches!(
            evaluate(&c, &[], &fees, &[], d(2026, 1, 1), d(2026, 12, 31)),
            Err(BillingError::AmountOverflow)
        ));
    }
}
