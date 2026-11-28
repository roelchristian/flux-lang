//! Typed, owned execution IR retaining stable source syntax IDs.
mod lower;
mod node;
mod program;
mod validate;

pub use crate::core::{lifting::LiftMode, operation::CoreOp};
pub use crate::diagnostic::Span;
pub(crate) use lower::lower_live;
pub use lower::{lower, Boundary, BoundaryKind};
pub use node::{
    BoundaryId, ConstructorDef, Definition, ExprId, ExprKind, ExprNode, Literal, MatchArm, Pattern,
    PipelineStage, RecoveryArm, Step, TableSchema, Target, ValuePattern, ValuePatternKind,
};
pub use program::HirProgram;
pub use validate::validate;

use crate::types::Type;

fn lift_for_call(
    op: CoreOp,
    args: &[ExprId],
    arena: &[ExprNode],
    result: &Type,
) -> Result<Option<LiftMode>, &'static str> {
    let Some(arity) = op.lift_arity() else {
        return Ok(None);
    };
    let mode = match (arity, args) {
        (1, [left]) => LiftMode::for_operands(&arena[left.0].ty, None)?,
        (2, [left, right]) => LiftMode::for_operands(&arena[left.0].ty, Some(&arena[right.0].ty))?,
        _ => return Err("invalid direct lifting arity"),
    };
    let dense = matches!(
        mode,
        LiftMode::LeftArray | LiftMode::RightArray | LiftMode::ArrayPair
    );
    if *result != Type::Never && dense != result.array_element().is_some() {
        return Err("HIR lifting evidence disagrees with result shape");
    }
    Ok(Some(mode))
}
