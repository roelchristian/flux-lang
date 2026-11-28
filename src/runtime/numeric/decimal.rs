//! Exact decimals preserve their written scale; the coefficient is signed.
use super::super::{as_ratio, CoreOp, NumericResult, V};
use super::wide::{self as wide, Wide};
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};
use std::{fmt, rc::Rc};

pub(in crate::hir_eval) const MAX_DECIMAL_SCALE: u32 = 10_000;
const SCALE_LIMIT: &str = "decimal scale exceeds the resource limit (10000)";
const COEFFICIENT_LIMIT: &str = "decimal coefficient exceeds the resource bit limit";
const NONFINITE_DECIMAL: &str = "exact value has no finite decimal expansion";

#[derive(Clone, Debug)]
pub(in crate::hir_eval) struct Decimal {
    coefficient: BigInt,
    scale: u32,
}

fn pow10(scale: u32) -> BigInt {
    BigInt::from(10).pow(scale)
}

fn check_scale(scale: usize) -> NumericResult<u32> {
    if scale > MAX_DECIMAL_SCALE as usize {
        Err(SCALE_LIMIT)
    } else {
        Ok(scale as u32)
    }
}

impl Decimal {
    fn new(coefficient: BigInt, scale: u32) -> NumericResult<Self> {
        check_scale(scale as usize)?;
        if coefficient.bits() > wide::max_bits() {
            return Err(COEFFICIENT_LIMIT);
        }
        Ok(Self { coefficient, scale })
    }

    fn parse(raw: &str) -> NumericResult<Self> {
        let (negative, unsigned) = match raw.as_bytes().first() {
            Some(b'-') => (true, &raw[1..]),
            Some(b'+') => (false, &raw[1..]),
            _ => (false, raw),
        };
        let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        if whole.is_empty() && fraction.is_empty()
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return Err("expected an exact decimal");
        }
        let scale = check_scale(fraction.len())?;
        let whole = whole.trim_start_matches('0');
        let fraction = if whole.is_empty() {
            fraction.trim_start_matches('0')
        } else {
            fraction
        };
        // Decimal digits carry at least three bits each after the first.
        if (whole.len() + fraction.len()).saturating_sub(1) > wide::decimal_digit_limit() {
            return Err(COEFFICIENT_LIMIT);
        }
        let coefficient = if whole.is_empty() && fraction.is_empty() {
            BigInt::zero()
        } else {
            let digits = format!("{whole}{fraction}");
            BigInt::parse_bytes(digits.as_bytes(), 10).ok_or("expected an exact decimal")?
        };
        Self::new(if negative { -coefficient } else { coefficient }, scale)
    }

    fn from_exact(ratio: &BigRational) -> NumericResult<Self> {
        let denom = ratio.denom();
        let twos = denom.trailing_zeros().unwrap_or(0);
        // A normalized successful expansion needs scale at least twos.
        let twos = check_scale(usize::try_from(twos).unwrap_or(usize::MAX))?;
        let mut residual = denom >> twos;
        let five = BigInt::from(5);
        let mut fives = 0;
        while !residual.is_one() {
            let (next, remainder) = residual.div_rem(&five);
            if !remainder.is_zero() {
                return Err(NONFINITE_DECIMAL);
            }
            fives += 1;
            if fives > MAX_DECIMAL_SCALE {
                return Err(SCALE_LIMIT);
            }
            residual = next;
        }
        let scale = twos.max(fives);
        let coefficient = (ratio.numer() << (scale - twos)) * five.pow(scale - fives);
        Self::new(coefficient, scale)
    }

    pub(in crate::hir_eval) fn storage_bits(&self) -> u64 {
        self.coefficient.bits()
    }

    pub(in crate::hir_eval) fn exact(&self) -> NumericResult<BigRational> {
        Ok(BigRational::new(
            self.coefficient.clone(),
            pow10(self.scale),
        ))
    }

    fn add_or_sub(&self, other: &Self, subtract: bool) -> NumericResult<Self> {
        let scale = self.scale.max(other.scale);
        let a = &self.coefficient * pow10(scale - self.scale);
        let b = &other.coefficient * pow10(scale - other.scale);
        Self::new(if subtract { a - b } else { a + b }, scale)
    }

    fn mul(&self, other: &Self) -> NumericResult<Self> {
        let scale = check_scale((self.scale + other.scale) as usize)?;
        if self
            .coefficient
            .bits()
            .saturating_add(other.coefficient.bits())
            .saturating_sub(1)
            > wide::max_bits()
            && !self.coefficient.is_zero()
            && !other.coefficient.is_zero()
        {
            return Err(COEFFICIENT_LIMIT);
        }
        Self::new(&self.coefficient * &other.coefficient, scale)
    }

    fn pow(&self, exponent: &BigInt) -> NumericResult<Self> {
        use num_traits::ToPrimitive;
        if exponent.is_negative() {
            return Err("decimal exponent must be natural");
        }
        let scale = if self.scale == 0 {
            0
        } else {
            check_scale(
                exponent
                    .to_usize()
                    .unwrap_or(usize::MAX)
                    .saturating_mul(self.scale as usize),
            )?
        };
        if exponent.is_zero() {
            return Self::new(BigInt::one(), scale);
        }
        if self.coefficient.is_zero() || self.coefficient.is_one() {
            return Self::new(self.coefficient.clone(), scale);
        }
        if self.coefficient == BigInt::from(-1) {
            return Self::new(BigInt::from(if exponent.is_odd() { -1 } else { 1 }), scale);
        }
        let exponent = exponent.to_u32().ok_or(COEFFICIENT_LIMIT)?;
        if self
            .coefficient
            .bits()
            .saturating_sub(1)
            .saturating_mul(u64::from(exponent))
            >= wide::max_bits()
        {
            return Err(COEFFICIENT_LIMIT);
        }
        Self::new(self.coefficient.pow(exponent), scale)
    }

    fn quantize(ratio: &BigRational, scale: u32) -> NumericResult<Self> {
        let scaled = ratio.numer() * pow10(scale);
        let (mut coefficient, remainder) = scaled.div_rem(ratio.denom());
        let doubled = remainder.abs() << 1_u32;
        if doubled > *ratio.denom() || doubled == *ratio.denom() && coefficient.is_odd() {
            if remainder.is_negative() {
                coefficient -= 1;
            } else {
                coefficient += 1;
            }
        }
        Self::new(coefficient, scale)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale == 0 {
            return write!(f, "{}", self.coefficient);
        }
        if self.coefficient.is_negative() {
            f.write_str("-")?;
        }
        let digits = self.coefficient.magnitude().to_str_radix(10);
        let scale = self.scale as usize;
        if digits.len() <= scale {
            f.write_str("0.")?;
            for _ in 0..(scale - digits.len()) {
                f.write_str("0")?;
            }
            f.write_str(&digits)
        } else {
            let (whole, fraction) = digits.split_at(digits.len() - scale);
            write!(f, "{whole}.{fraction}")
        }
    }
}

fn wrap(decimal: Decimal) -> V {
    V::Wide(Rc::new(Wide::Decimal(decimal)))
}

fn ratio(value: &V) -> NumericResult<BigRational> {
    let (num, denom) = as_ratio(value)?;
    Ok(BigRational::new_raw(num, denom))
}

fn with_decimal<T>(value: &V, f: impl FnOnce(&Decimal) -> NumericResult<T>) -> NumericResult<T> {
    if let V::Wide(wide) = value {
        if let Wide::Decimal(d) = wide.as_ref() {
            return f(d);
        }
    }
    f(&Decimal::from_exact(&ratio(value)?)?)
}

pub(in crate::hir_eval) fn decimal_literal(raw: &str) -> NumericResult<V> {
    Decimal::parse(raw).map(wrap)
}

pub(in crate::hir_eval) fn decimal_from_exact(value: &V) -> NumericResult<V> {
    with_decimal(value, |d| Ok(wrap(d.clone())))
}

pub(in crate::hir_eval) fn decimal_binary(op: CoreOp, a: &V, b: &V) -> NumericResult<V> {
    with_decimal(a, |a| {
        if op == CoreOp::Pow {
            let (exponent, denominator) = as_ratio(b)?;
            if !denominator.is_one() {
                return Err("decimal exponent must be natural");
            }
            return a.pow(&exponent).map(wrap);
        }
        with_decimal(b, |b| {
            match op {
                CoreOp::Add => a.add_or_sub(b, false),
                CoreOp::Sub => a.add_or_sub(b, true),
                CoreOp::Mul => a.mul(b),
                _ => Err("unsupported decimal operation"),
            }
            .map(wrap)
        })
    })
}

pub(in crate::hir_eval) fn decimal_unary(op: CoreOp, value: &V) -> NumericResult<V> {
    with_decimal(value, |d| {
        let coefficient = match op {
            CoreOp::Negate => -&d.coefficient,
            CoreOp::Abs => d.coefficient.abs(),
            _ => return Err("unsupported decimal operation"),
        };
        Decimal::new(coefficient, d.scale).map(wrap)
    })
}

pub(in crate::hir_eval) fn quantize(value: &V, scale: usize) -> NumericResult<V> {
    let scale = check_scale(scale)?;
    if let V::Wide(wide) = value {
        if let Wide::Decimal(d) = wide.as_ref() {
            if d.scale == scale {
                return Ok(value.clone());
            }
        }
    }
    Decimal::quantize(&ratio(value)?, scale).map(wrap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_scale_and_wide_coefficient() {
        let a = Decimal::parse("999999999999999999999999999999999999999999999.2300").unwrap();
        let b = Decimal::parse("-0.23").unwrap();
        assert!(a
            .add_or_sub(&b, false)
            .unwrap()
            .to_string()
            .ends_with(".0000"));
        assert_eq!(b.pow(&BigInt::from(2)).unwrap().to_string(), "0.0529");
        assert_eq!(
            b.mul(&Decimal::parse("0.000").unwrap())
                .unwrap()
                .to_string(),
            "0.00000"
        );
    }

    #[test]
    fn scale_limit_is_separate_from_value_magnitude() {
        let too_small = format!("0.{}1", "0".repeat(MAX_DECIMAL_SCALE as usize));
        assert_eq!(Decimal::parse(&too_small).unwrap_err(), SCALE_LIMIT);
        // Appending zeros is useful scale even if coefficient is zero.
        let zero = Decimal::parse(&format!("0.{}", "0".repeat(5_001))).unwrap();
        assert_eq!(zero.mul(&zero).unwrap_err(), SCALE_LIMIT);
        assert_eq!(zero.pow(&BigInt::zero()).unwrap().to_string(), "1");
        assert_eq!(
            Decimal::parse("0.00")
                .unwrap()
                .pow(&BigInt::from(1_000_000))
                .unwrap_err(),
            SCALE_LIMIT
        );
    }

    #[test]
    fn finite_conversion_and_half_even_rounding() {
        let rat = |n: i32, d: i32| BigRational::new(BigInt::from(n), BigInt::from(d));
        assert_eq!(
            Decimal::from_exact(&rat(-3, 40)).unwrap().to_string(),
            "-0.075"
        );
        assert!(Decimal::from_exact(&rat(1, 3)).is_err());
        for (n, d, expected) in [
            (1, 8, "0.12"),
            (3, 8, "0.38"),
            (-1, 8, "-0.12"),
            (-3, 8, "-0.38"),
            (1999, 200, "10.00"),
        ] {
            assert_eq!(
                Decimal::quantize(&rat(n, d), 2).unwrap().to_string(),
                expected
            );
        }
    }
}
