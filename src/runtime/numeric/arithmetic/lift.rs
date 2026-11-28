//! Apply a scalar kernel using the lift shape already selected in HIR.
use super::NumericResult;
use crate::{
    hir::LiftMode,
    hir_eval::{array, as_seq, V},
};

pub(in crate::hir_eval) fn lift(
    a: V,
    b: V,
    mode: Option<LiftMode>,
    f: &impl Fn(V, V) -> NumericResult<V>,
) -> NumericResult<V> {
    // Indirect callable references are not specialized yet. Direct calls carry
    // an explicit static mode and never use these value tags for selection.
    let mode = mode.ok_or("direct or indirect call lacks checked lifting shape")?;
    Ok(match mode {
        LiftMode::Scalar => f(a, b)?,
        LiftMode::LeftSequence => {
            let xs = as_seq(&a)?;
            V::Sequence(
                xs.iter()
                    .cloned()
                    .map(|x| f(x, b.clone()))
                    .collect::<NumericResult<Vec<_>>>()?
                    .into(),
            )
        }
        LiftMode::RightSequence => {
            let xs = as_seq(&b)?;
            V::Sequence(
                xs.iter()
                    .cloned()
                    .map(|x| f(a.clone(), x))
                    .collect::<NumericResult<Vec<_>>>()?
                    .into(),
            )
        }
        LiftMode::LeftArray | LiftMode::RightArray | LiftMode::ArrayPair => {
            array::lift_binary(a, b, mode, f)?
        }
        LiftMode::Pairwise => {
            let (a, b) = (as_seq(&a)?, as_seq(&b)?);
            if a.len() != b.len() {
                return Err("shape mismatch: sequence lengths differ");
            }
            V::Sequence(
                a.iter()
                    .cloned()
                    .zip(b.iter().cloned())
                    .map(|(x, y)| f(x, y))
                    .collect::<NumericResult<Vec<_>>>()?
                    .into(),
            )
        }
    })
}

pub(in crate::hir_eval) fn lift_one(
    a: V,
    mode: Option<LiftMode>,
    f: &impl Fn(V) -> NumericResult<V>,
) -> NumericResult<V> {
    let mode = mode.ok_or("direct or indirect call lacks checked lifting shape")?;
    match mode {
        LiftMode::Scalar => f(a),
        LiftMode::LeftSequence => Ok(V::Sequence(
            as_seq(&a)?
                .iter()
                .cloned()
                .map(f)
                .collect::<NumericResult<Vec<_>>>()?
                .into(),
        )),
        LiftMode::LeftArray => array::lift_unary(a, f),
        _ => Err("invalid unary lifting mode in typed HIR"),
    }
}
