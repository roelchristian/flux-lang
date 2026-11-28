//! Replay entry/completion accounting, then materialize at most one buffer.
use super::{ir::Kernel, validation::CheckedPlan};
use crate::{
    hir_eval::{array, error, numeric::TotalRealUnary, packed, resources, Eval, R, V},
    types::Prim,
};
use array::Dense;

fn at_depth<T>(eval: &mut Eval<'_>, depth: u16, run: impl FnOnce(&mut Eval<'_>) -> R<T>) -> R<T> {
    let saved = eval.depth;
    eval.depth = depth;
    let result = run(eval);
    eval.depth = saved;
    result
}

fn has_headroom(input: &Dense) -> bool {
    // The checked stages have one shape and no size-growing cells. While
    // the evaluated leaf is live, allow an intermediate and both generic
    // and native scratch/output buffers. Otherwise do ordinary stage work.
    array::checked_volume(input.shape()) == Ok(input.len())
        && input
            .len()
            .checked_mul(3 * std::mem::size_of::<V>())
            .is_some_and(|bytes| bytes <= resources::available_memory())
}

impl CheckedPlan {
    pub(super) fn run(self, eval: &mut Eval<'_>, env: usize) -> R<V> {
        let p = eval.p;
        let base_depth = eval.depth;
        // The caller has already charged the root expression. Enter each
        // dependent expression before the leaf in the original order.
        for stage in &self.stages()[1..] {
            eval.tick(p.node(stage.id).span)?;
        }
        let mut value = at_depth(
            eval,
            base_depth + (self.stages().len() - 1) as u16,
            |eval| eval.eval(self.leaf(), env),
        )?;

        if let V::Dense(input) = &value {
            if has_headroom(input) {
                match self.element() {
                    Prim::R => {
                        if let Some(cells) = packed::real_cells(input) {
                            self.charge_stages(eval, env, input.len())?;
                            return self.materialize_real(eval, input, cells);
                        }
                    }
                    Prim::B => {
                        if let Some(cells) = packed::bool_cells(input) {
                            self.charge_stages(eval, env, input.len())?;
                            return self.materialize_bool(eval, input, cells);
                        }
                    }
                    _ => unreachable!("validated pointwise element type"),
                }
            }
        }

        // A tight budget or unsupported representation must not re-evaluate
        // the leaf. Replay the original checked core nodes at their depths.
        // Concrete r/b cells need no post-core exact-size or specialization
        // checks for intermediates; the ordinary evaluator still guards the
        // root when this returns.
        for (index, stage) in self.stages().iter().enumerate().rev() {
            let node = p.node(stage.id);
            value = at_depth(eval, base_depth + index as u16, |eval| {
                let mut args = vec![value];
                if let Some(rhs) = stage.kernel.literal_operand() {
                    args.push(eval.eval(rhs, env)?);
                }
                eval.core(stage.op, args, &node.ty, None, node.lift_mode, node.span)
            })?;
        }
        Ok(value)
    }

    fn charge_stages(&self, eval: &mut Eval<'_>, env: usize, len: usize) -> R<()> {
        let base_depth = eval.depth;
        for (index, stage) in self.stages().iter().enumerate().rev() {
            if let Some(rhs) = stage.kernel.literal_operand() {
                // This pure scalar operand still has normal eval tick and
                // depth admission after the array operand, before its core.
                at_depth(eval, base_depth + index as u16, |eval| eval.eval(rhs, env))?;
            }
            let span = eval.p.node(stage.id).span;
            eval.tick(span)?;
            eval.charge_work(len, span)?;
        }
        Ok(())
    }

    fn materialize_real(
        &self,
        eval: &Eval<'_>,
        input: &Dense,
        cells: impl ExactSizeIterator<Item = f64>,
    ) -> R<V> {
        let span = eval.p.node(self.root()).span;
        let lease = resources::reserve_for::<f64>(cells.len()).map_err(|e| error(span, e))?;
        let out = if self
            .stages()
            .iter()
            .all(|stage| stage.kernel == Kernel::Real(TotalRealUnary::Negate))
        {
            // Preserve the cheap signed-zero-safe original negation parity.
            match (self.stages().len() % 2, input.real_slice()) {
                (0, Some(slice)) => slice.to_vec(),
                (0, None) => cells.collect(),
                (_, Some(slice)) => slice.iter().map(|cell| -cell).collect(),
                (_, None) => cells.map(|cell| -cell).collect(),
            }
        } else {
            cells.map(|cell| self.real(cell)).collect()
        };
        Dense::pack_real_reserved(input.shape().into(), out, lease, span)
    }

    fn materialize_bool(
        &self,
        eval: &Eval<'_>,
        input: &Dense,
        cells: impl ExactSizeIterator<Item = bool>,
    ) -> R<V> {
        let span = eval.p.node(self.root()).span;
        let lease = resources::reserve_for::<bool>(cells.len()).map_err(|e| error(span, e))?;
        let when_false = self.boolean(false);
        let when_true = self.boolean(true);
        let out = match (when_false, when_true, input.bool_slice()) {
            (false, true, Some(slice)) => slice.to_vec(),
            (false, true, None) => cells.collect(),
            (true, false, Some(slice)) => slice.iter().map(|cell| !cell).collect(),
            (true, false, None) => cells.map(|cell| !cell).collect(),
            (constant, _, _) => vec![constant; cells.len()],
        };
        Dense::pack_bool_reserved(input.shape().into(), out, lease, span)
    }
}
