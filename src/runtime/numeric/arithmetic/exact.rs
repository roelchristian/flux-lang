//! Inline exact values, checked arithmetic and conversions to wider domains.
use super::NumericResult;
use crate::{
    hir::CoreOp,
    hir_eval::{decimal, wide, V},
    types::{Prim, Type},
};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};
use std::cmp::Ordering;

pub(in crate::hir_eval) const OVERFLOW: &str =
    "exact arithmetic exceeds the prototype's i128 bound";
pub(in crate::hir_eval) const DIV_BY_ZERO: &str = "division by zero";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::hir_eval) enum Domain {
    Natural,
    Integer,
    Rational,
    Decimal,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::hir_eval) struct Exact {
    pub(in crate::hir_eval) n: i128,
    pub(in crate::hir_eval) d: i128,
    pub(in crate::hir_eval) domain: Domain,
}
impl Exact {
    pub(in crate::hir_eval) fn integer(n: i128, domain: Domain) -> Self {
        Self { n, d: 1, domain }
    }
    pub(in crate::hir_eval) fn new(n: i128, d: i128, domain: Domain) -> NumericResult<Self> {
        if d == 0 {
            return Err(DIV_BY_ZERO);
        }
        if n == 0 {
            return Ok(Self::integer(0, domain));
        }
        let g = gcd(n.unsigned_abs(), d.unsigned_abs());
        let (n, d) = if g == 1_u128 << 127 {
            (1, 1)
        } else {
            (n / g as i128, d / g as i128)
        };
        let (n, d) = if d < 0 {
            (
                n.checked_neg().ok_or(OVERFLOW)?,
                d.checked_neg().ok_or(OVERFLOW)?,
            )
        } else {
            (n, d)
        };
        Ok(Self { n, d, domain })
    }
    pub(in crate::hir_eval) fn integral(self) -> NumericResult<i128> {
        if self.d == 1 {
            Ok(self.n)
        } else {
            Err("expected an integral value")
        }
    }
    /// Round the exact quotient once, to nearest binary64, ties to even.
    fn real(self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        let n = self.n.unsigned_abs();
        let d = self.d as u128;
        let mut exponent = d.leading_zeros() as i32 - n.leading_zeros() as i32;
        if if exponent >= 0 {
            n < (d << exponent)
        } else {
            (n << -exponent) < d
        } {
            exponent -= 1;
        }
        let (scaled_n, scaled_d) = if exponent >= 0 {
            (n, d << exponent)
        } else {
            (n << -exponent, d)
        };
        let mut rem = scaled_n - scaled_d;
        let mut significand = 1u64;
        for _ in 0..52 {
            significand <<= 1;
            let complement = scaled_d - rem;
            if rem >= complement {
                significand |= 1;
                rem -= complement;
            } else {
                rem *= 2;
            }
        }
        let complement = scaled_d - rem;
        if rem > complement || (rem == complement && significand & 1 == 1) {
            significand += 1;
            if significand == 1u64 << 53 {
                significand >>= 1;
                exponent += 1;
            }
        }
        let sign = u64::from(self.n < 0) << 63;
        let exponent = ((exponent + 1023) as u64) << 52;
        f64::from_bits(sign | exponent | (significand & ((1u64 << 52) - 1)))
    }

    /// Convert the *stored* binary64 value to a reduced, bounded exact fraction.
    fn from_real(value: f64) -> NumericResult<Self> {
        if !value.is_finite() {
            return Err("nonfinite real value");
        }
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32;
        let mantissa = bits & ((1u64 << 52) - 1);
        let (mut m, mut shift) = if exponent == 0 {
            (mantissa, -1074)
        } else {
            (mantissa | (1u64 << 52), exponent - 1075)
        };
        if m == 0 {
            return Ok(Self::integer(0, Domain::Rational));
        }
        if shift < 0 {
            let cancel = (m.trailing_zeros() as i32).min(-shift);
            m >>= cancel;
            shift += cancel;
        }
        let (magnitude, denominator) = if shift < 0 {
            if -shift >= 127 {
                return Err(OVERFLOW);
            }
            (u128::from(m), 1i128 << -shift)
        } else {
            let magnitude = u128::from(m);
            if shift >= 128 || magnitude > (u128::MAX >> shift) {
                return Err(OVERFLOW);
            }
            (magnitude << shift, 1)
        };
        let signed = if bits >> 63 != 0 {
            if magnitude == 1u128 << 127 {
                i128::MIN
            } else {
                -i128::try_from(magnitude).map_err(|_| OVERFLOW)?
            }
        } else {
            i128::try_from(magnitude).map_err(|_| OVERFLOW)?
        };
        Self::new(signed, denominator, Domain::Rational)
    }
    pub(in crate::hir_eval) fn as_domain(self, domain: Domain) -> NumericResult<Exact> {
        if domain == Domain::Natural && (self.n < 0 || self.d != 1) {
            return Err("cannot represent value as n");
        }
        if domain == Domain::Integer && self.d != 1 {
            return Err("cannot represent value as z");
        }
        Ok(Self { domain, ..self })
    }
    fn compute(self, op: CoreOp, b: Self, domain: Domain) -> NumericResult<Self> {
        use CoreOp::*;
        use Domain::*;
        let mul = |a: i128, b: i128| a.checked_mul(b).ok_or(OVERFLOW);
        match op {
            Add | Sub => {
                let g = gcd(self.d as u128, b.d as u128) as i128;
                let a = mul(self.n, b.d / g)?;
                let c = mul(b.n, self.d / g)?;
                let n = if op == Add {
                    a.checked_add(c)
                } else {
                    a.checked_sub(c)
                }
                .ok_or(OVERFLOW)?;
                if domain == Natural && n < 0 {
                    return Err("natural subtraction underflow");
                }
                Self::new(n, mul(self.d / g, b.d)?, domain)
            }
            Mul => {
                let g = gcd(self.n.unsigned_abs(), b.d as u128) as i128;
                let h = gcd(b.n.unsigned_abs(), self.d as u128) as i128;
                Self::new(mul(self.n / g, b.n / h)?, mul(self.d / h, b.d / g)?, domain)
            }
            Div => {
                if b.n == 0 {
                    return Err(DIV_BY_ZERO);
                }
                if self.n == 0 {
                    return Ok(Self::integer(0, Rational));
                }
                let g = gcd(self.n.unsigned_abs(), b.n.unsigned_abs());
                let (a, c) = if g == 1u128 << 127 {
                    (1, 1)
                } else {
                    let g = g as i128;
                    (self.n / g, b.n / g)
                };
                let h = gcd(self.d as u128, b.d as u128) as i128;
                Self::new(mul(a, b.d / h)?, mul(self.d / h, c)?, Rational)
            }
            Quotient | Remainder => {
                let a = self.integral()?;
                let b = b.integral()?;
                if b == 0 {
                    return Err(DIV_BY_ZERO);
                }
                let q = a.checked_div(b).ok_or(OVERFLOW)?;
                let r = a.checked_rem(b).ok_or(OVERFLOW)?;
                let opposite = r != 0 && (r < 0) != (b < 0);
                let n = match (op, opposite) {
                    (Quotient, true) => q.checked_sub(1).ok_or(OVERFLOW)?,
                    (Quotient, false) => q,
                    (_, true) => r.checked_add(b).ok_or(OVERFLOW)?,
                    (_, false) => r,
                };
                Ok(Self::integer(n, domain))
            }
            Pow => {
                let exp = b.integral()?;
                if exp < 0 || b.domain != Natural {
                    return Err("power requires a natural exponent");
                }
                let mut a = self;
                let mut out = Self::integer(1, domain);
                let mut e = exp as u128;
                while e != 0 {
                    if e & 1 == 1 {
                        out = out.compute(Mul, a, domain)?;
                    }
                    e >>= 1;
                    if e != 0 {
                        a = a.compute(Mul, a, domain)?;
                    }
                }
                Ok(out)
            }
            _ => Err("invalid exact operation in typed HIR"),
        }
    }
    pub(in crate::hir_eval) fn order(self, b: Self) -> Ordering {
        fn positive(mut an: u128, mut ad: u128, mut bn: u128, mut bd: u128) -> Ordering {
            let mut reverse = false;
            loop {
                let mut c = (an / ad).cmp(&(bn / bd));
                let (ar, br) = (an % ad, bn % bd);
                if c == Ordering::Equal && (ar == 0 || br == 0) {
                    c = ar.cmp(&br)
                }
                if c != Ordering::Equal || ar == 0 {
                    return if reverse { c.reverse() } else { c };
                }
                (an, ad, bn, bd) = (ad, ar, bd, br);
                reverse = !reverse;
            }
        }
        match (self.n < 0, b.n < 0) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (negative, _) => {
                let c = positive(
                    self.n.unsigned_abs(),
                    self.d as u128,
                    b.n.unsigned_abs(),
                    b.d as u128,
                );
                if negative {
                    c.reverse()
                } else {
                    c
                }
            }
        }
    }
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b)
    }
    a
}
pub(in crate::hir_eval) fn domain(ty: &Type) -> Option<Domain> {
    match ty {
        Type::Prim(Prim::N) => Some(Domain::Natural),
        Type::Prim(Prim::Z) => Some(Domain::Integer),
        Type::Prim(Prim::Q) => Some(Domain::Rational),
        Type::Prim(Prim::D) => Some(Domain::Decimal),
        Type::Seq(inner) => domain(inner),
        _ => None,
    }
}
fn parse_inline_exact(s: &str, ty: &Type) -> NumericResult<Exact> {
    let (raw, chosen) = match s.as_bytes().last() {
        Some(b'n') => (&s[..s.len() - 1], Domain::Natural),
        Some(b'z') => (&s[..s.len() - 1], Domain::Integer),
        Some(b'q') => (&s[..s.len() - 1], Domain::Rational),
        _ => (
            s,
            domain(ty).unwrap_or(if s.contains('.') {
                Domain::Rational
            } else {
                Domain::Integer
            }),
        ),
    };
    let parse = |s: &str| s.parse::<i128>().map_err(|_| OVERFLOW);
    let value = if let Some((left, right)) = raw.split_once('.') {
        if chosen != Domain::Rational {
            return Err("fractional literal requires q in this backend");
        }
        let r = right.trim_end_matches('0');
        if r.is_empty() {
            Exact::integer(parse(left)?, chosen)
        } else {
            let d = 10_i128
                .checked_pow(r.len().try_into().map_err(|_| OVERFLOW)?)
                .ok_or(OVERFLOW)?;
            Exact::new(parse(&format!("{left}{r}"))?, d, chosen)?
        }
    } else {
        Exact::integer(parse(raw)?, chosen)
    };
    value.as_domain(chosen)
}

pub(in crate::hir_eval) fn exact_i128(n: i128, d: Domain) -> V {
    V::Exact(Exact::integer(n, d))
}

pub(in crate::hir_eval) fn rational(n: BigInt, d: BigInt) -> NumericResult<V> {
    if d.is_zero() {
        return Err(DIV_BY_ZERO);
    }
    wide::ratio_value(BigRational::new(n, d), Domain::Rational)
}
pub(in crate::hir_eval) fn as_ratio(v: &V) -> NumericResult<(BigInt, BigInt)> {
    Ok(wide::ratio(v)?.into_raw())
}
pub(in crate::hir_eval) fn parse_exact_value(raw: &str, d: Domain) -> NumericResult<V> {
    if d == Domain::Decimal {
        return decimal::decimal_literal(raw);
    }
    let ty = Type::Prim(match d {
        Domain::Natural => Prim::N,
        Domain::Integer => Prim::Z,
        _ => Prim::Q,
    });
    match parse_inline_exact(raw, &ty) {
        Ok(n) => return wide::guard_inline(n).map(V::Exact),
        Err(e) if e != OVERFLOW => return Err(e),
        Err(_) => {}
    }
    let value = if let Some((a, b)) = raw.split_once('.') {
        if d != Domain::Rational {
            return Err("fractional literal requires q");
        }
        let b = b.trim_end_matches('0');
        let decimal_digits = u32::try_from(b.len()).map_err(|_| wide::RESOURCE)?;
        if decimal_digits as usize > wide::decimal_digit_limit() {
            return Err(wide::RESOURCE);
        }
        let num = wide::parse_int(&format!("{a}{b}"))?;
        let den = BigInt::from(10u8).pow(decimal_digits);
        BigRational::new(num, den)
    } else {
        BigRational::from_integer(wide::parse_int(raw)?)
    };
    wide::ratio_value(value, d)
}
pub(in crate::hir_eval) fn parse_exact(s: &str, ty: &Type) -> NumericResult<V> {
    let (raw, d) = match s.as_bytes().last() {
        Some(b'n') => (&s[..s.len() - 1], Domain::Natural),
        Some(b'z') => (&s[..s.len() - 1], Domain::Integer),
        Some(b'q') => (&s[..s.len() - 1], Domain::Rational),
        Some(b'd') => (&s[..s.len() - 1], Domain::Decimal),
        _ => (
            s,
            domain(ty).ok_or("numeric literal has no checked domain")?,
        ),
    };
    parse_exact_value(raw, d)
}

pub(in crate::hir_eval) fn exact_binary(op: CoreOp, a: &V, b: &V, d: Domain) -> NumericResult<V> {
    if d == Domain::Decimal {
        return decimal::decimal_binary(op, a, b);
    }
    if let (V::Exact(a), V::Exact(b)) = (a, b) {
        match a.compute(op, *b, d) {
            Ok(v) => return wide::guard_inline(v).map(V::Exact),
            Err(e) if e != OVERFLOW => return Err(e),
            Err(_) => {}
        }
    }
    wide::binary(op, a, b, d)
}
pub(in crate::hir_eval) fn real_of(v: &V) -> NumericResult<f64> {
    match v {
        V::Real(x) => Ok(*x),
        V::Exact(x) => Ok(x.real()),
        v => wide::to_real(&wide::ratio(v)?),
    }
}
pub(in crate::hir_eval) fn to_exact(v: &V) -> NumericResult<V> {
    match v {
        V::Real(x) => match Exact::from_real(*x) {
            Ok(v) => wide::guard_inline(v).map(V::Exact),
            Err(e) if e != OVERFLOW => Err(e),
            Err(_) => wide::ratio_value(wide::from_real(*x)?, Domain::Rational),
        },
        _ => Ok(v.clone()),
    }
}
pub(in crate::hir_eval) fn exact_index(v: &V) -> NumericResult<usize> {
    if let V::Exact(n) = v {
        let n = n.integral()?;
        if n < 0 {
            return Err("expected a nonnegative integer");
        }
        return Ok(usize::try_from(n).unwrap_or(usize::MAX));
    }
    let n = wide::integral(v)?;
    if n.is_negative() {
        return Err("expected a nonnegative integer");
    }
    Ok(n.to_usize().unwrap_or(usize::MAX))
}
pub(in crate::hir_eval) fn exact_unary(op: CoreOp, v: &V, d: Domain) -> NumericResult<V> {
    if d == Domain::Decimal {
        return decimal::decimal_unary(op, v);
    }
    if let V::Exact(n) = v {
        let num = if op == CoreOp::Abs {
            n.n.checked_abs()
        } else {
            n.n.checked_neg()
        };
        if let Some(i) = num {
            return wide::guard_inline(Exact {
                n: i,
                d: n.d,
                domain: d,
            })
            .map(V::Exact);
        }
    }
    let v = wide::ratio(v)?;
    wide::ratio_value(if op == CoreOp::Abs { v.abs() } else { -v }, d)
}
