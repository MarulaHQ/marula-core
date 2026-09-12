//! Explicit currency conversion (ADR 0002): one direction-stated operation,
//! rounding pinned to half-even (consistent with [`Money::percent`]).
//!
//! Purpose: the single core-domain conversion operation plus the
//! [`FxRateProvider`] adapter boundary. Conversion is EXPLICIT, never
//! implicit — there is no `From`/`Into`/`Mul` between currencies, no
//! live-rate lookup in the type system, and no API surface that converts
//! without naming the rate. The built-in provider is manual entry
//! (standalone "none" mode); managed daily feeds are bundles on the trait.
//!
//! Non-purpose: no FX revaluation, no realized/unrealized gain-loss (that
//! belongs to the books), no rate-feed transports here — pure domain
//! definitions plus the half-even arithmetic every caller shares.

use crate::error::DomainError;
use crate::money::{CurrencyCode, Money};
use chrono::NaiveDate;
use std::fmt;

/// Fixed-point scale for rates: 12 fractional decimal places, carried as
/// `i128`. `numeric(24,12)` in Postgres stores this losslessly; floats are
/// forbidden anywhere near money (§5), including rate handling.
pub const FX_RATE_SCALE: i128 = 1_000_000_000_000;

/// Half-even division of `numer / denom` (denom > 0), sign-symmetric:
/// ties round to the even neighbor, matching `Money::percent`.
#[must_use]
pub fn div_half_even(numer: i128, denom: i128) -> i128 {
    debug_assert!(denom > 0, "denominator must be positive");
    let neg = numer < 0;
    let n = numer.unsigned_abs() as i128;
    let q = n / denom;
    let r = n % denom;
    let twice = r * 2;
    let q = if twice > denom || (twice == denom && q % 2 != 0) {
        q + 1
    } else {
        q
    };
    if neg {
        -q
    } else {
        q
    }
}

/// A direction-stated exchange rate: `rate` QUOTE major units per one
/// FROM major unit ("1 from = rate to"). Conversions run only in the
/// stated direction; the reverse direction is an explicit
/// [`FxRate::inverse`], never an implicit flip.
///
/// The rate is carried at [`FX_RATE_SCALE`] (12 decimal places) as an
/// integer — the same shape `fx_rates.rate numeric(24,12)` stores, so the
/// DB value and the domain value agree bit-for-bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxRate {
    pub from: CurrencyCode,
    pub to: CurrencyCode,
    /// `rate × 10^12` (e.g. 17.85 ZAR/USD ⇒ 17_850_000_000_000).
    pub scaled: i128,
}

impl FxRate {
    /// Builds a rate from a decimal string ("17.85", "0.056022408963").
    /// At most 12 fractional digits, strictly positive, from ≠ to.
    pub fn new(from: CurrencyCode, to: CurrencyCode, rate: &str) -> Result<Self, DomainError> {
        if from == to {
            return Err(DomainError::Validation(format!(
                "fx rate pair must differ: {from}"
            )));
        }
        let scaled = parse_decimal_scaled(rate, 12).ok_or_else(|| {
            DomainError::Validation(format!(
                "invalid fx rate: {rate} (positive decimal, max 12 fractional digits)"
            ))
        })?;
        Ok(Self { from, to, scaled })
    }

    /// Assembles from the DB's `numeric(24,12)` text form.
    pub fn from_db_text(
        from: CurrencyCode,
        to: CurrencyCode,
        rate_text: &str,
    ) -> Result<Self, DomainError> {
        Self::new(from, to, rate_text)
    }

    /// The stored precision form ("17.850000000000") — what
    /// `fx_rates.rate` holds.
    pub fn db_text(&self) -> String {
        format_rate_scaled(self.scaled)
    }

    /// The reverse direction at full 12-place scale, half-even: converting
    /// A→B and back need not round-trip exactly (corrections are facts),
    /// but the inversion itself is deterministic.
    #[must_use]
    pub fn inverse(&self) -> Self {
        let scaled = div_half_even(FX_RATE_SCALE * FX_RATE_SCALE, self.scaled);
        Self {
            from: self.to.clone(),
            to: self.from.clone(),
            scaled,
        }
    }
}

impl fmt::Display for FxRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "1 {} = {} {}", self.from, self.db_text(), self.to)
    }
}

/// Parses a positive decimal string into `value × 10^max_frac` when it has
/// at most `max_frac` fractional digits. `None` on any other input.
fn parse_decimal_scaled(s: &str, max_frac: u32) -> Option<i128> {
    let body = s.trim();
    if body.is_empty() {
        return None;
    }
    let (whole, frac) = match body.split_once('.') {
        Some((w, f)) => (w, f),
        None => (body, ""),
    };
    let max_frac_usize = max_frac as usize;
    if frac.len() > max_frac_usize {
        return None;
    }
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    let whole_val: i128 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    if whole_val < 0 {
        return None;
    }
    let mut frac_val: i128 = if frac.is_empty() {
        0
    } else {
        frac.parse().ok()?
    };
    if frac_val < 0 {
        return None;
    }
    for _ in frac.len()..max_frac_usize {
        frac_val *= 10;
    }
    let total = whole_val
        .checked_mul(10_i128.pow(max_frac))?
        .checked_add(frac_val)?;
    if total == 0 {
        return None;
    }
    Some(total)
}

fn format_rate_scaled(scaled: i128) -> String {
    let neg = scaled < 0;
    let abs = scaled.unsigned_abs();
    let whole = abs / 1_000_000_000_000u128;
    let frac = abs % 1_000_000_000_000u128;
    if neg && (whole != 0 || frac != 0) {
        format!("-{whole}.{frac:012}")
    } else {
        format!("{whole}.{frac:012}")
    }
}

impl FxRate {
    pub fn parse(from: CurrencyCode, to: CurrencyCode, rate: &str) -> Result<Self, DomainError> {
        Self::new(from, to, rate)
    }
}

/// The immutable conversion snapshot written at one of the five lock
/// moments (expense/purchase entry, invoice issue, time-entry costing,
/// inventory receipt, payment settlement). Once written it is never
/// rewritten; reports sum `base_amount` from snapshots and never
/// re-convert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxSnapshot {
    /// Direction-stated rate in the FACT → BASE direction:
    /// base_amount = half_even(original × rate_used).
    pub rate_used: FxRate,
    /// The rate date the books recognize.
    pub rate_date: NaiveDate,
    /// Where the rate came from ('manual' for the built-in provider).
    pub source: String,
    /// The base-currency equivalent, locked forever.
    pub base_amount: Money,
}

impl FxSnapshot {
    /// The persisted column tuple for the snapshot group
    /// (`fx_rate_used numeric(24,12)`, `fx_rate_date`, `fx_source`,
    /// base amount): the rate text reproduces `base_amount` from the
    /// original by multiplication, losslessly at 12 decimal places.
    #[must_use]
    pub fn columns(&self) -> (String, NaiveDate, &str, i64) {
        (
            self.rate_used.db_text(),
            self.rate_date,
            self.source.as_str(),
            self.base_amount.minor,
        )
    }

    /// Converts and assembles the snapshot in one explicit step.
    pub fn lock(
        original: &Money,
        base_currency: &CurrencyCode,
        rate_used: FxRate,
        rate_date: NaiveDate,
        source: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let base_amount = original.convert(&rate_used)?;
        if base_amount.currency != *base_currency {
            return Err(DomainError::CurrencyMismatch(
                base_amount.currency.to_string(),
                base_currency.to_string(),
            ));
        }
        Ok(Self {
            rate_used,
            rate_date,
            source: source.into(),
            base_amount,
        })
    }
}

impl Money {
    /// Explicit conversion through a direction-stated rate. Fails unless
    /// `self.currency == rate.from`; the result carries `rate.to`. Rounding
    /// is half-even over the exact rational product — no floats anywhere.
    ///
    /// Minor-unit handling: the rate is a major-unit ratio, so the factor
    /// `10^(to_exp − from_exp)` re-scales between exponents (USD↔JPY works
    /// without magic 100s).
    pub fn convert(&self, rate: &FxRate) -> Result<Money, DomainError> {
        if self.currency != rate.from {
            return Err(DomainError::CurrencyMismatch(
                self.currency.to_string(),
                rate.from.to_string(),
            ));
        }
        let from_exp = i128::from(self.currency.exponent());
        let to_exp = i128::from(rate.to.exponent());
        // quote_minor = half_even(minor × rate × 10^to_exp / (10^from_exp × 10^12))
        let numer = (self.minor as i128)
            .checked_mul(rate.scaled)
            .and_then(|v| v.checked_mul(10_i128.pow(to_exp as u32)))
            .ok_or_else(|| {
                DomainError::MoneyOverflow(self.minor.to_string(), rate.scaled.to_string())
            })?;
        let denom = 10_i128
            .pow(from_exp as u32)
            .checked_mul(FX_RATE_SCALE)
            .expect("scale product fits i128");
        let minor = i64::try_from(div_half_even(numer, denom))
            .map_err(|_| DomainError::MoneyOverflow(numer.to_string(), denom.to_string()))?;
        Ok(Money {
            minor,
            currency: rate.to.clone(),
        })
    }
}

/// The `FxRateProvider` trait boundary (§4.7 traits at the edge): every
/// rate source sits behind this trait; no rate service leaks past it.
///
/// The built-in implementation is manual entry over `fx_rates` (the
/// standalone "none" mode — zero network calls); managed daily feeds
/// (ECB/OFX/OpenExchangeRates-class) are commercial or community bundles
/// implementing the same trait. Implementations must be `Send + Sync`;
/// the async shape mirrors [`crate::completion::CompletionProvider`]
/// (RPITIT, dyn-free).
pub trait FxRateProvider: Send + Sync {
    /// Human/machine name of the provider ("manual", "ecb-daily", …).
    fn name(&self) -> &str;

    /// Resolves the effective rate for one direction on one date, or
    /// fails with a typed error. Implementations must never fabricate a
    /// 1:1 default and must never block the caller's event loop.
    fn resolve(
        &self,
        req: &FxRateRequest,
    ) -> impl std::future::Future<Output = Result<FxRate, FxError>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxRateRequest {
    pub org_id: uuid::Uuid,
    pub from: CurrencyCode,
    pub to: CurrencyCode,
    /// The date the rate must be effective on (or the latest on/before).
    pub as_of: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FxError {
    /// No rate covers the requested direction/date: manual entry is
    /// required (the fail-closed "none" mode).
    #[error("no fx rate for {from}→{to} effective {as_of}: record one before converting")]
    NoRate {
        from: String,
        to: String,
        as_of: NaiveDate,
    },
    #[error("fx rate direction is ambiguous for pair {0}/{1}")]
    AmbiguousPair(String, String),
    #[error("fx provider backend error: {reason}")]
    Backend { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(code: &str) -> CurrencyCode {
        CurrencyCode::new(code).unwrap()
    }

    #[test]
    fn converts_with_half_even_rounding() {
        // 1 USD = 17.85 ZAR; 100.00 USD → 1785.00 ZAR = 178_500 minor
        let rate = FxRate::new(cc("USD"), cc("ZAR"), "17.85").unwrap();
        let m = Money::new(10_000, cc("USD"));
        assert_eq!(m.convert(&rate).unwrap(), Money::new(178_500, cc("ZAR")));

        // 1 ZAR = 0.056022408963865... USD (inverse); 1.7850 ZAR → 0.10 USD
        let inv = rate.inverse();
        let m = Money::new(178, cc("ZAR"));
        // 178 × 0.056022408964 = 9.972 → 10 minor (rounded half-even at the end)
        assert_eq!(inv.to.exponent(), 2);
        let out = m.convert(&inv).unwrap();
        assert_eq!(out.minor, 10, "got {out}");
    }

    #[test]
    fn ties_round_half_even() {
        // 1.00 USD at 0.125 GBP = 0.125 GBP → 12.5 minor → 12 (even)
        let rate = FxRate::new(cc("USD"), cc("GBP"), "0.125").unwrap();
        let m = Money::new(100, cc("USD"));
        assert_eq!(m.convert(&rate).unwrap().minor, 12);
        // 0.375 → 37.5 → 38 (even)
        let rate2 = FxRate::new(cc("USD"), cc("GBP"), "0.375").unwrap();
        assert_eq!(m.convert(&rate2).unwrap().minor, 38);
    }

    #[test]
    fn direction_is_enforced() {
        let rate = FxRate::new(cc("USD"), cc("ZAR"), "17.85").unwrap();
        let zar = Money::new(100, cc("ZAR"));
        assert!(matches!(
            zar.convert(&rate),
            Err(DomainError::CurrencyMismatch(_, _))
        ));
    }

    #[test]
    fn cross_exponent_conversion() {
        // USD (2) → KWD (3): 1.00 USD at 0.307 KWD = 0.307 KWD = 307 minor
        let rate = FxRate::new(cc("USD"), cc("KWD"), "0.307").unwrap();
        let m = Money::new(100, cc("USD"));
        assert_eq!(m.convert(&rate).unwrap().minor, 307);
        // JPY (0) → USD (2): 1000 JPY at 0.0067 USD = 6.70 USD = 670 minor
        let rate2 = FxRate::new(cc("JPY"), cc("USD"), "0.0067").unwrap();
        let jpy = Money::new(1000, cc("JPY"));
        assert_eq!(jpy.convert(&rate2).unwrap().minor, 670);
    }

    #[test]
    fn inverse_is_half_even_at_scale() {
        // 1/rate for 3 → 0.333... at 12 places: 333333333333.333… → 333333333333 (tie? 1/3*1e12 = 333333333333.333 → down)
        let rate = FxRate::new(cc("AAB"), cc("XYZ"), "3").unwrap();
        let inv = rate.inverse();
        assert_eq!(inv.scaled, 333_333_333_333);
        // 1/6 = 0.1666…×1e12 = 166666666666.67 → 166666666667
        let rate2 = FxRate::new(cc("AAB"), cc("XYZ"), "6").unwrap();
        assert_eq!(rate2.inverse().scaled, 166_666_666_667);
        // exact tie: rate 8 → 0.125 exactly
        let rate3 = FxRate::new(cc("AAB"), cc("XYZ"), "8").unwrap();
        assert_eq!(rate3.inverse().scaled, 125_000_000_000);
    }

    #[test]
    fn rejects_bad_rates() {
        assert!(FxRate::new(cc("USD"), cc("ZAR"), "0").is_err());
        assert!(FxRate::new(cc("USD"), cc("ZAR"), "-1.5").is_err());
        assert!(FxRate::new(cc("USD"), cc("ZAR"), "1.1234567890123").is_err()); // 13 dp
        assert!(FxRate::new(cc("USD"), cc("ZAR"), "abc").is_err());
        assert!(FxRate::new(cc("USD"), cc("USD"), "1").is_err());
        assert!(FxRate::new(cc("USD"), cc("ZAR"), "17.85").is_ok());
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        #[test]
        fn convert_then_inverse_stays_in_range(minor in 1_i64..10_000_000_i64) {
            use proptest::prelude::prop_assert;
            let rate = FxRate::new(cc("USD"), cc("ZAR"), "17.85").unwrap();
            let usd = Money::new(minor, cc("USD"));
            let zar = usd.convert(&rate).unwrap();
            let back = zar.convert(&rate.inverse()).unwrap();
            // 12-place scale keeps the round trip within 1 minor unit.
            prop_assert!((back.minor - minor).abs() <= 1, "{minor} → {} → {}", zar.minor, back.minor);
        }
    }
}
