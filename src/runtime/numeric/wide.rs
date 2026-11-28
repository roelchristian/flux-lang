//! Heap representation and algorithms reached only when an exact value is
//! decimal or cannot be held in the inline fraction.
use super::{decimal05::Decimal, CoreOp, Domain, Exact, NumericResult, DIV_BY_ZERO, V};
use num_bigint::{BigInt, BigUint};
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::{fmt, rc::Rc};

pub(super) const RESOURCE: &str = "exact numeric resource bit limit exceeded";

pub(super) fn max_bits() -> u64 {
    super::resources::limits().max_exact_bits
}

pub(super) fn decimal_digit_limit() -> usize {
    // 30103/100000 is just above log10(2); leave room for a sign and rounding.
    ((u128::from(max_bits()) * 30_103 / 100_000 + 4).min(usize::MAX as u128)) as usize
}

pub(super) fn guard_inline(value: Exact) -> NumericResult<Exact> {
    let limit = max_bits();
    if limit < 128 {
        let bits = |n: i128| u64::from(u128::BITS - n.unsigned_abs().leading_zeros());
        if bits(value.n).max(bits(value.d)) > limit {
            return Err(RESOURCE);
        }
    }
    Ok(value)
}

pub(super) fn guard_value(value: V) -> NumericResult<V> {
    if let V::Exact(exact) = value {
        guard_inline(exact)?;
    }
    Ok(value)
}

/// User data and callback-built values have already gone through the scalar
/// guards. Some primitives synthesize integers inside a result, sequence, or
/// record (for example date offsets and stable sort indices). Check those
/// value boundaries under narrower host caps. The default fast path makes
/// no walk, and nested values use an explicit depth-first stack.
pub(super) fn guard_embedded(value: V) -> NumericResult<V> {
    let limit = max_bits();
    if limit >= 128 {
        return Ok(value);
    }
    let bits = |n: i128| u64::from(u128::BITS - n.unsigned_abs().leading_zeros());
    enum Next {
        Value(V),
        Items(Rc<[V]>, usize),
        Dense(Rc<super::arrays012::Dense>, usize),
        Record(Rc<[(Box<str>, V)]>, usize),
        Dict(Rc<[(V, V)]>, usize),
    }
    let mut pending = vec![Next::Value(value.clone())];
    while let Some(next) = pending.pop() {
        match next {
            Next::Value(item) => match item {
                V::Exact(n) if bits(n.n).max(bits(n.d)) > limit => return Err(RESOURCE),
                V::Sequence(xs) | V::Tuple(xs) | V::Table(xs, _) => {
                    pending.push(Next::Items(xs, 0))
                }
                V::Dense(xs) => pending.push(Next::Dense(xs, 0)),
                V::Record(fs) => pending.push(Next::Record(fs, 0)),
                V::Dict(entries) => pending.push(Next::Dict(entries, 0)),
                V::Optional(Some(item)) | V::Outcome(Ok(item)) => {
                    pending.push(Next::Value((*item).clone()))
                }
                V::Outcome(Err(f)) => pending.push(Next::Value((*f.payload).clone())),
                V::Variant(v) => pending.push(Next::Items(v.fields.clone(), 0)),
                _ => {}
            },
            Next::Items(xs, i) if i < xs.len() => {
                let value = xs[i].clone();
                pending.push(Next::Items(xs, i + 1));
                pending.push(Next::Value(value));
            }
            Next::Dense(xs, i) if i < xs.len() => {
                let value = xs.value(i);
                pending.push(Next::Dense(xs, i + 1));
                pending.push(Next::Value(value));
            }
            Next::Record(fs, i) if i < fs.len() => {
                let value = fs[i].1.clone();
                pending.push(Next::Record(fs, i + 1));
                pending.push(Next::Value(value));
            }
            Next::Dict(entries, i) if i < entries.len() => {
                let (key, value) = entries[i].clone();
                pending.push(Next::Dict(entries, i + 1));
                pending.push(Next::Value(value));
                pending.push(Next::Value(key));
            }
            _ => {}
        }
    }
    Ok(value)
}

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
    if x.bits() > max_bits() {
        Err(RESOURCE)
    } else {
        Ok(())
    }
}
pub(super) fn parse_int(s: &str) -> NumericResult<BigInt> {
    if s.trim_start_matches(['-', '+'])
        .trim_start_matches('0')
        .len()
        > decimal_digit_limit()
    {
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
                if (largest - 1).saturating_mul(u64::from(exp)) >= max_bits() {
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

/// A wide exact radicand need not itself fit binary64. Round its square root
/// from integer bounds so neither intermediate overflow nor cancellation in a
/// lossy converted radicand determines the result.
pub(super) fn sqrt(x: &BigRational) -> NumericResult<f64> {
    if x.is_negative() {
        return Err("square root of a negative number");
    }
    if x.is_zero() {
        return Ok(0.0);
    }
    let n = x.numer().magnitude();
    let d = x.denom().magnitude();
    let mut e = n.bits() as i64 - d.bits() as i64;
    // Only compare at nearby exponents. The most extreme estimates can be
    // classified before a shift, avoiding work proportional to input width.
    if e > 2048 {
        return Err("nonfinite real result");
    }
    if e < -2150 {
        return Ok(0.0);
    }
    if if e >= 0 {
        n < &(d << e as usize)
    } else {
        &(n << (-e) as usize) < d
    } {
        e -= 1;
    }
    let mut exponent = e.div_euclid(2);
    if exponent >= 1024 {
        return Err("nonfinite real result");
    }
    let normal = exponent >= -1022;
    let shift = 2 * if normal { 52 - exponent } else { 1074 };
    let rounded = |num: &BigUint, den: &BigUint| -> NumericResult<u64> {
        let q = (num / den).sqrt().to_u64().ok_or("nonfinite real result")?;
        let mid = BigUint::from(2 * q + 1);
        let cmp = (num << 2usize).cmp(&(&mid * &mid * den));
        Ok(q + u64::from(cmp.is_gt() || (cmp.is_eq() && q & 1 == 1)))
    };
    let mut significand = if shift >= 0 {
        rounded(&(n << shift as usize), d)?
    } else {
        rounded(n, &(d << (-shift) as usize))?
    };
    if !normal {
        // This also encodes the carry into the smallest normal correctly.
        return Ok(f64::from_bits(significand));
    }
    if significand == 1u64 << 53 {
        significand >>= 1;
        exponent += 1;
    }
    if exponent >= 1024 {
        return Err("nonfinite real result");
    }
    Ok(f64::from_bits(
        (((exponent + 1023) as u64) << 52) | (significand & ((1u64 << 52) - 1)),
    ))
}
