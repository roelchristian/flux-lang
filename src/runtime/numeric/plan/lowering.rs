//! Discover a bounded direct-call spine without evaluating anything.
use super::ir::{Candidate, Kernel, LiteralBool, LiteralReal, Stage};
use crate::{
    hir::{CoreOp, ExprId, ExprKind, HirProgram, LiftMode, Literal, Target},
    hir_eval::numeric::TotalRealUnary,
    types::{Prim, Type},
};

fn bool_literal(program: &HirProgram, id: ExprId) -> Option<LiteralBool> {
    let ExprKind::Literal(Literal::Bool(value)) = program.arena.get(id.0)?.kind else {
        return None;
    };
    Some(LiteralBool { id, value })
}

fn total_real_scalar(program: &HirProgram, op: CoreOp, id: ExprId) -> Option<LiteralReal> {
    let node = program.arena.get(id.0)?;
    if node.ty != Type::Prim(Prim::R) {
        return None;
    }
    // Mirror checked r-literal decoding. Never speculate on a binding,
    // conversion or function call.
    let value: f64 = match &node.kind {
        ExprKind::Literal(Literal::Integer(x)) => *x as f64,
        ExprKind::Literal(Literal::Numeric(text)) => {
            text.strip_suffix('r').unwrap_or(text).parse().ok()?
        }
        _ => return None,
    };
    let total = value.is_finite()
        && match op {
            CoreOp::Add | CoreOp::Sub => value == 0.0,
            CoreOp::Mul => value.abs() <= 1.0,
            CoreOp::Div => value.abs() >= 1.0,
            _ => false,
        };
    total.then_some(LiteralReal {
        id,
        bits: value.to_bits(),
    })
}

pub(super) fn stage(program: &HirProgram, id: ExprId) -> Option<Stage> {
    let node = program.arena.get(id.0)?;
    if node.lift_mode != Some(LiftMode::LeftArray) {
        return None;
    }
    let ExprKind::Call(Target::Builtin(op), operands) = &node.kind else {
        return None;
    };
    let (input, kernel) = match (*op, operands.as_slice()) {
        (CoreOp::Not, [input]) => (input, Kernel::Not),
        (CoreOp::And, [input, rhs]) => (input, Kernel::And(bool_literal(program, *rhs)?)),
        (CoreOp::Or, [input, rhs]) => (input, Kernel::Or(bool_literal(program, *rhs)?)),
        (CoreOp::ToReal, [input]) => (input, Kernel::RealIdentity),
        (op @ (CoreOp::Add | CoreOp::Sub | CoreOp::Mul | CoreOp::Div), [input, rhs]) => (
            input,
            Kernel::RealScalar {
                op,
                literal: total_real_scalar(program, op, *rhs)?,
            },
        ),
        (op, [input]) => (input, Kernel::Real(TotalRealUnary::from_core(op)?)),
        _ => return None,
    };
    let element: Prim = kernel.element();
    let input_type = &program.arena.get(input.0)?.ty;
    if node.ty.array_element() != Some(&Type::Prim(element)) || input_type != &node.ty {
        return None;
    }
    Some(Stage {
        id,
        input: *input,
        op: *op,
        kernel,
    })
}

pub(super) fn build(program: &HirProgram, root: ExprId, limit: usize) -> Option<Candidate> {
    if limit < 2 {
        return None;
    }
    let outer = stage(program, root)?;
    let inner = stage(program, outer.input)?;
    let mut stages = vec![outer, inner];
    let mut leaf = inner.input;
    while let Some(next) = stage(program, leaf) {
        if stages.len() == limit {
            // Leave depth rejection to the normal walk at the exact source.
            return None;
        }
        stages.push(next);
        leaf = next.input;
    }
    Some(Candidate { root, stages, leaf })
}
