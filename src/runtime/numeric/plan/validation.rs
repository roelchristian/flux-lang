//! Plan-local evidence: the primitive/domain, original dependency edges and
//! source IDs, and a shape-preserving lift. Checked HIR still owns all typing.
use super::{
    ir::{Candidate, Stage},
    lowering,
};
use crate::{
    core::operation::Operation,
    hir::{CoreOp, ExprId, HirProgram},
    types::{Prim, Type},
};

/// This wrapper is constructible only by the plan validator.
pub(super) struct CheckedPlan {
    candidate: Candidate,
    element: Prim,
}

impl CheckedPlan {
    pub(super) fn validate(
        program: &HirProgram,
        candidate: Candidate,
        depth: usize,
    ) -> Option<Self> {
        let [first, _, ..] = candidate.stages.as_slice() else {
            return None;
        };
        if first.id != candidate.root || candidate.stages.len() > depth {
            return None;
        }
        let array_type = &program.arena.get(first.id.0)?.ty;
        let element = first.kernel.element();
        for (index, stage) in candidate.stages.iter().enumerate() {
            let next = candidate
                .stages
                .get(index + 1)
                .map_or(candidate.leaf, |next| next.id);
            let node = program.arena.get(stage.id.0)?;
            let checked = lowering::stage(program, stage.id)?;
            if checked.op != stage.op
                || checked.kernel != stage.kernel
                || checked.input != stage.input
                || stage.input != next
                || stage.id.0 <= next.0
                || node.ty != *array_type
                || stage.kernel.element() != element
            {
                return None;
            }
            if let Some(literal) = stage.kernel.literal_operand() {
                // Right operand is a pure scalar literal that stays in its own
                // evaluation position, including tick/depth admission.
                if literal.0 >= stage.id.0
                    || program.arena.get(literal.0)?.ty != Type::Prim(element)
                {
                    return None;
                }
            }
            match &node.evidence {
                Some(Operation::Numeric { result, .. }) if result != array_type => return None,
                Some(Operation::Numeric { .. }) => {}
                _ if matches!(
                    stage.op,
                    CoreOp::Negate
                        | CoreOp::Abs
                        | CoreOp::Add
                        | CoreOp::Sub
                        | CoreOp::Mul
                        | CoreOp::Div
                ) =>
                {
                    return None
                }
                _ => {}
            }
        }
        if &program.arena.get(candidate.leaf.0)?.ty != array_type {
            return None;
        }
        Some(Self { candidate, element })
    }

    pub(super) fn root(&self) -> ExprId {
        self.candidate.root
    }

    pub(super) fn leaf(&self) -> ExprId {
        self.candidate.leaf
    }

    pub(super) fn stages(&self) -> &[Stage] {
        &self.candidate.stages
    }

    pub(super) fn element(&self) -> Prim {
        self.element
    }

    pub(super) fn real(&self, initial: f64) -> f64 {
        self.stages()
            .iter()
            .rev()
            .fold(initial, |value, stage| stage.kernel.real(value))
    }

    pub(super) fn boolean(&self, initial: bool) -> bool {
        self.stages()
            .iter()
            .rev()
            .fold(initial, |value, stage| stage.kernel.boolean(value))
    }
}
