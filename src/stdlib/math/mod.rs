//! Mathesis kernels reached through checked HIR operations and the ordinary lift plan.
use super::{
    as_seq, error, exact_i128, inner_type, invariant, lift, lift_one, numeric_identity, resources,
    scalar_binary, wide, BigRational, CoreOp, Domain, Eval, Integer, LiftMode, NumericResult, Prim,
    Rc, Span, ToPrimitive, Type, Wide, Zero, COLLECTION_LIMIT, R, V,
};
mod elementary;
pub(super) mod linear;
mod number_theory;
mod numerical;
mod polynomial;
mod special;
mod transcendental;

pub(super) fn apply(
    op: CoreOp,
    args: &[V],
    ty: &Type,
    mode: Option<LiftMode>,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    use CoreOp::*;
    let kernel: fn(CoreOp, &[V], &Type) -> NumericResult<V> = match op {
        MathFloor | MathCeil | MathTrunc | MathRound | MathSign | MathFrac | MathRecip => {
            elementary::apply
        }
        MathExp | MathExpm1 | MathLn | MathLog10 | MathLog1p | MathSin | MathCos | MathTan
        | MathAsin | MathAcos | MathAtan | MathSinh | MathCosh | MathTanh | MathAsinh
        | MathAcosh | MathAtanh | MathRad | MathDeg | MathLog | MathPow | MathHypot | MathAtan2
        | MathRoot | MathPi | MathE | MathTau => transcendental::apply,
        MathGcd | MathLcm | MathFact | MathChoose | MathPerm | MathModPow | MathIsqrt
        | MathPrime | MathFactor | MathDivisors | MathFib => number_theory::apply,
        MathGamma | MathLgamma | MathErf | MathErfc | MathBeta => special::apply,
        PolyEval | PolyAdd | PolySub | PolyMul | PolyDeriv | PolyInteg | PolyDegree => {
            polynomial::apply
        }
        NumBisect | NumNewton | NumIntegrate | NumInterp | NumDiff => {
            return numerical::apply(op, args, eval, span)
        }

        _ => return Err(invariant(span, "unknown Mathesis operation")),
    };
    let scalar_type = if mode.is_some() { inner_type(ty) } else { ty };
    let allowance = (mode.is_some_and(|shape| shape != LiftMode::Scalar)
        && matches!(
            op,
            MathGcd | MathLcm | MathFact | MathChoose | MathPerm | MathIsqrt | MathPrime | MathFib
        ))
    .then(number_theory::LiftBudget::new);
    let scalar = |args: &[V]| match &allowance {
        Some(budget) => number_theory::apply_budgeted(op, args, scalar_type, budget),
        None => kernel(op, args, scalar_type),
    };
    let result = match (mode, args) {
        (None, _) => kernel(op, args, ty),
        (Some(_), [a]) => lift_one(a.clone(), mode, &|a| scalar(&[a])),
        (Some(_), [a, b]) => lift(a.clone(), b.clone(), mode, &|a, b| scalar(&[a, b])),
        _ => return Err(invariant(span, "invalid Mathesis lift arity")),
    };
    result.map_err(|message| error(span, message))
}
