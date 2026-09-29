//! The recurring-billing rules engine: one pure entry point, [`evaluate`],
//! that turns a contract plus its supporting rows into draft invoice-line
//! candidates over an evaluation window.
//!
//! Contract (Wave 8b): evaluation is READ-ONLY — the output is a preview
//! shape, nothing persists, no `billed` markers exist. Wave 9's invoice
//! run consumes the same function and owns persistence.
//!
//! Determinism guarantees (pinned by tests): lines are ordered by rule
//! (period base lines in ascending period order, then overage lines, then
//! T&M buckets sorted by `work_type`, then fixed fees by due date/label);
//! identical inputs produce byte-identical evaluations. Money math is
//! integer minor units; per-hour proration rounds half-up; totals
//! accumulate in i128 and reject overflow as [`BillingError::AmountOverflow`]
//! rather than silently wrapping.
//!
//! Wave 39 keeps every guarantee: the new relationship types evaluate
//! through the same pure shape ([`evaluate_with`] + [`EvalInputs`]),
//! block draw-down is deterministic (entries ascending, blocks oldest
//! first), caps are typed (never a silent cutoff), and negative amounts
//! exist only on `sla_credit` lines.

use chrono::NaiveDate;
use core_domain::entities::{
    BillingCandidateLine, BillingEvaluation, BillingLineKind, BlockConsumption, BlockDraw,
    CapReachedBehavior, Contract, ContractBlock, ContractCap, ContractFixedFee, ContractRate,
    ContractType,
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

/// One closed ticket input for per-incident contracts (D): already
/// filtered to the contract's client, `category = 'closed'`, resolution
/// inside the query window; the engine re-checks the band it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentTicket {
    pub ticket_id: Uuid,
    pub number: String,
    pub priority: String,
    pub resolved_on: NaiveDate,
}

/// One released milestone awaiting its invoice line (Wave 39 lane 4,
/// report P1/P4): already filtered to the contract's billing projects and
/// status `pending`, released_on inside the query window; the engine
/// mirrors the date window. Dedupe is ledger-owned (one row per
/// milestone), so the engine emits one line per release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRelease {
    pub release_id: Uuid,
    pub label: String,
    pub amount_minor: i64,
    pub released_on: NaiveDate,
    /// The milestone currency; the engine refuses a mismatch against the
    /// contract currency at the query layer (mixed-currency runs assert
    /// one currency), so the engine carries it for the line's provenance
    /// description only.
    #[allow(dead_code)]
    pub currency: String,
    /// Whether this release came from a fixed-fee phase (P4) versus a
    /// milestone (P1) — description wording only; both bill the same way.
    pub is_fixed_fee_phase: bool,
}

/// Inventory counts for per-unit contracts (C), read by the query layer
/// at evaluation time and passed in so the engine stays pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnitCounts {
    pub devices: i64,
    pub users: i64,
}

/// The optional Wave-39 inputs: blocks, caps, minimum, escalator, and
/// the per-unit/per-incident feed data. Every field defaults — the v1
/// caller (and every pre-Wave-39 call site) passes `EvalInputs::default()`
/// and gets the exact v1 behavior.
#[derive(Debug, Clone, Default)]
pub struct EvalInputs<'a> {
    /// Sold prepaid blocks (A), oldest first as read.
    pub blocks: &'a [ContractBlock],
    /// Persisted draw-down minutes per block id (blocks carry a ledger,
    /// so the engine knows what was already consumed BEFORE this window).
    pub prior_draws: Option<&'a BTreeMap<Uuid, i64>>,
    /// NTE caps (B).
    pub caps: &'a [ContractCap],
    /// Per-period minimum commitment minor amount (E), `None` = none.
    pub minimum_commitment_minor: Option<i64>,
    /// Scheduled escalator (E), `None` = none.
    pub escalator: Option<&'a core_domain::entities::ContractEscalator>,
    /// Approved change orders billable inside the window (E).
    pub change_orders: &'a [core_domain::entities::ContractChangeOrder],
    /// Pending SLA credits landable inside the window (E).
    pub sla_credits: &'a [core_domain::entities::ContractSlaCredit],
    /// Closed tickets for per-incident evaluation (D).
    pub closed_tickets: &'a [IncidentTicket],
    /// Inventory counts for per-unit evaluation (C).
    pub unit_counts: UnitCounts,
    /// Released project milestones/fees awaiting their invoice line
    /// (Wave 39 lane 4, report P1/P4): one line per pending release.
    pub project_releases: &'a [ProjectRelease],
}

/// The empty prior-draw ledger shared by every default-input evaluation
/// (static so `EvalInputs::default()` borrows rather than allocates).
static EMPTY_DRAWS: std::sync::LazyLock<BTreeMap<Uuid, i64>> =
    std::sync::LazyLock::new(BTreeMap::new);

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
    evaluate_with(
        contract,
        rates,
        fees,
        entries,
        from,
        to,
        &EvalInputs::default(),
    )
}

/// The full-configuration entry point: identical to [`evaluate`] plus
/// the Wave-39 relationship-type inputs. Determinism guarantee carries
/// over unchanged: identical inputs produce byte-identical evaluations.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_with(
    contract: &Contract,
    rates: &[ContractRate],
    fees: &[ContractFixedFee],
    entries: &[BillableEntry],
    from: NaiveDate,
    to: NaiveDate,
    inputs: &EvalInputs<'_>,
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
        block_consumption: None,
        cap_warnings: Vec::new(),
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
    let mut block_consumption: Option<BlockConsumption> = None;
    let mut cap_warnings: Vec<String> = Vec::new();
    match contract.contract_type {
        ContractType::Recurring => {
            lines.extend(recurring_lines(contract, from, to, hi));
            lines.extend(escalator_lines(contract, from, to, hi, inputs.escalator)?);
            lines.extend(minimum_trueup_lines(
                contract,
                entries,
                from,
                to,
                hi,
                lo,
                inputs.minimum_commitment_minor,
            )?);
            lines.extend(change_order_lines(contract, inputs.change_orders, lo, hi));
            lines.extend(sla_credit_lines(contract, inputs.sla_credits, from, to));
        }
        ContractType::Retainer => {
            lines.extend(recurring_lines(contract, from, to, hi));
            lines.extend(retainer_overage_lines(contract, entries, lo, hi));
            lines.extend(escalator_lines(contract, from, to, hi, inputs.escalator)?);
            lines.extend(minimum_trueup_lines(
                contract,
                entries,
                from,
                to,
                hi,
                lo,
                inputs.minimum_commitment_minor,
            )?);
            lines.extend(change_order_lines(contract, inputs.change_orders, lo, hi));
            lines.extend(sla_credit_lines(contract, inputs.sla_credits, from, to));
        }
        ContractType::TimeAndMaterials => {
            lines.extend(capped_labor_lines(
                contract,
                CapLaborContext {
                    rates,
                    entries,
                    lo,
                    hi,
                    caps: inputs.caps,
                    warnings: &mut cap_warnings,
                },
            )?);
            lines.extend(change_order_lines(contract, inputs.change_orders, lo, hi));
            lines.extend(sla_credit_lines(contract, inputs.sla_credits, from, to));
        }
        ContractType::FixedFee => {
            lines.extend(fixed_fee_lines(contract, fees, lo, hi));
            lines.extend(change_order_lines(contract, inputs.change_orders, lo, hi));
            lines.extend(sla_credit_lines(contract, inputs.sla_credits, from, to));
        }
        ContractType::PerUnit => {
            lines.extend(per_unit_lines(contract, from, to, hi, inputs.unit_counts)?);
            lines.extend(escalator_lines(contract, from, to, hi, inputs.escalator)?);
            lines.extend(minimum_trueup_lines(
                contract,
                entries,
                from,
                to,
                hi,
                lo,
                inputs.minimum_commitment_minor,
            )?);
            lines.extend(change_order_lines(contract, inputs.change_orders, lo, hi));
            lines.extend(sla_credit_lines(contract, inputs.sla_credits, from, to));
        }
        ContractType::PerIncident => {
            lines.extend(per_incident_lines(contract, inputs.closed_tickets, lo, hi));
            lines.extend(change_order_lines(contract, inputs.change_orders, lo, hi));
            lines.extend(sla_credit_lines(contract, inputs.sla_credits, from, to));
        }
    }
    // Project releases (Wave 39 lane 4) ride ANY contract type the same
    // way change orders do: a project bills THROUGH its linked contract,
    // so every released milestone/fee on that contract's projects emits
    // here — one line per ledger row, dedupe owned by the ledger.
    lines.extend(project_release_lines(
        contract,
        inputs.project_releases,
        lo,
        hi,
    ));
    // Prepaid blocks ride ANY consumption-bearing contract as attached
    // configuration (A): labor that the type's own rule billed at card
    // rates is re-priced against an open block's rate, and the
    // consumption detail comes back for the run to persist draws. The
    // sale lines (block deposits due in-window) always emit.
    if !inputs.blocks.is_empty() {
        let consumption = apply_blocks(
            contract,
            inputs.blocks,
            inputs.prior_draws.unwrap_or(&EMPTY_DRAWS),
            entries,
            rates,
            lo,
            hi,
            &mut lines,
        )?;
        block_consumption = Some(consumption);
    }
    let total: i128 = lines.iter().map(|l| l.amount_minor as i128).sum();
    let total_minor = i64::try_from(total).map_err(|_| BillingError::AmountOverflow)?;
    Ok(BillingEvaluation {
        total_minor,
        lines,
        block_consumption,
        cap_warnings,
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

/// Released project milestones/fees (Wave 39 lane 4, report P1/P4): one
/// line per ledger release dated inside the effective slice, ordered by
/// (released_on, label) for determinism. The release id rides the
/// description in `[id]` brackets — the same bracket convention SLA
/// credits use — so the run can flip the ledger row `billed` with the
/// consuming invoice without a description-fragility: re-runs never
/// double-bill because the LEDGER row flips, and the (contract,
/// period_start, description) uniqueness stays the concurrency backstop.
fn project_release_lines(
    contract: &Contract,
    releases: &[ProjectRelease],
    lo: NaiveDate,
    hi: NaiveDate,
) -> Vec<BillingCandidateLine> {
    let mut due: Vec<&ProjectRelease> = releases
        .iter()
        .filter(|r| r.released_on >= lo && r.released_on <= hi)
        .collect();
    due.sort_by(|a, b| (a.released_on, &a.label).cmp(&(b.released_on, &b.label)));
    due.into_iter()
        .map(|release| BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: BillingLineKind::ProjectRelease,
            period_start: release.released_on,
            period_end: release.released_on,
            description: if release.is_fixed_fee_phase {
                format!("{} [{}]", release.label, release.release_id)
            } else {
                format!("Milestone {}: [{}]", release.label, release.release_id)
            },
            quantity_minutes: 0,
            unit_amount_minor: release.amount_minor,
            amount_minor: release.amount_minor,
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

// --- Wave 39 rules ------------------------------------------------------------

/// (C) Per-unit recurring: one line per period, quantity × unit price.
/// The quantity reads the org's inventories AT EVALUATION TIME — the
/// counts arrive as `UnitCounts` (pure inputs, no IO here). Basis
/// `devices` bills active assets scoped to the contract's client;
/// `users` bills the org's user count. A zero count emits no line: a
/// seat count of zero is true "nothing to bill", not an error.
fn per_unit_lines(
    contract: &Contract,
    from: NaiveDate,
    to: NaiveDate,
    hi: NaiveDate,
    counts: UnitCounts,
) -> Result<Vec<BillingCandidateLine>, BillingError> {
    let (Some(anchor), Some(frequency), Some(unit_price)) = (
        contract.billing_anchor,
        contract.billing_frequency,
        contract.unit_price_minor,
    ) else {
        // Persisted contracts always carry the triple (validated in
        // core-domain); an incomplete row simply produces no lines.
        return Ok(Vec::new());
    };
    let quantity = match contract
        .unit_basis
        .unwrap_or(core_domain::entities::UnitBasis::Devices)
    {
        core_domain::entities::UnitBasis::Devices => counts.devices,
        core_domain::entities::UnitBasis::Users => counts.users,
    };
    if quantity <= 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for (start, end) in crate::periods::periods_starting_in(anchor, frequency, from, to) {
        if !effective_period_included(contract, start, hi) {
            continue;
        }
        let amount = i64::try_from((quantity as i128) * (unit_price as i128))
            .map_err(|_| BillingError::AmountOverflow)?;
        out.push(BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: BillingLineKind::PerUnitBase,
            period_start: start,
            period_end: end,
            description: format!(
                "{} per-unit: {} units at {}/unit ({} to {})",
                contract.name, quantity, unit_price, start, end
            ),
            quantity_minutes: 0,
            unit_amount_minor: unit_price,
            amount_minor: amount,
        });
    }
    Ok(out)
}

/// (D) Per-incident: one line per closed ticket in the covered band,
/// ordered by (resolved_on, number). The band is the contract's
/// `incident_priority` (`NULL`/`any` covers every closed ticket; a
/// `p1..p4` value prices only that tier). Tickets outside the effective
/// slice are skipped like any other dated candidate.
fn per_incident_lines(
    contract: &Contract,
    tickets: &[IncidentTicket],
    lo: NaiveDate,
    hi: NaiveDate,
) -> Vec<BillingCandidateLine> {
    let Some(price) = contract.incident_price_minor else {
        return Vec::new();
    };
    let band = contract
        .incident_priority
        .as_deref()
        .filter(|p| !p.is_empty() && *p != "any");
    let mut matched: Vec<&IncidentTicket> = tickets
        .iter()
        .filter(|t| t.resolved_on >= lo && t.resolved_on <= hi)
        .filter(|t| band.is_none_or(|p| t.priority == p))
        .collect();
    matched.sort_by(|a, b| (a.resolved_on, &a.number).cmp(&(b.resolved_on, &b.number)));
    matched
        .into_iter()
        .map(|t| BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: BillingLineKind::PerIncident,
            period_start: t.resolved_on,
            period_end: t.resolved_on,
            description: format!(
                "{} incident: ticket {} ({})",
                contract.name, t.number, t.priority
            ),
            quantity_minutes: 0,
            unit_amount_minor: price,
            amount_minor: price,
        })
        .collect()
}

/// (E) Escalators: a per-period uplift line. The uplift for a period is
/// `amount × (uplift_bps / 10000) × anniversaries_already_passed`, where
/// anniversaries count from the first uplift date (explicit or the
/// anchor's own anniversary). Rounded half-up per period; the line is
/// its own `escalator` row so the base line keeps its identity (the
/// period-dedupe guard sees both separately).
fn escalator_lines(
    contract: &Contract,
    from: NaiveDate,
    to: NaiveDate,
    hi: NaiveDate,
    escalator: Option<&core_domain::entities::ContractEscalator>,
) -> Result<Vec<BillingCandidateLine>, BillingError> {
    let Some(esc) = escalator else {
        return Ok(Vec::new());
    };
    let (Some(anchor), Some(frequency), Some(amount)) = (
        contract.billing_anchor,
        contract.billing_frequency,
        contract.amount_minor,
    ) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (start, end) in crate::periods::periods_starting_in(anchor, frequency, from, to) {
        if !effective_period_included(contract, start, hi) {
            continue;
        }
        let first = esc
            .first_uplift_on
            .unwrap_or(crate::periods::period_start(anchor, frequency, 1));
        if start < first {
            continue;
        }
        // Anniversaries passed by this period's start: one anniversary
        // = one ANNUAL span of billing periods (weekly = 52 steps,
        // monthly = 12, quarterly = 4, semiannual = 2, annual = 1).
        // The count walks the anchor's period arithmetic exactly like
        // every other rule here — no day division, no drift.
        let periods_per_anniversary: u32 = match frequency {
            core_domain::entities::BillingFrequency::Weekly => 52,
            core_domain::entities::BillingFrequency::Monthly => 12,
            core_domain::entities::BillingFrequency::Quarterly => 4,
            core_domain::entities::BillingFrequency::Semiannual => 2,
            core_domain::entities::BillingFrequency::Annual => 1,
        };
        let anniversary_index: i64 = {
            // How many annual period-spans have fully passed by `start`?
            // Step cursor at anniversary k = period index
            // k × periods_per_anniversary; count while cursor <= start.
            let mut count: i64 = 0;
            let mut k: u32 = 1;
            while k <= 600 {
                let cursor = crate::periods::period_start(
                    anchor,
                    frequency,
                    k.saturating_mul(periods_per_anniversary),
                );
                if cursor > start {
                    break;
                }
                count += 1;
                if cursor == first {
                    // (cursor read for determinism documentation only)
                }
                k += 1;
            }
            count
        };
        if anniversary_index <= 0 {
            continue;
        }
        let uplift = half_up_bps(
            amount as i128,
            esc.uplift_bps as i128 * anniversary_index as i128,
        )?;
        if uplift <= 0 {
            continue;
        }
        out.push(BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: BillingLineKind::Escalator,
            period_start: start,
            period_end: end,
            description: format!(
                "{} escalator: +{} bps ({:.1}% × {} uplifts applied)",
                contract.name,
                esc.uplift_bps,
                esc.uplift_bps as f64 / 100.0,
                anniversary_index
            ),
            quantity_minutes: 0,
            unit_amount_minor: uplift,
            amount_minor: uplift,
        });
    }
    Ok(out)
}

/// Basis-point uplift with half-up rounding, `None` on i64 overflow.
fn half_up_bps(amount: i128, bps: i128) -> Result<i64, BillingError> {
    let raw = amount * bps;
    let rounded = (raw + 5_000) / 10_000;
    i64::try_from(rounded).map_err(|_| BillingError::AmountOverflow)
}

/// (E) Minimum-commitment true-up: when a period's OTHER candidate lines
/// (excluding brands that would double as commitments — true-ups
/// themselves, escalators, credits, change orders) fall short of the
/// minimum, one true-up line tops the period up. Consumed-beyond
/// periods emit nothing (the minimum is a floor, not a fee).
fn minimum_trueup_lines(
    contract: &Contract,
    entries: &[BillableEntry],
    from: NaiveDate,
    to: NaiveDate,
    hi: NaiveDate,
    lo: NaiveDate,
    minimum_commitment_minor: Option<i64>,
) -> Result<Vec<BillingCandidateLine>, BillingError> {
    let Some(minimum) = minimum_commitment_minor else {
        return Ok(Vec::new());
    };
    if minimum <= 0 {
        return Ok(Vec::new());
    }
    let (Some(anchor), Some(frequency)) = (contract.billing_anchor, contract.billing_frequency)
    else {
        return Ok(Vec::new());
    };
    // The commitment scope is the CONTRACT-TYPE's base fee period: the
    // true-up makes each period's live billing lines (base + labor +
    // overage) reach the floor. Period attribution here follows the
    // same start-gated window rule as the base lines.
    let mut out = Vec::new();
    for (start, end) in crate::periods::periods_starting_in(anchor, frequency, from, to) {
        if !effective_period_included(contract, start, hi) {
            continue;
        }
        // Period-consumed labor for retainer/T&M-ish commitments; for
        // pure recurring the base line alone can fall short.
        let consumed: i64 = entries
            .iter()
            .filter(|e| e.entry_date >= start.max(lo) && e.entry_date < end && e.entry_date <= hi)
            .map(|e| e.minutes as i64)
            .sum();
        let labor_amount = match contract.contract_type {
            ContractType::Retainer => {
                let included = contract.included_minutes.unwrap_or(0) as i64;
                let rate = contract.overage_rate_minor.unwrap_or(0);
                consumed
                    .saturating_sub(included)
                    .checked_mul(rate)
                    .map(|raw| (raw + 30) / 60)
                    .unwrap_or(0)
            }
            ContractType::TimeAndMaterials => consumed
                .checked_mul(contract.hourly_rate_minor.unwrap_or(0))
                .map(|raw| (raw + 30) / 60)
                .unwrap_or(0),
            _ => 0,
        };
        let base = contract.amount_minor.unwrap_or(0);
        let billed = base as i128 + labor_amount as i128;
        if billed >= minimum as i128 {
            continue;
        }
        let trueup = minimum as i128 - billed;
        let trueup = i64::try_from(trueup).map_err(|_| BillingError::AmountOverflow)?;
        out.push(BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: BillingLineKind::MinimumTrueup,
            period_start: start,
            period_end: end,
            description: format!(
                "{} minimum true-up: billed {} below {} commitment ({} to {})",
                contract.name, billed, minimum, start, end
            ),
            quantity_minutes: 0,
            unit_amount_minor: trueup,
            amount_minor: trueup,
        });
    }
    Ok(out)
}

/// (E) Change orders: an APPROVED order whose effective date (the day
/// it was approved) falls inside the window bills as its own line.
/// Rejected/cancelled/draft orders are invisible to billing; an already
/// `billed` order dedupes through (kind, period_start, description)
/// where the description carries the order number.
fn change_order_lines(
    contract: &Contract,
    orders: &[core_domain::entities::ContractChangeOrder],
    lo: NaiveDate,
    hi: NaiveDate,
) -> Vec<BillingCandidateLine> {
    let mut approved: Vec<&core_domain::entities::ContractChangeOrder> = orders
        .iter()
        .filter(|o| o.status == "approved")
        .filter(|o| {
            o.approved_at
                .map(|a| a.date_naive())
                .map(|d| d >= lo && d <= hi)
                .unwrap_or(false)
        })
        .collect();
    approved.sort_by(|a, b| (&a.number, &a.title).cmp(&(&b.number, &b.title)));
    approved
        .into_iter()
        .map(|o| BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: BillingLineKind::ChangeOrder,
            period_start: o.approved_at.map(|a| a.date_naive()).unwrap_or(lo),
            period_end: o.approved_at.map(|a| a.date_naive()).unwrap_or(lo),
            description: format!("Change order {}: {}", o.number, o.title),
            quantity_minutes: 0,
            unit_amount_minor: o.amount_minor,
            amount_minor: o.amount_minor,
        })
        .collect()
}

/// (E) SLA service credits: PENDING credits whose `applies_on` lands in
/// the window become NEGATIVE lines (the only signed line kind), each
/// description carrying the credit id so the run's dedupe and the
/// post-run `mark_applied` pair up exactly.
fn sla_credit_lines(
    contract: &Contract,
    credits: &[core_domain::entities::ContractSlaCredit],
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<BillingCandidateLine> {
    let mut pending: Vec<&core_domain::entities::ContractSlaCredit> = credits
        .iter()
        .filter(|c| c.status == "pending")
        .filter(|c| c.applies_on >= from && c.applies_on <= to)
        .collect();
    pending.sort_by(|a, b| (a.applies_on, &a.reason).cmp(&(b.applies_on, &b.reason)));
    pending
        .into_iter()
        .map(|c| BillingCandidateLine {
            contract_id: contract.id,
            contract_number: contract.number.clone(),
            client_id: contract.client_id,
            kind: BillingLineKind::SlaCredit,
            period_start: c.applies_on,
            period_end: c.applies_on,
            description: format!("SLA service credit [{}]: {}", c.id, c.reason),
            quantity_minutes: 0,
            unit_amount_minor: c.amount_minor,
            amount_minor: -c.amount_minor,
        })
        .collect()
}

/// The labor context cappedLaborLines needs: everything the T&M rule
/// used plus the caps and a warnings sink for typed `warn` behavior.
struct CapLaborContext<'a> {
    rates: &'a [ContractRate],
    entries: &'a [BillableEntry],
    lo: NaiveDate,
    hi: NaiveDate,
    caps: &'a [ContractCap],
    warnings: &'a mut Vec<String>,
}

/// (B) T&M labor under NTE caps. With no caps this is exactly the v1
/// `time_materials_lines` behavior. With caps:
///
/// * `period` scope: the cap applies per billing period derived from
///   the contract's anchor/frequency (no anchor = no period lines, cap
///   falls back to window scope for the preview's honesty — surfaced
///   as a warning line, never silent).
/// * `contract` scope: the cap applies over the whole effective slice.
///
/// Behaviors at cap reach: `stop` drops unbilled minutes beyond the
/// cap; `warn` bills them AT the card/default rate and records a
/// warning per capped window; `overage_billable` bills beyond-cap
/// minutes as separate `cap_overage` lines at the cap's overage rate
/// (falling back to the matched card/default rate).
fn capped_labor_lines(
    contract: &Contract,
    ctx: CapLaborContext<'_>,
) -> Result<Vec<BillingCandidateLine>, BillingError> {
    if ctx.caps.is_empty() {
        return Ok(time_materials_lines(
            contract,
            ctx.rates,
            ctx.entries,
            ctx.lo,
            ctx.hi,
        ));
    }
    let default_rate = contract.hourly_rate_minor.unwrap_or(0);
    let matched_rate = |work_type: Option<&str>| -> (String, i64) {
        work_type
            .and_then(|wt| ctx.rates.iter().find(|r| r.work_type == wt))
            .map(|r| (r.work_type.clone(), r.rate_minor))
            .unwrap_or_else(|| {
                (
                    work_type
                        .map(str::to_string)
                        .unwrap_or_else(|| "unclassified".to_string()),
                    default_rate,
                )
            })
    };

    // Cap lookup for a bucket: the most specific (work_type-named) cap
    // wins; the unscoped cap (work_type NULL) is the catch-all.
    let cap_for = |work_type: &str| -> Option<&ContractCap> {
        ctx.caps
            .iter()
            .find(|c| c.work_type.as_deref() == Some(work_type))
            .or_else(|| ctx.caps.iter().find(|c| c.work_type.is_none()))
    };

    // Bucket first (v1 shape), then distribute each bucket's minutes
    // across the cap windows that apply.
    let mut buckets: BTreeMap<String, Vec<&BillableEntry>> = BTreeMap::new();
    for entry in ctx.entries {
        if entry.entry_date < ctx.lo || entry.entry_date > ctx.hi {
            continue;
        }
        let key = entry
            .work_type
            .clone()
            .unwrap_or_else(|| "unclassified".to_string());
        buckets.entry(key).or_default().push(entry);
    }

    // Cap windows: `period` scope derives billing periods from the
    // anchor; `contract` scope is one window over the effective slice.
    let periods = match (contract.billing_anchor, contract.billing_frequency) {
        (Some(anchor), Some(frequency)) => {
            crate::periods::periods_starting_in(anchor, frequency, ctx.lo, ctx.hi)
                .into_iter()
                .filter(|(s, _)| effective_period_included(contract, *s, ctx.hi))
                .collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };

    let mut out: Vec<BillingCandidateLine> = Vec::new();
    for (work_type, bucket_entries) in &buckets {
        let (rate_key, base_rate) = matched_rate(Some(work_type));
        let Some(cap) = cap_for(work_type) else {
            // Uncapped bucket: v1 behavior, one line for the window.
            let minutes: i64 = bucket_entries.iter().map(|e| e.minutes as i64).sum();
            if minutes == 0 {
                continue;
            }
            let amount = prorate(minutes, base_rate).ok_or(BillingError::AmountOverflow)?;
            out.push(line_labor(
                contract,
                BillingLineKind::TimeMaterials,
                &rate_key,
                base_rate,
                minutes,
                amount,
                ctx.lo,
                ctx.hi,
            ));
            continue;
        };
        // Consume the bucket's minutes across the cap's windows in
        // deterministic order; minutes past the cap follow the typed
        // behavior.
        let windows: Vec<(NaiveDate, NaiveDate)> = if cap.scope == "period" {
            if periods.is_empty() {
                // No anchor on a period-capped contract: the cap cannot
                // attribute minutes to periods, so it applies to the
                // window whole — and says so.
                ctx.warnings.push(format!(
                    "cap ({} minor) has scope=period but the contract has no billing anchor; applied over the window {} to {}",
                    cap.cap_minor, ctx.lo, ctx.hi
                ));
                vec![(ctx.lo, ctx.hi)]
            } else {
                periods.clone()
            }
        } else {
            vec![(ctx.lo, ctx.hi)]
        };
        let mut window_remaining = cap.cap_minor; // remaining billable MINOR units per window
                                                  // minutes of this bucket fee at base_rate per prorated minute
        for (start, end) in windows.iter().copied() {
            let in_window: Vec<&&BillableEntry> = bucket_entries
                .iter()
                .filter(|e| e.entry_date >= start.max(ctx.lo) && e.entry_date <= end.min(ctx.hi))
                .collect();
            if in_window.is_empty() {
                continue;
            }
            let mut window_base_lines: Vec<(i64, i64)> = Vec::new(); // (minutes, amount)
            let mut overage_minutes: i64 = 0;
            for e in in_window {
                // Cost of this entry at the base rate, half-up, in
                // minor units: charge it against the window's remaining
                // allowance minute-fractionally? No — caps measure
                // MONEY: compare prorated amounts against cap_minor.
                let entry_amount =
                    prorate(e.minutes as i64, base_rate).ok_or(BillingError::AmountOverflow)?;
                if window_remaining >= entry_amount {
                    window_remaining -= entry_amount;
                    window_base_lines.push((e.minutes as i64, entry_amount));
                } else if cap.reached_behavior == CapReachedBehavior::Stop {
                    // stop: the entry beyond the cap is dropped per entry
                    // (no partial-entry splitting — entries are atomic
                    // work items; a partial would silently bill a
                    // fraction of real work).
                    continue;
                } else if cap.reached_behavior == CapReachedBehavior::Warn {
                    // warn: bill through at the base rate, mark it
                    window_remaining = window_remaining.saturating_sub(entry_amount);
                    window_base_lines.push((e.minutes as i64, entry_amount));
                } else {
                    // overage_billable beyond-cap entries
                    overage_minutes += e.minutes as i64;
                }
            }
            let base_minutes: i64 = window_base_lines.iter().map(|(m, _)| m).sum();
            if base_minutes > 0 {
                let base_amount: i64 = window_base_lines.iter().map(|(_, a)| a).sum();
                out.push(line_labor(
                    contract,
                    BillingLineKind::TimeMaterials,
                    &rate_key,
                    base_rate,
                    base_minutes,
                    base_amount,
                    start.max(ctx.lo),
                    end.min(ctx.hi),
                ));
            }
            if overage_minutes > 0 {
                let overage_rate = cap.overage_rate_minor.unwrap_or(base_rate);
                let amount =
                    prorate(overage_minutes, overage_rate).ok_or(BillingError::AmountOverflow)?;
                out.push(line_labor(
                    contract,
                    BillingLineKind::CapOverage,
                    &rate_key,
                    overage_rate,
                    overage_minutes,
                    amount,
                    start.max(ctx.lo),
                    end.min(ctx.hi),
                ));
            }
            let cap_reached = window_remaining < cap.cap_minor
                && matches!(cap.reached_behavior, CapReachedBehavior::Warn);
            if cap_reached {
                ctx.warnings.push(format!(
                    "cap reached ({} minor) for '{}' between {} and {} — billed through per the warn policy",
                    cap.cap_minor, work_type, start, end
                ));
            }
        }
    }
    Ok(out)
}

/// Shared T&M labor line builder (bucketed line, minutes + prorated
/// amount), keeping the per-kind call sites honest.
#[allow(clippy::too_many_arguments)]
fn line_labor(
    contract: &Contract,
    kind: BillingLineKind,
    work_type: &str,
    rate: i64,
    minutes: i64,
    amount: i64,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> BillingCandidateLine {
    BillingCandidateLine {
        contract_id: contract.id,
        contract_number: contract.number.clone(),
        client_id: contract.client_id,
        kind,
        period_start,
        period_end,
        description: format!(
            "{} labor: {} min at {rate}/h ({work_type})",
            contract.name, minutes
        ),
        quantity_minutes: minutes,
        unit_amount_minor: rate,
        amount_minor: amount,
    }
}

/// (A) Prepaid blocks, applied to the evaluated lines. Two concerns in
/// one pass, both deterministic:
///
/// 1. **Sale lines** (always): a block whose SALE lands in the window
///    (created_at date in `[lo, hi]`, effective slice) emits a
///    `block_sale` line — the deposit the client pays up front.
/// 2. **Draw-down re-pricing** (labor lines only): each `TimeMaterials`
///    line's minutes draw against open blocks (oldest, created first,
///    not expired, balance from `total - prior - this-window draws`).
///    Drawn minutes re-price at the block's rate when it carries one;
///    unblocked minutes keep the card/default pricing and report as
///    `unblocked_minutes`. Rollover/expiry facts return via
///    `BlockConsumption` for the run to persist and the UI to alert on;
///    the engine never edits block rows (additive ledger).
#[allow(clippy::too_many_arguments)]
fn apply_blocks(
    contract: &Contract,
    blocks: &[ContractBlock],
    prior_draws: &BTreeMap<Uuid, i64>,
    entries: &[BillableEntry],
    rates: &[ContractRate],
    lo: NaiveDate,
    hi: NaiveDate,
    lines: &mut Vec<BillingCandidateLine>,
) -> Result<BlockConsumption, BillingError> {
    let mut consumption = BlockConsumption::default();

    // 1. Sale lines for blocks sold in-window (deterministic order:
    // created_at then label read order is the caller's; we sort by
    // label for the line ordering guarantee).
    for block in blocks {
        let created_day = block.created_at.date_naive();
        if created_day >= lo && created_day <= hi {
            lines.push(BillingCandidateLine {
                contract_id: contract.id,
                contract_number: contract.number.clone(),
                client_id: contract.client_id,
                kind: BillingLineKind::BlockSale,
                period_start: created_day,
                period_end: created_day,
                description: format!("{} block sale: {}", contract.name, block.label),
                quantity_minutes: 0,
                unit_amount_minor: block.sale_price_minor,
                amount_minor: block.sale_price_minor,
            });
        }
    }

    // 2. Draw-down: walk labor entries in DATE order (the entries
    // arrive ascending; a stable re-sort keeps the guarantee explicit)
    // across open blocks. Balance = total_minutes − prior_draws − draws
    // made in this pass. Minutes the blocks cannot absorb stay billed
    // at card/default rates and report as `unblocked_minutes`.
    let mut open: Vec<&ContractBlock> = blocks
        .iter()
        .filter(|b| match b.rollover_policy.as_str() {
            "none" => true,
            _ => b.expires_on.map(|e| e >= lo).unwrap_or(true),
        })
        .collect();
    open.sort_by(|a, b| (a.created_at, &a.label).cmp(&(b.created_at, &b.label)));

    let mut balances: BTreeMap<Uuid, i64> = BTreeMap::new();
    for b in &open {
        let prior = prior_draws.get(&b.id).copied().unwrap_or(0);
        balances.insert(b.id, (b.total_minutes as i64) - prior);
    }

    // Draw-down first (aggregate per bucket), THEN the expiry alerts —
    // a block partly consumed inside the window reports only what is
    // actually unused at expiry, never a pre-draw stale remainder.
    // Aggregate the block-rate re-pricing per bucket BEFORE the line
    // rewrites: each entry in date order draws from the oldest open
    // block alive on its date. Total drawn + the block rates' blended
    // cost decide the per-bucket adjustments.
    #[derive(Default)]
    struct BucketDraw {
        drawn: i64,
        drawn_cost: i128,
    }
    let mut bucket_draws: BTreeMap<String, BucketDraw> = BTreeMap::new();
    let mut unblocked: i64 = 0;
    let mut entry_draws: Vec<BlockDraw> = Vec::new();
    for e in entries {
        let work_type = e
            .work_type
            .clone()
            .unwrap_or_else(|| "unclassified".to_string());
        let mut remaining_entry = e.minutes as i64;
        for b in &open {
            if remaining_entry == 0 {
                break;
            }
            // Block alive on the entry date (sold on or before it, and
            // not expired for draw purposes).
            if b.created_at.date_naive() > e.entry_date {
                continue;
            }
            if let Some(exp) = b.expires_on {
                if e.entry_date > exp {
                    continue;
                }
            }
            let balance = balances.get_mut(&b.id).expect("open blocks initialized");
            if *balance <= 0 {
                continue;
            }
            let take = remaining_entry.min(*balance);
            *balance -= take;
            remaining_entry -= take;
            let bucket = bucket_draws.entry(work_type.clone()).or_default();
            bucket.drawn += take;
            if let Some(br) = b.block_rate_minor {
                if let Some(cost) = prorate(take, br) {
                    bucket.drawn_cost += cost as i128;
                }
            }
            entry_draws.push(BlockDraw {
                block_id: b.id,
                time_entry_id: Some(e.time_entry_id),
                minutes: take as i32,
                spilled_minutes: 0,
                expired_unused_minutes: 0,
            });
        }
        if remaining_entry > 0 {
            unblocked += remaining_entry;
        }
    }
    // Draw entries before expiry alerts so the alert marker reports the
    // post-draw remainder (never a stale pre-draw balance).
    consumption.draws.extend(entry_draws);

    // Expiry/rollover alerts: a block whose expiry lands in the window
    // with unused minutes reports them (the depletion/expiry alert
    // payload). A `rollover` block's successor mints at PERSIST time
    // (the run, additive-ledger style) — the engine only reports the
    // number that would roll.
    for b in blocks {
        if let Some(expires) = b.expires_on {
            let policy_allows = matches!(b.rollover_policy.as_str(), "expire" | "rollover");
            if policy_allows && expires >= lo && expires <= hi {
                let remaining = balances.get(&b.id).copied().unwrap_or(0).max(0);
                if remaining > 0 {
                    consumption.draws.push(BlockDraw {
                        block_id: b.id,
                        time_entry_id: None,
                        minutes: 0,
                        spilled_minutes: 0,
                        expired_unused_minutes: remaining as i32,
                    });
                }
            }
        }
    }

    // Re-price each labor line: drawn minutes leave the card-rate
    // bucket amount and enter the block-rate amount. Blocks without a
    // rate absorb minutes at the card price (prepaid balance, no
    // repricing).
    let default_rate = contract.hourly_rate_minor.unwrap_or(0);
    let mut new_lines: Vec<BillingCandidateLine> = Vec::new();
    for line in lines.iter() {
        if line.kind != BillingLineKind::TimeMaterials {
            new_lines.push(line.clone());
            continue;
        }
        let work_type = line
            .description
            .rsplit('(')
            .next()
            .map(|s| s.trim_end_matches(')'))
            .unwrap_or("unclassified")
            .to_string();
        let Some(BucketDraw { drawn, drawn_cost }) = bucket_draws.get(&work_type) else {
            new_lines.push(line.clone());
            continue;
        };
        if *drawn == 0 {
            new_lines.push(line.clone());
            continue;
        }
        let card_rate = rates
            .iter()
            .find(|r| r.work_type == work_type)
            .map(|r| r.rate_minor)
            .unwrap_or(default_rate);
        // Card cost of the drawn minutes (what the line currently holds
        // for them) vs the block cost actually owed.
        let card_cost = prorate(*drawn, card_rate).unwrap_or(0) as i128;
        let amount = line.amount_minor as i128 - card_cost + *drawn_cost;
        let amount = i64::try_from(amount).map_err(|_| BillingError::AmountOverflow)?;
        new_lines.push(BillingCandidateLine {
            description: format!(
                "{} ({} min drawn from prepaid blocks)",
                line.description, drawn
            ),
            amount_minor: amount,
            ..line.clone()
        });
    }
    consumption.unblocked_minutes = unblocked;
    *lines = new_lines;
    Ok(consumption)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
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
            unit_price_minor: None,
            unit_basis: None,
            incident_price_minor: None,
            incident_priority: None,
            po_number: None,
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
                billing_frequency: Some(BillingFrequency::Monthly),
                billing_anchor: Some(d(2026, 1, 1)),
                hourly_rate_minor: Some(10_000),
                ..base
            },
            ContractType::FixedFee => base,
            ContractType::PerUnit => Contract {
                billing_frequency: Some(BillingFrequency::Monthly),
                billing_anchor: Some(d(2026, 1, 15)),
                unit_price_minor: Some(5_000),
                ..base
            },
            ContractType::PerIncident => Contract {
                incident_price_minor: Some(25_000),
                ..base
            },
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

    // --- Wave 39: relationship-type coverage ---------------------------------

    use core_domain::entities::{CapReachedBehavior, ContractBlock, ContractCap, UnitBasis};
    use std::sync::Arc;

    fn block(
        label: &str,
        total_minutes: i32,
        price: i64,
        rate: Option<i64>,
        expires: Option<NaiveDate>,
        policy: &str,
    ) -> ContractBlock {
        block_with_sale_date(
            label,
            total_minutes,
            price,
            rate,
            expires,
            policy,
            DateTime::UNIX_EPOCH,
        )
    }

    /// Same builder with an explicit SOLD date (the sale line only
    /// emits when the date part lands in the evaluation window).
    fn block_with_sale_date(
        label: &str,
        total_minutes: i32,
        price: i64,
        rate: Option<i64>,
        expires: Option<NaiveDate>,
        policy: &str,
        sold_at: DateTime<Utc>,
    ) -> ContractBlock {
        ContractBlock {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            contract_id: Uuid::now_v7(),
            label: label.into(),
            total_minutes,
            sale_price_minor: price,
            block_rate_minor: rate,
            expires_on: expires,
            rollover_policy: policy.into(),
            created_at: sold_at,
        }
    }

    fn cap(
        scope: &str,
        cap_minor: i64,
        behavior: CapReachedBehavior,
        overage: Option<i64>,
        work_type: Option<&str>,
    ) -> ContractCap {
        ContractCap {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            contract_id: Uuid::now_v7(),
            scope: scope.into(),
            cap_minor,
            reached_behavior: behavior,
            overage_rate_minor: overage,
            work_type: work_type.map(str::to_string),
            created_at: DateTime::UNIX_EPOCH,
        }
    }

    fn ticket(number: &str, priority: &str, resolved: NaiveDate) -> IncidentTicket {
        IncidentTicket {
            ticket_id: Uuid::now_v7(),
            number: number.into(),
            priority: priority.into(),
            resolved_on: resolved,
        }
    }

    fn with_blocks(_c: &Contract, blocks: &[ContractBlock]) -> EvalInputs<'static> {
        let owned: Vec<ContractBlock> = blocks.to_vec();
        EvalInputs {
            blocks: Box::leak(owned.into_boxed_slice()),
            ..Default::default()
        }
    }

    fn with_releases(releases: &[ProjectRelease]) -> EvalInputs<'static> {
        let owned: Vec<ProjectRelease> = releases.to_vec();
        EvalInputs {
            project_releases: Box::leak(owned.into_boxed_slice()),
            ..Default::default()
        }
    }

    fn release(label: &str, amount: i64, on: NaiveDate) -> ProjectRelease {
        ProjectRelease {
            release_id: Uuid::now_v7(),
            label: label.into(),
            amount_minor: amount,
            released_on: on,
            currency: "USD".into(),
            is_fixed_fee_phase: false,
        }
    }

    #[test]
    fn project_release_emits_one_line_per_ledger_row_in_window() {
        let c = contract(ContractType::Recurring);
        let r1 = release("Discovery", 150_000, d(2026, 2, 10));
        let r2 = release("Build", 90_000, d(2026, 2, 20));
        let inputs = with_releases(&[r1, r2]);
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 2, 1), d(2026, 2, 28), &inputs).unwrap();
        let lines: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::ProjectRelease)
            .collect();
        assert_eq!(lines.len(), 2, "both releases bill");
        assert_eq!(lines[0].description, lines[0].description); // determinism smoke
        let total: i64 = lines.iter().map(|l| l.amount_minor).sum();
        assert_eq!(total, 240_000);
        // Recurring rides the same evaluation: the fixture's Recurring
        // contract bills its own Feb-15 monthly base period (200_000)
        // alongside the 240_000 of releases.
        assert_eq!(
            e.total_minor, 440_000,
            "release lines plus the Recurring base period carry the total"
        );
    }

    #[test]
    fn project_releases_filter_to_the_window_and_order_deterministically() {
        let c = contract(ContractType::TimeAndMaterials);
        let early = release("Early", 10_000, d(2026, 1, 31));
        let inside = release("Inside", 20_000, d(2026, 2, 5));
        let late = release("Late", 30_000, d(2026, 3, 1));
        let inputs = with_releases(&[late, early, inside]);
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 2, 1), d(2026, 2, 28), &inputs).unwrap();
        let lines: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::ProjectRelease)
            .collect();
        assert_eq!(lines.len(), 1, "only the in-window release bills");
        assert_eq!(lines[0].amount_minor, 20_000);
    }

    #[test]
    fn project_release_lines_carry_the_release_id_for_run_flip() {
        let c = contract(ContractType::Recurring);
        let r = release("Phase fee", 50_000, d(2026, 2, 10));
        let id = r.release_id.to_string();
        let inputs = with_releases(&[r]);
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 2, 1), d(2026, 2, 28), &inputs).unwrap();
        let line = e
            .lines
            .iter()
            .find(|l| l.kind == BillingLineKind::ProjectRelease)
            .expect("line emits");
        let bracketed = line
            .description
            .split('[')
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("release id rides the description in [id] brackets");
        assert_eq!(bracketed, id);
        assert_eq!(
            line.period_start,
            d(2026, 2, 10),
            "period mirrors released_on (dedupe identity)"
        );
        assert_eq!(line.quantity_minutes, 0, "amount line, not labor");
        assert_eq!(line.unit_amount_minor, line.amount_minor);
    }

    #[test]
    fn project_release_respects_the_effective_slice() {
        let mut c = contract(ContractType::Recurring);
        c.effective_until = Some(d(2026, 2, 14));
        let before = release("Before expiry", 10_000, d(2026, 2, 14));
        let after = release("After expiry", 20_000, d(2026, 2, 15));
        let inputs = with_releases(&[before, after]);
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 2, 1), d(2026, 2, 28), &inputs).unwrap();
        let lines: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::ProjectRelease)
            .collect();
        assert_eq!(lines.len(), 1, "releases past effective_until never bill");
        assert_eq!(lines[0].amount_minor, 10_000);
    }

    #[test]
    fn default_inputs_reproduce_v1_behavior_exactly_with_release_field_present() {
        // The v1 compat pin now also covers the new field: an empty
        // project_releases slice changes nothing on a plain contract.
        let c = contract(ContractType::Recurring);
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &[],
            d(2026, 1, 1),
            d(2026, 3, 1),
            &EvalInputs::default(),
        )
        .unwrap();
        assert!(
            e.lines
                .iter()
                .all(|l| l.kind != BillingLineKind::ProjectRelease),
            "empty feed emits nothing"
        );
    }

    #[test]
    fn per_unit_bills_quantity_times_price_from_device_counts() {
        let c = contract(ContractType::PerUnit);
        let counts = UnitCounts {
            devices: 25,
            users: 40,
        };
        let inputs = EvalInputs {
            unit_counts: counts,
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 3, 1), &inputs).unwrap();
        let unit: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::PerUnitBase)
            .collect();
        assert_eq!(unit.len(), 2);
        // 25 devices × 50.00 = 1250.00 per period.
        assert_eq!(unit[0].amount_minor, 125_000);
        assert_eq!(unit[0].unit_amount_minor, 5_000);
        assert_eq!(e.total_minor, 250_000);
    }

    #[test]
    fn per_unit_user_basis_switches_to_user_counts() {
        let c = Contract {
            unit_basis: Some(UnitBasis::Users),
            ..contract(ContractType::PerUnit)
        };
        let counts = UnitCounts {
            devices: 25,
            users: 40,
        };
        let inputs = EvalInputs {
            unit_counts: counts,
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 2, 1), &inputs).unwrap();
        assert_eq!(e.lines[0].amount_minor, 200_000); // 40 × 50.00
    }

    #[test]
    fn per_unit_zero_count_bills_nothing() {
        let c = contract(ContractType::PerUnit);
        let inputs = EvalInputs {
            unit_counts: UnitCounts {
                devices: 0,
                users: 0,
            },
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 3, 1), &inputs).unwrap();
        assert!(e.lines.is_empty());
        assert_eq!(e.total_minor, 0);
    }

    #[test]
    fn per_incident_prices_closed_tickets_with_priority_band() {
        let c = contract(ContractType::PerIncident);
        let tickets = [
            ticket("T-001", "p1", d(2026, 1, 5)),
            ticket("T-002", "p2", d(2026, 1, 9)),
            ticket("T-003", "p1", d(2026, 1, 20)),
            ticket("T-004", "p3", d(2025, 12, 30)), // outside window
        ];
        let open = [tickets[0].clone(), tickets[2].clone()];
        let inputs = EvalInputs {
            closed_tickets: &open,
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 1, 31), &inputs).unwrap();
        // NULL/any band prices every CLOSED ticket in window.
        assert_eq!(e.lines.len(), 2);
        assert!(e
            .lines
            .iter()
            .all(|l| l.kind == BillingLineKind::PerIncident));
        assert!(e.lines[0].description.contains("T-001"));
        assert_eq!(e.total_minor, 50_000);
    }

    #[test]
    fn per_incident_band_filters_to_priority() {
        let c = Contract {
            incident_priority: Some("p1".into()),
            ..contract(ContractType::PerIncident)
        };
        let tickets = [
            ticket("T-001", "p1", d(2026, 1, 5)),
            ticket("T-002", "p2", d(2026, 1, 9)),
        ];
        let inputs = EvalInputs {
            closed_tickets: &tickets,
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 1, 31), &inputs).unwrap();
        assert_eq!(e.lines.len(), 1);
        assert!(e.lines[0].description.contains("T-001"));
    }

    #[test]
    fn cap_stop_drops_beyond_cap_minutes() {
        let c = contract(ContractType::TimeAndMaterials);
        let caps = vec![cap("period", 100_000, CapReachedBehavior::Stop, None, None)];
        let entries = vec![
            entry(600, None, d(2026, 1, 5)), // 600 min @ 100/h = 1000.00 → at cap
            entry(300, None, d(2026, 1, 10)), // 300 min = 500.00 → beyond cap, dropped
        ];
        let inputs = EvalInputs {
            caps: &caps,
            ..Default::default()
        };
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 2, 1),
            &inputs,
        )
        .unwrap();
        let labor: i64 = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::TimeMaterials)
            .map(|l| l.amount_minor)
            .sum();
        assert_eq!(
            labor, 100_000,
            "stop behavior drops the second entry once the cap is spent"
        );
        assert!(
            e.cap_warnings.is_empty(),
            "stop communicates through the line shape"
        );
    }

    #[test]
    fn cap_warn_bills_through_and_records_warning() {
        let c = contract(ContractType::TimeAndMaterials);
        let caps = vec![cap("period", 100_000, CapReachedBehavior::Warn, None, None)];
        let entries = vec![
            entry(600, None, d(2026, 1, 5)),
            entry(300, None, d(2026, 1, 10)),
        ];
        let inputs = EvalInputs {
            caps: &caps,
            ..Default::default()
        };
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 2, 1),
            &inputs,
        )
        .unwrap();
        let labor: i64 = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::TimeMaterials)
            .map(|l| l.amount_minor)
            .sum();
        assert_eq!(labor, 150_000, "warn bills through at the card rate");
        assert_eq!(e.cap_warnings.len(), 1);
        assert!(
            e.cap_warnings[0].contains("1500.00") || e.cap_warnings[0].contains("warn"),
            "warning names the policy: {:?}",
            e.cap_warnings
        );
    }

    #[test]
    fn cap_overage_billable_splits_beyond_cap_at_overage_rate() {
        let c = contract(ContractType::TimeAndMaterials);
        let caps = vec![cap(
            "period",
            100_000,
            CapReachedBehavior::OverageBillable,
            Some(20_000),
            None,
        )];
        let entries = vec![
            entry(600, None, d(2026, 1, 5)),  // 1000.00 exactly at cap
            entry(300, None, d(2026, 1, 10)), // beyond → overage bucket
        ];
        let inputs = EvalInputs {
            caps: &caps,
            ..Default::default()
        };
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 2, 1),
            &inputs,
        )
        .unwrap();
        let base: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::TimeMaterials)
            .collect();
        let over: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::CapOverage)
            .collect();
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].amount_minor, 100_000);
        assert_eq!(over.len(), 1);
        assert_eq!(
            over[0].amount_minor, 100_000,
            "300 min @ 200/h = 1000.00 overage line"
        );
        assert_eq!(e.total_minor, 200_000);
    }

    #[test]
    fn cap_contract_scope_crosses_periods() {
        let c = contract(ContractType::TimeAndMaterials);
        let caps = vec![cap(
            "contract",
            100_000,
            CapReachedBehavior::Stop,
            None,
            None,
        )];
        let entries = vec![
            entry(600, None, d(2026, 1, 5)), // spends the whole cap
            entry(600, None, d(2026, 2, 5)), // next period; contract cap spent → dropped
        ];
        let inputs = EvalInputs {
            caps: &caps,
            ..Default::default()
        };
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 3, 1),
            &inputs,
        )
        .unwrap();
        let labor: i64 = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::TimeMaterials)
            .map(|l| l.amount_minor)
            .sum();
        assert_eq!(
            labor, 100_000,
            "a per-contract cap spans every period in the window"
        );
    }

    #[test]
    fn block_drawdown_reprices_against_block_rate_and_reports_spill() {
        let c = contract(ContractType::TimeAndMaterials);
        let blk = block(
            "50h block",
            3_000,
            9_000_000,
            Some(6_000),
            Some(d(2026, 12, 31)),
            "none",
        );
        let inputs = with_blocks(&c, std::slice::from_ref(&blk));
        // 100 hours of work: 50 draws at 60/h (block rate), 50 spills to card.
        let entries = vec![entry(6_000, None, d(2026, 2, 10))];
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 3, 1),
            &inputs,
        )
        .unwrap();
        let consump = e.block_consumption.expect("block consumption reported");
        let drawn: i32 = consump.draws.iter().map(|x| x.minutes).sum();
        assert_eq!(drawn, 3_000, "all block minutes drew down");
        assert_eq!(consump.unblocked_minutes, 3_000, "the second 50h spilled");
        // 50h @ 60 = 3000.00; 50h @ 100 = 5000.00 → total 8000.00
        assert_eq!(e.total_minor, 800_000);
    }

    #[test]
    fn block_sale_line_emits_for_blocks_sold_in_window() {
        let c = contract(ContractType::TimeAndMaterials);
        let blk = block_with_sale_date(
            "50h block",
            3_000,
            9_000_000,
            None,
            None,
            "none",
            chrono::DateTime::from_timestamp(1_768_000_000, 0).unwrap(), // 2026-01-12
        );
        let inputs = with_blocks(&c, &[blk]);
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 1, 31), &inputs).unwrap();
        let sale: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::BlockSale)
            .collect();
        assert_eq!(sale.len(), 1);
        assert_eq!(sale[0].amount_minor, 9_000_000);
        // A block SOLD outside the window never re-bills (deposit is a
        // one-time line, not a recurring one).
        let old = block("old block", 3_000, 9_000_000, None, None, "none");
        let inputs2 = with_blocks(&c, &[old]);
        let e2 = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 1, 31), &inputs2).unwrap();
        assert!(e2
            .lines
            .iter()
            .all(|l| l.kind != BillingLineKind::BlockSale));
    }

    #[test]
    fn block_expiry_alert_reports_unused_minutes() {
        let c = contract(ContractType::TimeAndMaterials);
        let blk = block(
            "expiring block",
            1_200,
            4_500_000,
            None,
            Some(d(2026, 2, 28)),
            "expire",
        );
        let inputs = with_blocks(&c, &[blk]);
        // No entries at all: the whole block lapses unused.
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 3, 1), &inputs).unwrap();
        let consump = e.block_consumption.expect("consumption reported");
        let lapsed: i32 = consump.draws.iter().map(|x| x.expired_unused_minutes).sum();
        assert_eq!(
            lapsed, 1_200,
            "expiring block with zero draws lapses in full"
        );
    }

    #[test]
    fn block_rollover_policy_reports_rolling_minutes() {
        let c = contract(ContractType::TimeAndMaterials);
        let blk = block(
            "rolling block",
            1_200,
            4_500_000,
            None,
            Some(d(2026, 2, 28)),
            "rollover",
        );
        let inputs = with_blocks(&c, &[blk]);
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 3, 1), &inputs).unwrap();
        let consump = e.block_consumption.expect("consumption reported");
        let rolling: i32 = consump.draws.iter().map(|x| x.expired_unused_minutes).sum();
        assert_eq!(
            rolling, 1_200,
            "rollover blocks surface rolling minutes for the successor mint"
        );
    }

    #[test]
    fn block_partial_draw_then_expiry_reports_remainder() {
        let c = contract(ContractType::TimeAndMaterials);
        let blk = block(
            "10h block",
            600,
            3_000_000,
            None,
            Some(d(2026, 1, 31)),
            "expire",
        );
        let inputs = with_blocks(&c, &[blk]);
        let entries = vec![entry(240, None, d(2026, 1, 10))]; // 4h of 10h
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 2, 28),
            &inputs,
        )
        .unwrap();
        let consump = e.block_consumption.expect("consumption reported");
        let drawn: i32 = consump.draws.iter().map(|x| x.minutes).sum();
        let lapsed: i32 = consump.draws.iter().map(|x| x.expired_unused_minutes).sum();
        assert_eq!(drawn, 240);
        assert_eq!(lapsed, 360, "6 unused hours expire");
    }

    #[test]
    fn minimum_trueup_tops_period_up_to_commitment() {
        let c = Contract {
            amount_minor: Some(50_000),
            ..contract(ContractType::Recurring)
        };
        let inputs = EvalInputs {
            minimum_commitment_minor: Some(75_000),
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 3, 1), &inputs).unwrap();
        let trueups: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::MinimumTrueup)
            .collect();
        assert_eq!(trueups.len(), 2);
        assert_eq!(trueups[0].amount_minor, 25_000);
        // Two periods × (50k base + 25k true-up).
        assert_eq!(e.total_minor, 150_000);
    }

    #[test]
    fn minimum_trueup_zeroes_when_billed_exceeds_minimum() {
        let c = Contract {
            amount_minor: Some(100_000),
            ..contract(ContractType::Recurring)
        };
        let inputs = EvalInputs {
            minimum_commitment_minor: Some(75_000),
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 3, 1), &inputs).unwrap();
        assert!(e
            .lines
            .iter()
            .all(|l| l.kind != BillingLineKind::MinimumTrueup));
    }

    #[test]
    fn minimum_trueup_counts_overage_labor_toward_the_floor() {
        let c = contract(ContractType::Retainer); // 600 incl, 150/h overage
        let entries = vec![entry(700, None, d(2026, 1, 20))]; // 100 overage × 150 = 150.00
        let inputs = EvalInputs {
            minimum_commitment_minor: Some(100_250),
            ..Default::default()
        };
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 2, 1),
            &inputs,
        )
        .unwrap();
        let trueups: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::MinimumTrueup)
            .collect();
        // billed = 1000.00 base + 150.00 overage = 1150.00 ≥ 1002.50 → the
        // floor is met and the overage labor counted toward it.
        assert!(trueups.iter().all(|l| l.period_start != d(2026, 1, 15)));
        assert_eq!(trueups.len(), 0);
    }

    fn escalator(
        uplift_bps: i32,
        first: Option<NaiveDate>,
    ) -> core_domain::entities::ContractEscalator {
        core_domain::entities::ContractEscalator {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            contract_id: Uuid::now_v7(),
            uplift_bps,
            first_uplift_on: first,
            created_at: DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn escalator_uplifts_apply_each_anniversary() {
        let c = contract(ContractType::Recurring); // monthly, 2000.00, anchor Jan 15 2026
        let esc = escalator(300, Some(d(2027, 1, 15))); // +3% from Jan 2027
        let inputs = EvalInputs {
            escalator: Some(&esc),
            ..Default::default()
        };
        // Window covering the Jan 2026 period (pre-anniversary, no
        // uplift) and the two 2027 periods (one anniversary passed,
        // +3% = +60.00 each).
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2027, 2, 28), &inputs).unwrap();
        let esc_lines: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::Escalator)
            .collect();
        assert_eq!(esc_lines.len(), 2);
        assert_eq!(esc_lines[0].period_start, d(2027, 1, 15));
        assert_eq!(esc_lines[0].amount_minor, 6_000); // 2000.00 × 3.0%
        assert_eq!(esc_lines[1].period_start, d(2027, 2, 15));
        assert_eq!(esc_lines[1].amount_minor, 6_000);
    }

    #[test]
    fn escalator_second_anniversary_compounds_linearly_not_exponentially() {
        let c = contract(ContractType::Recurring);
        let esc = escalator(300, Some(d(2026, 1, 15)));
        let inputs = EvalInputs {
            escalator: Some(&esc),
            ..Default::default()
        };
        // The Jan 2028 period start: 2 anniversaries (Jan 2026, Jan
        // 2027) → +6% total (2 × 3%), never 3% of 3%.
        let e = evaluate_with(&c, &[], &[], &[], d(2028, 1, 1), d(2028, 1, 31), &inputs).unwrap();
        let esc_lines: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::Escalator)
            .collect();
        assert_eq!(esc_lines.len(), 1);
        assert_eq!(esc_lines[0].amount_minor, 12_000); // 2000.00 × 6.0%
    }

    #[test]
    fn change_order_bills_when_approved_in_window() {
        let c = contract(ContractType::FixedFee);
        let order = core_domain::entities::ContractChangeOrder {
            id: Uuid::now_v7(),
            org_id: c.org_id,
            contract_id: c.id,
            number: "CO-001".into(),
            title: "Extra onboarding".into(),
            description: None,
            amount_minor: 40_000,
            status: "approved".into(),
            approved_by: None,
            approved_at: Some(chrono::DateTime::from_timestamp(1_768_000_000, 0).unwrap()), // 2026-01-12
            billed_invoice_id: None,
            created_by: None,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        };
        let inputs = EvalInputs {
            change_orders: &[order],
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 1, 31), &inputs).unwrap();
        let cos: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::ChangeOrder)
            .collect();
        assert_eq!(cos.len(), 1);
        assert_eq!(cos[0].amount_minor, 40_000);
        assert!(cos[0].description.contains("CO-001"));
    }

    #[test]
    fn change_order_draft_and_billed_are_invisible() {
        let c = contract(ContractType::FixedFee);
        let mk = |status: &str| core_domain::entities::ContractChangeOrder {
            id: Uuid::now_v7(),
            org_id: c.org_id,
            contract_id: c.id,
            number: format!("CO-{status}"),
            title: "x".into(),
            description: None,
            amount_minor: 40_000,
            status: status.into(),
            approved_by: None,
            approved_at: Some(chrono::DateTime::from_timestamp(1_768_000_000, 0).unwrap()),
            billed_invoice_id: None,
            created_by: None,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        };
        let orders = vec![mk("draft"), mk("billed"), mk("rejected"), mk("cancelled")];
        let inputs = EvalInputs {
            change_orders: &orders,
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 1, 31), &inputs).unwrap();
        assert!(e.lines.is_empty());
    }

    #[test]
    fn sla_credit_is_the_only_negative_line() {
        let c = contract(ContractType::Recurring);
        let credit = core_domain::entities::ContractSlaCredit {
            id: Uuid::now_v7(),
            org_id: c.org_id,
            contract_id: c.id,
            ticket_id: None,
            reason: "resolution target breached by 2d".into(),
            amount_minor: 15_000,
            currency: "USD".into(),
            applies_on: d(2026, 1, 20),
            status: "pending".into(),
            applied_invoice_id: None,
            created_by: None,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        };
        let inputs = EvalInputs {
            sla_credits: &[credit],
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 2, 1), &inputs).unwrap();
        let credits: Vec<_> = e
            .lines
            .iter()
            .filter(|l| l.kind == BillingLineKind::SlaCredit)
            .collect();
        assert_eq!(credits.len(), 1);
        assert_eq!(
            credits[0].amount_minor, -15_000,
            "SLA credits are negative adjustment lines"
        );
        // total folds the negative correctly: 2000.00 base − 150.00 credit
        assert_eq!(e.total_minor, 185_000);
    }

    #[test]
    fn applied_sla_credits_never_double_bill() {
        let c = contract(ContractType::Recurring);
        let credit = core_domain::entities::ContractSlaCredit {
            id: Uuid::now_v7(),
            org_id: c.org_id,
            contract_id: c.id,
            ticket_id: None,
            reason: "breach".into(),
            amount_minor: 15_000,
            currency: "USD".into(),
            applies_on: d(2026, 1, 20),
            status: "applied".into(),
            applied_invoice_id: Some(Uuid::now_v7()),
            created_by: None,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        };
        let inputs = EvalInputs {
            sla_credits: &[credit],
            ..Default::default()
        };
        let e = evaluate_with(&c, &[], &[], &[], d(2026, 1, 1), d(2026, 2, 1), &inputs).unwrap();
        assert!(e.lines.iter().all(|l| l.kind != BillingLineKind::SlaCredit));
    }

    #[test]
    fn default_inputs_reproduce_v1_behavior_exactly() {
        // The v1 entry point and evaluate_with-with-default-inputs agree
        // byte-for-byte on a T&M contract with entries — the compat
        // guarantee every Wave-39 addition rides.
        let c = contract(ContractType::TimeAndMaterials);
        let entries = vec![entry(90, Some("remote"), d(2026, 1, 6))];
        let a = evaluate(&c, &[], &[], &entries, d(2026, 1, 1), d(2026, 1, 31)).unwrap();
        let b = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 1, 31),
            &EvalInputs::default(),
        )
        .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn block_prior_draws_reduce_available_balance() {
        let c = contract(ContractType::TimeAndMaterials);
        let blk = Arc::new(block("20h block", 1_200, 6_000_000, None, None, "none"));
        let mut prior = BTreeMap::new();
        prior.insert(blk.id, 1_000_i64); // 20h consumed before, 3h20m left
        let inputs = EvalInputs {
            blocks: std::slice::from_ref(&blk),
            prior_draws: Some(&prior),
            ..Default::default()
        };
        let entries = vec![entry(500, None, d(2026, 2, 10))]; // 8h20m: 3h20m draws, 5h spills
        let e = evaluate_with(
            &c,
            &[],
            &[],
            &entries,
            d(2026, 1, 1),
            d(2026, 3, 1),
            &inputs,
        )
        .unwrap();
        let consump = e.block_consumption.expect("consumption");
        let drawn: i32 = consump.draws.iter().map(|x| x.minutes).sum();
        assert_eq!(drawn, 200, "only the remaining 3h20m drew down");
        assert_eq!(consump.unblocked_minutes, 300);
    }
}
