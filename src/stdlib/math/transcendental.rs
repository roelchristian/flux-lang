//! Binary64 kernels with explicit finite-result and domain contracts.
use super::super::{finite, wide, CoreOp, NumericResult, Type, DIV_BY_ZERO, V};
use crate::hir_eval::numeric::TotalRealUnary;
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed};
use std::f64::consts::{E, PI, TAU};

fn root(degree: &V, x: f64) -> NumericResult<f64> {
    let n = wide::integral(degree)?;
    if !n.is_positive() {
        return Err("math.root degree must be positive");
    }
    let odd = n.is_odd();
    if x < 0.0 && !odd {
        return Err("math.root of a negative number needs an odd degree");
    }
    if x == 0.0 {
        return Ok(if odd { x } else { 0.0 });
    }
    if n.is_one() {
        return Ok(x);
    }
    if n == BigInt::from(2) {
        return Ok(libm::sqrt(x));
    }
    if n == BigInt::from(3) {
        return Ok(libm::cbrt(x));
    }
    // Compute 1/n directly: an arbitrary-precision degree itself might not
    // be representable as a finite floating-point value.
    let reciprocal = wide::to_real(&BigRational::new_raw(BigInt::one(), n))?;
    Ok(libm::pow(x.abs(), reciprocal).copysign(x))
}

fn unary(op: CoreOp, x: f64) -> NumericResult<f64> {
    use CoreOp::*;
    if let Some(op) = TotalRealUnary::from_core(op) {
        return Ok(op.apply(x));
    }
    Ok(match op {
        MathExp => libm::exp(x),
        MathExpm1 => libm::expm1(x),
        MathLn | MathLog10 if x <= 0.0 => return Err("logarithm requires a positive argument"),
        MathLn => libm::log(x),
        MathLog10 => libm::log10(x),
        MathLog1p if x <= -1.0 => return Err("math.log1p requires an argument greater than -1"),
        MathLog1p => libm::log1p(x),
        MathTan => libm::tan(x),
        MathAsin | MathAcos if x.abs() > 1.0 => {
            return Err("inverse sine and cosine require an argument in [-1, 1]");
        }
        MathAsin => libm::asin(x),
        MathAcos => libm::acos(x),
        MathSinh => libm::sinh(x),
        MathCosh => libm::cosh(x),
        MathAsinh => libm::asinh(x),
        MathAcosh if x < 1.0 => return Err("math.acosh requires an argument at least 1"),
        MathAcosh => libm::acosh(x),
        MathAtanh if x.abs() >= 1.0 => return Err("math.atanh requires an argument in (-1, 1)"),
        MathAtanh => libm::atanh(x),
        MathRad => x * (PI / 180.0),
        MathDeg => x * (180.0 / PI),
        _ => return Err("unknown checked unary transcendental operation"),
    })
}

fn binary(op: CoreOp, x: f64, y: f64) -> NumericResult<f64> {
    use CoreOp::*;
    Ok(match op {
        MathLog if x <= 0.0 || x == 1.0 || y <= 0.0 => {
            return Err("math.log requires a positive base other than 1 and a positive argument");
        }
        MathLog => libm::log(y) / libm::log(x),
        MathPow if x == 0.0 && y < 0.0 => return Err(DIV_BY_ZERO),
        MathPow if x < 0.0 && y.fract() != 0.0 => {
            return Err("math.pow of a negative base needs an integral exponent");
        }
        MathPow => libm::pow(x, y),
        MathHypot => libm::hypot(x, y),
        MathAtan2 if x == 0.0 && y == 0.0 => return Err("math.atan2 is undefined at the origin"),
        MathAtan2 => libm::atan2(x, y),
        _ => return Err("unknown checked binary transcendental operation"),
    })
}

pub(super) fn apply(op: CoreOp, args: &[V], _ty: &Type) -> NumericResult<V> {
    use CoreOp::*;
    let result = match (op, args) {
        (MathPi, []) => PI,
        (MathE, []) => E,
        (MathTau, []) => TAU,
        (MathRoot, [degree, V::Real(x)]) => root(degree, *x)?,
        (op, [V::Real(x)]) => unary(op, *x)?,
        (op, [V::Real(x), V::Real(y)]) => binary(op, *x, *y)?,
        _ => return Err("invalid checked transcendental operands"),
    };
    finite(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Prim;

    #[test]
    fn stable_small_arguments_large_magnitudes_and_huge_degree() {
        let r = Type::Prim(Prim::R);
        for op in [CoreOp::MathExpm1, CoreOp::MathLog1p] {
            let v = apply(op, &[V::Real(1e-20)], &r).unwrap();
            assert!(matches!(v, V::Real(small) if small == 1e-20));
        }
        let v = apply(CoreOp::MathHypot, &[V::Real(3e200), V::Real(4e200)], &r).unwrap();
        assert!(matches!(v, V::Real(h) if (h / 5e200 - 1.0).abs() <= 2.0 * f64::EPSILON));
        let n = (BigInt::one() << 20_000_usize) + BigInt::one();
        let n = wide::ratio_value(
            BigRational::from_integer(n),
            super::super::super::Domain::Natural,
        )
        .unwrap();
        assert_eq!(root(&n, -10.0).unwrap(), -1.0);
        assert_eq!(root(&n, 0.0).unwrap(), 0.0);
    }
}
