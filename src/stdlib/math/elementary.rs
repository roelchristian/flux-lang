//! Domain-preserving elementary scalar operations.
use super::super::{decimal, finite, wide, CoreOp, Domain, NumericResult, Type, DIV_BY_ZERO, V};
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};
use std::cmp::Ordering;

fn integer(n: BigInt) -> NumericResult<V> {
    wide::ratio_value(BigRational::from_integer(n), Domain::Integer)
}

fn rounded(op: CoreOp, x: &BigRational) -> BigInt {
    let (q, rem) = x.numer().div_rem(x.denom());
    match op {
        CoreOp::MathFloor if rem.is_negative() => q - BigInt::one(),
        CoreOp::MathCeil if rem.is_positive() => q + BigInt::one(),
        CoreOp::MathRound => {
            let cmp = (rem.abs() << 1_usize).cmp(x.denom());
            if cmp == Ordering::Greater || (cmp == Ordering::Equal && q.is_odd()) {
                q + rem.signum()
            } else {
                q
            }
        }
        _ => q,
    }
}

fn real(op: CoreOp, x: f64) -> NumericResult<V> {
    use CoreOp::*;
    match op {
        MathSign => {
            let n = i128::from(x > 0.0) - i128::from(x < 0.0);
            integer(BigInt::from(n))
        }
        MathFrac => finite(x.fract()),
        MathRecip if x == 0.0 => Err(DIV_BY_ZERO),
        MathRecip => finite(x.recip()),
        _ => {
            let r = match op {
                MathFloor => x.floor(),
                MathCeil => x.ceil(),
                MathTrunc => x.trunc(),
                MathRound => x.round_ties_even(),
                _ => return Err("unknown checked elementary operation"),
            };
            integer(wide::from_real(r)?.to_integer())
        }
    }
}

pub(super) fn apply(op: CoreOp, args: &[V], _ty: &Type) -> NumericResult<V> {
    use CoreOp::*;
    let [value] = args else {
        return Err("invalid checked elementary arity");
    };
    if let V::Real(x) = value {
        return real(op, *x);
    }
    let x = wide::ratio(value)?;
    match op {
        MathSign => integer(BigInt::from(match x.numer().sign() {
            Sign::Minus => -1,
            Sign::NoSign => 0,
            Sign::Plus => 1,
        })),
        MathRecip => {
            if x.is_zero() {
                Err(DIV_BY_ZERO)
            } else {
                wide::ratio_value(x.recip(), Domain::Rational)
            }
        }
        MathFrac => {
            let kind = wide::kind(value).ok_or("expected an exact scalar")?;
            if kind == Domain::Decimal {
                // Subtract a scale-zero integer, preserving the written scale.
                let truncated = integer(x.to_integer())?;
                decimal::decimal_binary(Sub, value, &truncated)
            } else {
                let (num, den) = x.into_raw();
                wide::ratio_value(BigRational::new(num % &den, den), kind)
            }
        }
        MathTrunc | MathFloor | MathCeil | MathRound => integer(rounded(op, &x)),
        _ => Err("unknown checked elementary operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Prim;

    #[test]
    fn rounding_half_even_and_unbounded_integer_results() {
        let z = Type::Prim(Prim::Z);
        for (n, expected) in [
            (-7, -4),
            (-5, -2),
            (-3, -2),
            (-1, 0),
            (1, 0),
            (3, 2),
            (5, 2),
            (7, 4),
        ] {
            let ratio = BigRational::new(BigInt::from(n), BigInt::from(2));
            let value = wide::ratio_value(ratio, Domain::Rational).unwrap();
            let exact = apply(CoreOp::MathRound, &[value], &z).unwrap();
            let approx = apply(CoreOp::MathRound, &[V::Real(f64::from(n) / 2.0)], &z).unwrap();
            assert_eq!(wide::integral(&exact).unwrap(), BigInt::from(expected));
            assert_eq!(wide::integral(&approx).unwrap(), BigInt::from(expected));
        }

        let rounded = apply(CoreOp::MathFloor, &[V::Real(2.0f64.powi(1000))], &z).unwrap();
        assert_eq!(wide::integral(&rounded).unwrap(), BigInt::one() << 1000);
        let zero = apply(CoreOp::MathSign, &[V::Real(-0.0)], &z).unwrap();
        assert!(wide::integral(&zero).unwrap().is_zero());
    }
}
