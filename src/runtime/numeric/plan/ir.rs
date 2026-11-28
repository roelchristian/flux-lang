//! Candidate plan data, preceding the local safety checks in `validation`.
use crate::{
    hir::{CoreOp, ExprId},
    hir_eval::numeric::TotalRealUnary,
    types::Prim,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kernel {
    Real(TotalRealUnary),
    RealIdentity,
    RealScalar { op: CoreOp, literal: LiteralReal },
    Not,
    And(LiteralBool),
    Or(LiteralBool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LiteralBool {
    pub(super) id: ExprId,
    pub(super) value: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LiteralReal {
    pub(super) id: ExprId,
    /// Equality of the proof distinguishes the sign of zero.
    pub(super) bits: u64,
}

impl Kernel {
    pub(super) fn supports(op: CoreOp) -> bool {
        matches!(
            op,
            CoreOp::Not
                | CoreOp::And
                | CoreOp::Or
                | CoreOp::ToReal
                | CoreOp::Add
                | CoreOp::Sub
                | CoreOp::Mul
                | CoreOp::Div
        ) || TotalRealUnary::from_core(op).is_some()
    }

    pub(super) fn element(self) -> Prim {
        match self {
            Self::Real(_) | Self::RealIdentity | Self::RealScalar { .. } => Prim::R,
            Self::Not | Self::And(_) | Self::Or(_) => Prim::B,
        }
    }

    pub(super) fn literal_operand(self) -> Option<ExprId> {
        match self {
            Self::And(literal) | Self::Or(literal) => Some(literal.id),
            Self::RealScalar { literal, .. } => Some(literal.id),
            _ => None,
        }
    }

    pub(super) fn real(self, x: f64) -> f64 {
        match self {
            Self::Real(op) => op.apply(x),
            Self::RealIdentity => x,
            Self::RealScalar { op, literal } => {
                let y = f64::from_bits(literal.bits);
                match op {
                    CoreOp::Add => x + y,
                    CoreOp::Sub => x - y,
                    CoreOp::Mul => x * y,
                    CoreOp::Div => x / y,
                    _ => unreachable!("validated total real scalar operation"),
                }
            }
            _ => unreachable!("validated homogeneous real plan"),
        }
    }

    pub(super) fn boolean(self, x: bool) -> bool {
        match self {
            Self::Not => !x,
            Self::And(literal) => x & literal.value,
            Self::Or(literal) => x | literal.value,
            _ => unreachable!("validated homogeneous boolean plan"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Stage {
    pub(super) id: ExprId,
    pub(super) input: ExprId,
    pub(super) op: CoreOp,
    pub(super) kernel: Kernel,
}

#[derive(Debug)]
pub(super) struct Candidate {
    pub(super) root: ExprId,
    /// Lexical descent, outermost first. Cell kernels run in reverse order.
    pub(super) stages: Vec<Stage>,
    pub(super) leaf: ExprId,
}
