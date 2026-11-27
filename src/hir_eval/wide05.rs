//! Heap representation and algorithms reached only when an exact value is
//! decimal or cannot be held in the inline fraction.
use super::{decimal05::Decimal, CoreOp, Domain, Exact, NumericResult, DIV_BY_ZERO, V};
use num_bigint::{BigInt, BigUint};
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::{fmt, rc::Rc};

pub(super) const MAX_BITS: u64 = 1_048_576;
pub(super) const RESOURCE: &str = "exact numeric resource limit exceeded (1048576 bits)";

#[derive(Clone)]
pub(super) enum Wide {
    Exact { ratio: BigRational, domain: Domain },
    Decimal(Decimal),
}
impl fmt::Display for Wide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact { ratio, .. } => write!(f, "{ratio}"),
            Self::Decimal(d) => write!(f, "{d}"),
        }
    }
}
pub(super) fn guard(x: &BigInt) -> NumericResult<()> {
    if x.bits() > MAX_BITS {
        Err(RESOURCE)
    } else {
        Ok(())
    }
}
pub(super) fn parse_int(s: &str) -> NumericResult<BigInt> {
    if s.len() > 315_655 {
        return Err(RESOURCE);
    }
    let n = s
        .parse::<BigInt>()
        .map_err(|_| "expected an exact integer")?;
    guard(&n)?;
    Ok(n)
}
pub(super) fn ratio(v: &V) -> NumericResult<BigRational> {
    Ok(match v {
        V::Exact(x) => BigRational::new_raw(BigInt::from(x.n), BigInt::from(x.d)),
        V::Wide(x) => match x.as_ref() {
            Wide::Exact { ratio, .. } => ratio.clone(),
            Wide::Decimal(x) => x.exact()?,
        },
        _ => return Err("expected an exact number"),
    })
}
pub(super) fn kind(v: &V) -> Option<Domain> {
    match v {
        V::Exact(x) => Some(x.domain),
        V::Wide(x) => match x.as_ref() {
            Wide::Exact { domain, .. } => Some(*domain),
            Wide::Decimal(_) => Some(Domain::Decimal),
        },
        _ => None,
    }
}
pub(super) fn ratio_value(ratio: BigRational, domain: Domain) -> NumericResult<V> {
    guard(ratio.numer())?;
    guard(ratio.denom())?;
    match domain {
        Domain::Natural if ratio.numer().is_negative() || !ratio.is_integer() => {
            return Err("cannot represent value as n")
        }
        Domain::Integer if !ratio.is_integer() => return Err("cannot represent value as z"),
        Domain::Decimal => return Err("decimal representation requires explicit scale metadata"),
        _ => {}
    }
    Ok(
        if let (Some(n), Some(d)) = (ratio.numer().to_i128(), ratio.denom().to_i128()) {
            V::Exact(Exact { n, d, domain })
        } else {
            V::Wide(Rc::new(Wide::Exact { ratio, domain }))
        },
    )
}
pub(super) fn integral(v: &V) -> NumericResult<BigInt> {
    if let V::Exact(n) = v {
        return Ok(BigInt::from(n.integral()?));
    }
    let ratio = ratio(v)?;
    if !ratio.is_integer() {
        return Err("expected an integral value");
    }
    Ok(ratio.into_raw().0)
}
pub(super) fn exponent(b: &V) -> NumericResult<BigInt> {
    let e = integral(b)?;
    if e.is_negative() || kind(b) != Some(Domain::Natural) {
        return Err("power requires a natural exponent");
    }
    Ok(e)
}
pub(super) fn binary(op: CoreOp, a: &V, b: &V, domain: Domain) -> NumericResult<V> {
    use CoreOp::*;
    let x = ratio(a)?;
    let y = ratio(b)?;
    let result = match op {
        Add => x + y,
        Sub => {
            let z = x - y;
            if domain == Domain::Natural && z.is_negative() {
                return Err("natural subtraction underflow");
            }
            z
        }
        Mul => x * y,
        Div => {
            if y.is_zero() {
                return Err(DIV_BY_ZERO);
            }
            x / y
        }
        Quotient | Remainder => {
            if !x.is_integer() || !y.is_integer() {
                return Err("expected an integral value");
            }
            let (x, y) = (x.into_raw().0, y.into_raw().0);
            if y.is_zero() {
                return Err(DIV_BY_ZERO);
            }
            BigRational::from_integer(if op == Quotient {
                x.div_floor(&y)
            } else {
                x.mod_floor(&y)
            })
        }
        Pow => {
            let e = exponent(b)?;
            if e.is_zero() {
                BigRational::one()
            } else if x.is_zero() {
                BigRational::zero()
            } else if x.numer().magnitude() == x.denom().magnitude() {
                BigRational::from_integer(BigInt::from(if x.is_negative() && e.is_odd() {
                    -1
                } else {
                    1
                }))
            } else {
                let exp = e.to_u32().ok_or(RESOURCE)?;
                let largest = x.numer().bits().max(x.denom().bits());
                if (largest - 1).saturating_mul(u64::from(exp)) >= MAX_BITS {
                    return Err(RESOURCE);
                }
                // Reduced numerator and denominator stay relatively prime under a power.
                BigRational::new_raw(x.numer().pow(exp), x.denom().pow(exp))
            }
        }
        _ => return Err("invalid wide exact operation"),
    };
    ratio_value(result, if op == Div { Domain::Rational } else { domain })
}

/// Convert a stored finite binary64 payload to its exact mathematical value.
pub(super) fn from_real(value: f64) -> NumericResult<BigRational> {
    if !value.is_finite() {
        return Err("nonfinite real value");
    }
    let bits = value.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32;
    let m = bits & ((1u64 << 52) - 1);
    let (m, shift) = if e == 0 {
        (m, -1074)
    } else {
        (m | (1u64 << 52), e - 1075)
    };
    let mut n = BigInt::from(m);
    if bits >> 63 != 0 {
        n = -n
    }
    Ok(if shift >= 0 {
        BigRational::from_integer(n << (shift as usize))
    } else {
        BigRational::new(n, BigInt::one() << ((-shift) as usize))
    })
}

/// Round a reduced exact fraction straight to IEEE binary64; no double rounding.
pub(super) fn to_real(x: &BigRational) -> NumericResult<f64> {
    if x.is_zero() {
        return Ok(0.0);
    }
    let sign = u64::from(x.is_negative()) << 63;
    let n = x.numer().magnitude();
    let d = x.denom().magnitude();
    let mut e = n.bits() as i64 - d.bits() as i64;
    if e > 1024 {
        return Err("nonfinite real result");
    }
    if e < -1075 {
        return Ok(f64::from_bits(sign));
    }
    if if e >= 0 {
        n < &(d << (e as usize))
    } else {
        &(n << ((-e) as usize)) < d
    } {
        e -= 1;
    }
    if e >= 1024 {
        return Err("nonfinite real result");
    }
    let normal = e >= -1022;
    let scale = if normal { 52 - e } else { 1074 };
    let round = |n: &BigUint, d: &BigUint| -> NumericResult<u64> {
        let (q, r) = n.div_rem(d);
        let bump = (&r << 1usize).cmp(d);
        let q = q.to_u64().ok_or("nonfinite real result")?;
        Ok(q + u64::from(bump.is_gt() || (bump.is_eq() && q & 1 == 1)))
    };
    let mut q = if scale >= 0 {
        round(&(n << (scale as usize)), d)?
    } else {
        round(n, &(d << ((-scale) as usize)))?
    };
    if !normal {
        return Ok(f64::from_bits(sign | q));
    }
    if q == (1u64 << 53) {
        q >>= 1;
        e += 1;
    }
    if e >= 1024 {
        return Err("nonfinite real result");
    }
    Ok(f64::from_bits(
        sign | (((e + 1023) as u64) << 52) | (q & ((1u64 << 52) - 1)),
    ))
}
