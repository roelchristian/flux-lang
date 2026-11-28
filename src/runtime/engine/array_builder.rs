//! Bounded, non-escaping range construction in the ordinary checked evaluator.
use super::{primitives::WitnessedOp, Eval};
use crate::{
    core::operation::Operation,
    hir::{CoreOp, ExprId, ExprKind, Target},
    hir_eval::{
        array::builder::{AdmittedRange, PackedRange},
        error, resources, wide, Domain, R, V,
    },
    types::{Prim, Type},
};

enum RangeOutput {
    Packed(AdmittedRange),
    Sequence(V),
}

pub(super) fn try_form_range(
    eval: &mut Eval<'_>,
    form_id: ExprId,
    env: usize,
    operands: &[ExprId],
) -> Option<R<V>> {
    if !resources::optimized() {
        return None;
    }
    let [dims, range_id] = *operands else {
        return None;
    };
    let p = eval.p;
    let form = p.node(form_id);
    let range = p.node(range_id);
    let ExprKind::Call(Target::Builtin(CoreOp::Range), args) = &range.kind else {
        return None;
    };
    let [lo, hi] = *args.as_slice() else {
        return None;
    };
    let Type::Seq(element) = &range.ty else {
        return None;
    };
    // Only the concrete checked domains and result shape are admitted.
    // Generic expressions, references, and escaping ranges keep the
    // ordinary evaluation path.
    if form.ty.array_element() != Some(element.as_ref())
        || range.lift_mode.is_some()
        || matches!(&range.evidence, Some(Operation::Numeric { result, .. }) if result != &range.ty)
    {
        return None;
    }
    let domain = match element.as_ref() {
        Type::Prim(Prim::Z) => Domain::Integer,
        Type::Prim(Prim::N) => Domain::Natural,
        _ => return None,
    };
    Some((|| {
        // Evaluate the shape expression, but defer its validation until
        // after the range operand, just as in ordinary core dispatch.
        let shape = eval.eval(dims, env)?;
        let source = eval.with_expression(range.span, |eval| {
            let lo = eval.eval(lo, env)?;
            let hi = eval.eval(hi, env)?;
            if let Some(packed) = PackedRange::from_values(&lo, &hi, domain) {
                eval.tick(range.span)?; // Range core dispatch
                let len = packed
                    .checked_len()
                    .map_err(|reason| error(range.span, reason))?;
                eval.charge_work(len, range.span)?;
                // Both integral endpoints have already passed their
                // scalar guards; each interior magnitude is bounded by
                // the larger endpoint and has the same unit denominator.
                packed.admit(len, range.span).map(RangeOutput::Packed)
            } else {
                // Exact widening is not a reason to re-evaluate (and
                // possibly repeat effects in) either endpoint.
                eval.core(
                    CoreOp::Range,
                    vec![lo, hi],
                    &range.ty,
                    None,
                    None,
                    range.span,
                )
                .map(RangeOutput::Sequence)
            }
        })?;
        match source {
            RangeOutput::Packed(packed) => {
                eval.tick(form.span)?; // witnessed dispatch
                eval.tick(form.span)?; // array dispatch
                packed.form(&shape, form.span)
            }
            RangeOutput::Sequence(seq) => {
                let seq = wide::guard_embedded(seq).map_err(|reason| error(range.span, reason))?;
                eval.witnessed_core(
                    WitnessedOp::Dense(CoreOp::ArrayForm),
                    vec![shape, seq],
                    &[eval.type_at(&p.node(dims).ty, env), range.ty.clone()],
                    &form.ty,
                    form.table_schema.as_ref(),
                    form.span,
                )
            }
        }
    })())
}
