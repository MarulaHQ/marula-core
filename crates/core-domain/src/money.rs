//! Money: integer minor units + ISO 4217 currency code. Floats are forbidden
//! anywhere near amounts (§5).

use crate::error::DomainError;
use std::fmt;
use std::str::FromStr;

/// ISO 4217 code, always uppercase, exactly three letters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CurrencyCode(String);

impl CurrencyCode {
    pub fn new(code: &str) -> Result<Self, DomainError> {
        let c = code.to_ascii_uppercase();
        if c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()) {
            Ok(Self(c))
        } else {
            Err(DomainError::Validation(format!(
                "invalid currency code: {code}"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Number of minor units per major unit for the currency (2 for USD/EUR,
    /// 0 for JPY, 3 for KWD/BHD). Covers all ISO 4217 exponents.
    pub fn exponent(&self) -> u32 {
        match self.0.as_str() {
            "JPY" | "KRW" | "VND" | "CLP" | "ISK" | "XOF" | "XAF" | "XPF" => 0,
            "KWD" | "BHD" | "OMR" | "JOD" | "TND" | "LYD" | "IQD" => 3,
            _ => 2,
        }
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CurrencyCode {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// An amount of money in minor units (e.g. cents). `i128` internally to make
/// accumulation overflow implausible; serializes as `{ minor, currency }`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Money {
    pub minor: i64,
    pub currency: CurrencyCode,
}

impl Money {
    pub const fn zero(currency: CurrencyCode) -> Self {
        Self { minor: 0, currency }
    }

    pub fn new(minor: i64, currency: CurrencyCode) -> Self {
        Self { minor, currency }
    }

    /// Parses a decimal major-unit string ("149.99", "-12.50") into minor units.
    pub fn parse_major(s: &str, currency: &CurrencyCode) -> Result<Self, DomainError> {
        let exp = currency.exponent();
        let (neg, body) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let (whole, frac) = match body.split_once('.') {
            Some((w, f)) => (w, f.to_string()),
            None => (body, String::new()),
        };
        if frac.len() > exp as usize {
            return Err(DomainError::Validation(format!(
                "too many decimal places for {currency} (max {exp})"
            )));
        }
        let whole: i64 = whole
            .parse()
            .map_err(|_| DomainError::Validation(format!("invalid amount: {s}")))?;
        let mut frac_val: i64 = if frac.is_empty() {
            0
        } else {
            frac.parse()
                .map_err(|_| DomainError::Validation(format!("invalid amount: {s}")))?
        };
        for _ in frac.len() as u32..exp {
            frac_val *= 10;
        }
        let minor = whole
            .checked_mul(10_i64.pow(exp))
            .and_then(|w| w.checked_add(frac_val))
            .ok_or_else(|| DomainError::MoneyOverflow(s.to_string(), currency.to_string()))?;
        Ok(Self { minor: if neg { -minor } else { minor }, currency: currency.clone() })
    }

    pub fn checked_add(&self, other: &Self) -> Result<Self, DomainError> {
        if self.currency != other.currency {
            return Err(DomainError::CurrencyMismatch(
                self.currency.to_string(),
                other.currency.to_string(),
            ));
        }
        self.minor
            .checked_add(other.minor)
            .map(|m| Self { minor: m, currency: self.currency.clone() })
            .ok_or_else(|| {
                DomainError::MoneyOverflow(self.minor.to_string(), other.minor.to_string())
            })
    }

    pub fn checked_sub(&self, other: &Self) -> Result<Self, DomainError> {
        if self.currency != other.currency {
            return Err(DomainError::CurrencyMismatch(
                self.currency.to_string(),
                other.currency.to_string(),
            ));
        }
        self.minor
            .checked_sub(other.minor)
            .map(|m| Self { minor: m, currency: self.currency.clone() })
            .ok_or_else(|| {
                DomainError::MoneyOverflow(self.minor.to_string(), other.minor.to_string())
            })
    }

    /// Percentage applied with banker's rounding (round-half-even) to keep
    /// invoice totals stable under summation.
    pub fn percent(&self, pct: i64) -> Result<Self, DomainError> {
        let raw = (self.minor as i128) * (pct as i128);
        let mut q = raw.div_euclid(100);
        let r = raw.rem_euclid(100);
        if r > 50 || (r == 50 && q % 2 != 0) {
            q += 1;
        }
        let minor = i64::try_from(q)
            .map_err(|_| DomainError::MoneyOverflow(raw.to_string(), "percent".into()))?;
        Ok(Self { minor, currency: self.currency.clone() })
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let exp = self.currency.exponent();
        let sign = if self.minor < 0 { "-" } else { "" };
        let abs = self.minor.unsigned_abs();
        let scale = 10_u64.pow(exp);
        write!(f, "{sign}{}.{:0width$}", abs / scale, abs % scale, width = exp as usize)?;
        write!(f, " {}", self.currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn usd() -> CurrencyCode {
        CurrencyCode::new("USD").unwrap()
    }

    #[test]
    fn parses_major_units() {
        let m = Money::parse_major("149.99", &usd()).unwrap();
        assert_eq!(m.minor, 14999);
        assert_eq!(m.to_string(), "149.99 USD");
    }

    #[test]
    fn rejects_excess_precision() {
        assert!(Money::parse_major("1.999", &usd()).is_err());
        // JPY has exponent 0
        let jpy = CurrencyCode::new("JPY").unwrap();
        assert!(Money::parse_major("1000", &jpy).unwrap().minor == 1000);
        assert!(Money::parse_major("1000.5", &jpy).is_err());
    }

    #[test]
    fn percent_rounds_half_even() {
        let m = Money::new(125, usd()); // 1.25
        assert_eq!(m.percent(10).unwrap().minor, 12); // 12.5 → 12 (even)
        let m2 = Money::new(175, usd()); // 1.75 → 17.5 → 18 (even)
        assert_eq!(m2.percent(10).unwrap().minor, 18);
    }

    proptest! {
        #[test]
        fn add_then_sub_is_identity(a in -10_000_000_i64..10_000_000_i64, b in -10_000_000_i64..10_000_000_i64) {
            let x = Money::new(a, usd());
            let y = Money::new(b, usd());
            let sum = x.checked_add(&y).unwrap();
            prop_assert_eq!(sum.checked_sub(&y).unwrap(), x);
        }

        #[test]
        fn parse_display_roundtrip(minor in -10_000_000_i64..10_000_000_i64) {
            let m = Money::new(minor, usd());
            let s = m.to_string();
            let (amt, cur) = s.rsplit_once(' ').unwrap();
            let reparsed = Money::parse_major(amt, &CurrencyCode::new(cur).unwrap()).unwrap();
            prop_assert_eq!(reparsed, m);
        }
    }
}
