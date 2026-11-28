//! Plan only checked, strict pointwise chains whose cell kernels are total.
//! Unsupported shapes, storage or budget replay the ordinary checked stages.
mod execution;
mod ir;
mod lowering;
mod validation;

#[cfg(test)]
mod tests;

use crate::{
    hir::{CoreOp, ExprId},
    hir_eval::{resources, Eval, R, V},
};

/// Cheap syntactic guard for the expression dispatcher; not a proof of eligibility.
#[inline]
pub(in crate::hir_eval) fn can_start(op: CoreOp) -> bool {
    ir::Kernel::supports(op)
}

/// Entered after the enclosing evaluator charges and admits the root expression.
pub(in crate::hir_eval) fn try_eval(eval: &mut Eval<'_>, root: ExprId, env: usize) -> Option<R<V>> {
    if !resources::optimized() {
        return None;
    }
    let remaining = usize::from(eval.max_depth.saturating_sub(eval.depth));
    let candidate = lowering::build(eval.p, root, remaining)?;
    let plan = validation::CheckedPlan::validate(eval.p, candidate, remaining)?;
    Some(plan.run(eval, env))
}
