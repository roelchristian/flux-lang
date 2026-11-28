//! Checked, seedless left reduction of a sequence.
use super::*;

pub(super) fn apply(
    args: &[V],
    types: &[Type],
    output: &Type,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    let ([callback, V::Sequence(items)], [_, Type::Seq(input)], Type::Optional(result)) =
        (args, types, output)
    else {
        return Err(invariant(
            span,
            "reduce requires checked sequence and optional witnesses",
        ));
    };
    let Some((first, rest)) = items.split_first() else {
        return Ok(V::Optional(None));
    };
    let mut acc = first.clone();
    let residual = Type::Fun(input.clone(), result.clone());
    for item in rest {
        let part = eval.apply_checked(
            callback.clone(),
            vec![acc],
            Some(vec![(**result).clone()]),
            Some(residual.clone()),
            span,
        )?;
        acc = eval.apply_checked(
            part,
            vec![item.clone()],
            Some(vec![(**input).clone()]),
            Some((**result).clone()),
            span,
        )?;
    }
    Ok(V::Optional(Some(Rc::new(acc))))
}
