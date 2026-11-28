//! Statically selected collection lifting.
use crate::types::Type;

/// Static shape execution plan for pointwise operations. Sequences remain
/// length-matched; arrays broadcast along compatible trailing axes at runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiftMode {
    Scalar,
    LeftSequence,
    RightSequence,
    Pairwise,
    LeftArray,
    RightArray,
    ArrayPair,
}
impl LiftMode {
    /// Choose the lift using checked types, never runtime value tags.
    pub(crate) fn for_operands(left: &Type, right: Option<&Type>) -> Result<Self, &'static str> {
        let left_seq = matches!(left, Type::Seq(_));
        let right_seq = matches!(right, Some(Type::Seq(_)));
        let left_array = left.array_element().is_some();
        let right_array = right.is_some_and(|ty| ty.array_element().is_some());
        if (left_seq || right_seq) && (left_array || right_array) {
            return Err("checked lifting cannot mix sequences and dense arrays");
        }
        Ok(match (left_seq, right_seq, left_array, right_array) {
            (_, _, true, true) => Self::ArrayPair,
            (_, _, true, false) => Self::LeftArray,
            (_, _, false, true) => Self::RightArray,
            (true, true, _, _) => Self::Pairwise,
            (true, false, _, _) => Self::LeftSequence,
            (false, true, _, _) => Self::RightSequence,
            _ => Self::Scalar,
        })
    }
}
