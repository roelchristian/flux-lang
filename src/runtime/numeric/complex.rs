//! Finite binary64 complex numbers. Products and quotients have a single
//! rounding per component, without intermediate overflow or loss to underflow.
use super::wide;
use crate::{
    hir::CoreOp,
    hir_eval::{finite, NumericResult, DIV_BY_ZERO, V},
};
use num_traits::Zero;

fn pair(value: V) -> NumericResult<[f64; 2]> {
    match value {
        V::Complex(value) => Ok(value),
        _ => Err("expected a checked complex number"),
    }
}

fn checked(re: f64, im: f64) -> NumericResult<V> {
    if re.is_finite() && im.is_finite() {
        Ok(V::Complex([re, im]))
    } else {
        Err("nonfinite complex result")
    }
}

pub(in crate::hir_eval) fn binary(op: CoreOp, a: V, b: V) -> NumericResult<V> {
    let [ar, ai] = pair(a)?;
    let [br, bi] = pair(b)?;
    match op {
        CoreOp::Add => checked(ar + br, ai + bi),
        CoreOp::Sub => checked(ar - br, ai - bi),
        CoreOp::Mul | CoreOp::Div => {
            // Conversion here is an internal exact view of the finite inputs.
            // Each component has at most binary64's fixed exponent/mantissa
            // width. The sizes of all temporaries are bounded independently
            // of any user exact atom or decimal scale.
            let (ar, ai, br, bi) = (
                wide::from_real(ar)?,
                wide::from_real(ai)?,
                wide::from_real(br)?,
                wide::from_real(bi)?,
            );
            let (re, im) = if op == CoreOp::Mul {
                (&ar * &br - &ai * &bi, &ar * &bi + &ai * &br)
            } else {
                let denom = &br * &br + &bi * &bi;
                if denom.is_zero() {
                    return Err(DIV_BY_ZERO);
                }
                (
                    (&ar * &br + &ai * &bi) / &denom,
                    (&ai * &br - &ar * &bi) / denom,
                )
            };
            checked(
                wide::to_real(&re).map_err(|_| "nonfinite complex result")?,
                wide::to_real(&im).map_err(|_| "nonfinite complex result")?,
            )
        }
        _ => Err("invalid checked complex arithmetic operation"),
    }
}

pub(in crate::hir_eval) fn apply(op: CoreOp, args: &[V]) -> NumericResult<V> {
    use CoreOp::*;
    match (op, args) {
        (CxMake, [V::Real(re), V::Real(im)]) => checked(*re, *im),
        (CxRe, [V::Complex([re, _])]) => finite(*re),
        (CxIm, [V::Complex([_, im])]) => finite(*im),
        (CxConj, [V::Complex([re, im])]) => checked(*re, -*im),
        (CxAbs, [V::Complex([re, im])]) => finite(libm::hypot(*re, *im)),
        (CxArg, [V::Complex([re, im])]) => finite(libm::atan2(*im, *re)),
        _ => Err("invalid checked complex primitive operands"),
    }
}
