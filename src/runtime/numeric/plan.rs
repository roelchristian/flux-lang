//! Conservative fusion of checked, shape-preserving total unary operations.
use super::super::{
    array, error, resources, CoreOp, Eval, ExprId, ExprKind, HirProgram, LiftMode, Prim, Target,
    Type, R, V,
};
use array::Dense;

struct Plan {
    stages: Vec<ExprId>,
    leaf: ExprId,
    op: CoreOp,
}

fn stage(program: &HirProgram, id: ExprId) -> Option<(CoreOp, ExprId)> {
    let node = program.node(id);
    let ExprKind::Call(Target::Builtin(op), operands) = &node.kind else {
        return None;
    };
    let [child] = operands.as_slice() else {
        return None;
    };
    let element = match op {
        CoreOp::Negate => Prim::R,
        CoreOp::Not => Prim::B,
        _ => return None,
    };
    (node.lift_mode == Some(LiftMode::LeftArray)
        && node.ty.array_element() == Some(&Type::Prim(element))
        && program.node(*child).ty == node.ty)
        .then_some((*op, *child))
}

impl Plan {
    fn build(program: &HirProgram, root: ExprId, limit: usize) -> Option<Self> {
        let (op, mut leaf) = stage(program, root)?;
        if stage(program, leaf)?.0 != op || limit < 2 {
            return None;
        }
        let mut stages = vec![root];
        while let Some((next, child)) = stage(program, leaf) {
            if next != op {
                break;
            }
            if stages.len() == limit {
                // An over-depth chain must fail at the same source position
                // as a normal recursive walk, not at plan construction.
                return None;
            }
            stages.push(leaf);
            leaf = child;
        }
        Some(Self { stages, leaf, op })
    }

    fn run(self, eval: &mut Eval<'_>, env: usize) -> R<V> {
        let program = eval.p;
        let base_depth = eval.depth;
        // The caller already admitted and charged the outermost expression.
        // Charge the intervening expressions before evaluating the leaf, in
        // original entry order; its effects and failure still happen once.
        for &id in &self.stages[1..] {
            eval.tick(program.node(id).span)?;
        }
        eval.depth += (self.stages.len() - 1) as u16;
        let leaf = eval.eval(self.leaf, env);
        eval.depth = base_depth;
        let mut value = leaf?;

        if let V::Dense(input) = &value {
            if self.has_headroom(input) {
                match self.op {
                    CoreOp::Negate => {
                        if let Some(values) = input.real_slice() {
                            self.charge_stages(eval, input.len())?;
                            let span = program.node(self.stages[0]).span;
                            let lease = resources::reserve_for::<f64>(values.len())
                                .map_err(|reason| error(span, reason))?;
                            let out = if self.stages.len() % 2 == 0 {
                                values.to_vec()
                            } else {
                                values.iter().map(|x| -x).collect()
                            };
                            return Dense::pack_real_reserved(
                                input.shape().into(),
                                out,
                                lease,
                                span,
                            );
                        }
                    }
                    CoreOp::Not => {
                        if let Some(values) = input.bool_slice() {
                            self.charge_stages(eval, input.len())?;
                            let span = program.node(self.stages[0]).span;
                            let lease = resources::reserve_for::<bool>(values.len())
                                .map_err(|reason| error(span, reason))?;
                            let out = if self.stages.len() % 2 == 0 {
                                values.to_vec()
                            } else {
                                values.iter().map(|x| !x).collect()
                            };
                            return Dense::pack_bool_reserved(
                                input.shape().into(),
                                out,
                                lease,
                                span,
                            );
                        }
                    }
                    _ => unreachable!("a plan admits only total unary kernels"),
                }
            }
        }

        // Use the same core path on unsupported storage or when the budget is
        // tight; pass the already-evaluated leaf instead of repeating it.
        for (index, &id) in self.stages.iter().enumerate().rev() {
            let node = program.node(id);
            eval.depth = base_depth + index as u16;
            let next = eval.core(
                self.op,
                vec![value],
                &node.ty,
                None,
                node.lift_mode,
                node.span,
            );
            eval.depth = base_depth;
            value = next?;
        }
        Ok(value)
    }

    fn has_headroom(&self, input: &Dense) -> bool {
        // Deliberately conservative: while the original leaf is live, allow
        // an intermediate stage and both generic and packed output buffers.
        // If accounting could fail, replay the original stage allocations.
        input
            .len()
            .checked_mul(3 * std::mem::size_of::<V>())
            .is_some_and(|bytes| bytes <= resources::available_memory())
    }

    fn charge_stages(&self, eval: &mut Eval<'_>, len: usize) -> R<()> {
        for &id in self.stages.iter().rev() {
            let span = eval.p.node(id).span;
            eval.tick(span)?;
            eval.charge_work(len, span)?;
        }
        Ok(())
    }
}

/// Called from eval_kind after the root expression's tick and depth admission.
pub(in crate::hir_eval) fn try_eval(eval: &mut Eval<'_>, root: ExprId, env: usize) -> Option<R<V>> {
    if !resources::optimized() {
        return None;
    }
    let remaining = usize::from(eval.max_depth.saturating_sub(eval.depth));
    let plan = Plan::build(eval.p, root, remaining)?;
    Some(plan.run(eval, env))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir_eval::{run_with_options, ResourcePolicy, RuntimeOptions};

    fn checked(source: &str) -> HirProgram {
        use crate::semantic::{self, Mode, Policy};
        let program = crate::parser::parse(source).unwrap();
        semantic::prepare(&program, Mode::Script, Policy::TypedOnly)
            .unwrap()
            .hir()
            .unwrap()
            .clone()
    }

    fn find(program: &HirProgram, op: CoreOp) -> ExprId {
        ExprId(
            program
                .arena
                .iter()
                .rposition(
                    |node| matches!(node.kind, ExprKind::Call(Target::Builtin(k), _) if k == op),
                )
                .expect("checked kernel"),
        )
    }

    #[test]
    fn admits_only_same_shape_direct_real_negation_and_boolean_not() {
        let program = checked("x := -: -: -: form [2n] [-0.0r 1r]; y := ~ ~ form [2n] [0b 1b];");
        let negate = find(&program, CoreOp::Negate);
        assert_eq!(Plan::build(&program, negate, 128).unwrap().stages.len(), 3);
        assert!(Plan::build(&program, negate, 2).is_none());
        let not = find(&program, CoreOp::Not);
        assert_eq!(Plan::build(&program, not, 128).unwrap().stages.len(), 2);
    }

    #[test]
    fn excludes_partial_exact_indirect_and_scalar_expressions() {
        for (source, op) in [
            ("x := -: -: form [1n] [1z];", CoreOp::Negate),
            ("x := -: -: 1r;", CoreOp::Negate),
            ("x := ^/ ^/ form [1n] [4r];", CoreOp::Sqrt),
            (
                "n:array[r]->array[r] := &-:; x := -: @ n (form [1n] [0r]);",
                CoreOp::Negate,
            ),
        ] {
            let program = checked(source);
            assert!(Plan::build(&program, find(&program, op), 128).is_none());
        }
    }
    #[test]
    fn fused_unary_chain_does_not_move_work_or_depth_failures() {
        let program = checked("x := -: -: -: form [4n] [-0.0r 1r -2r 0.0r]; !^{ ! print x; };");
        let mut passed = false;
        let mut failed = false;
        let mut compare = |limits: ResourcePolicy| {
            let attempt = |optimize| {
                run_with_options(
                    &program,
                    RuntimeOptions {
                        resources: limits,
                        optimize,
                    },
                )
                .map(|output| output.stdout)
                .map_err(|error| {
                    let diag = error.structured().unwrap();
                    (diag.primary, diag.category, error.message)
                })
            };
            let generic = attempt(false);
            assert_eq!(attempt(true), generic, "{limits:?}");
            match generic {
                Ok(stdout) => {
                    passed = true;
                    assert_eq!(stdout, "array [4] [0 -1 2 -0]\n");
                }
                Err(_) => failed = true,
            }
        };
        for max_work in 1..70 {
            compare(ResourcePolicy {
                max_work,
                ..ResourcePolicy::default()
            });
        }
        for max_depth in 1..10 {
            compare(ResourcePolicy {
                max_depth,
                ..ResourcePolicy::default()
            });
        }
        assert!(passed && failed);
    }
}
