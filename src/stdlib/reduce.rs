//! Checked, seedless left reduction of a sequence.
use super::{invariant, Eval, Rc, Span, Type, R, V};

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
    let acc = rest.iter().try_fold(first.clone(), |acc, item| {
        eval.apply_dyadic(callback, (acc, result), (item.clone(), input), result, span)
    })?;
    Ok(V::Optional(Some(Rc::new(acc))))
}
