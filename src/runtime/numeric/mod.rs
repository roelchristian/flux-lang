//! Exact numeric representations and checked numerical execution plans.
pub(super) mod arithmetic;
pub(super) mod decimal;
pub(super) mod plan;
pub(super) mod wide;

// Checked r -> r operations whose results are finite for every finite input.
// The same implementations are used by scalar calls, packed kernels and plans.
use crate::hir::CoreOp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::hir_eval) enum TotalRealUnary {
    Negate,
    Abs,
    Sin,
    Cos,
    Atan,
    Tanh,
}

impl TotalRealUnary {
    pub(in crate::hir_eval) fn from_core(op: CoreOp) -> Option<Self> {
        Some(match op {
            CoreOp::Negate => Self::Negate,
            CoreOp::Abs => Self::Abs,
            CoreOp::MathSin => Self::Sin,
            CoreOp::MathCos => Self::Cos,
            CoreOp::MathAtan => Self::Atan,
            CoreOp::MathTanh => Self::Tanh,
            _ => return None,
        })
    }

    #[inline]
    pub(in crate::hir_eval) fn apply(self, value: f64) -> f64 {
        match self {
            Self::Negate => -value,
            Self::Abs => value.abs(),
            Self::Sin => libm::sin(value),
            Self::Cos => libm::cos(value),
            Self::Atan => libm::atan(value),
            Self::Tanh => libm::tanh(value),
        }
    }
}
