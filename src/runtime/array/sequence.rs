//! Explicit indexed sequence operations. Unlike dense-array operations these
//! consume and produce ordinary, potentially nested Flux sequences.
use super::super::{
    error, exact_i128, exact_index, invariant, order, resources, CoreOp, Domain, Eval, Ordering,
    Rc, Span, Type, R, V,
};

fn seq(value: &V, span: Span) -> R<&[V]> {
    let V::Sequence(values) = value else {
        return Err(invariant(
            span,
            "indexed sequence operation requires a checked sequence",
        ));
    };
    if values.len() > resources::limits().max_elements {
        return Err(error(
            span,
            "indexed sequence input exceeds the collection limit",
        ));
    }
    Ok(values)
}

fn natural(value: &V, span: Span) -> R<usize> {
    // Wide naturals above usize saturate to usize::MAX, which is larger
    // than any supported sequence and so is always out of bounds.
    exact_index(value).map_err(|e| invariant(span, e))
}

fn windows(width: &V, input: &V, span: Span) -> R<V> {
    let width = natural(width, span)?;
    if width == 0 {
        return Err(error(span, "arr.windows width must be positive"));
    }
    let input = seq(input, span)?;
    if width > input.len() {
        return Ok(V::Sequence(Rc::from([])));
    }
    let count = input.len() - width + 1;
    if count
        .checked_mul(width)
        .is_none_or(|size| size > resources::limits().max_elements)
    {
        return Err(error(
            span,
            "arr.windows output exceeds the collection limit",
        ));
    }
    Ok(V::Sequence(
        input
            .windows(width)
            .map(|w| V::Sequence(Rc::from(w)))
            .collect(),
    ))
}

fn select(indices: &V, input: &V, span: Span) -> R<V> {
    let input = seq(input, span)?;
    let indices = seq(indices, span)?;
    let mut out = Vec::with_capacity(indices.len());
    for index in indices {
        let i = natural(index, span)?;
        let Some(item) = i.checked_sub(1).and_then(|i| input.get(i)) else {
            return Ok(V::Optional(None));
        };
        out.push(item.clone());
    }
    Ok(V::Optional(Some(Rc::new(V::Sequence(out.into())))))
}

fn permute(indices: &V, input: &V, span: Span) -> R<V> {
    let input = seq(input, span)?;
    let indices = seq(indices, span)?;
    let bad = || {
        error(
            span,
            "arr.permute requires each one-based index exactly once",
        )
    };
    if indices.len() != input.len() {
        return Err(bad());
    }
    let mut seen = vec![false; input.len()];
    let mut checked = Vec::with_capacity(indices.len());
    for index in indices {
        let i = natural(index, span)?
            .checked_sub(1)
            .filter(|&i| i < input.len())
            .ok_or_else(bad)?;
        if std::mem::replace(&mut seen[i], true) {
            return Err(bad());
        }
        checked.push(i);
    }
    Ok(V::Sequence(
        checked.into_iter().map(|i| input[i].clone()).collect(),
    ))
}

fn grade(selector: &V, input: &V, types: &[Type], eval: &mut Eval<'_>, span: Span) -> R<V> {
    let [Type::Fun(item, key), Type::Seq(_)] = types else {
        return Err(invariant(
            span,
            "arr.grade is missing its checked selector type",
        ));
    };
    let input = seq(input, span)?;
    let mut keys = Vec::with_capacity(input.len());
    for (i, value) in input.iter().enumerate() {
        let key = eval.apply_unary(
            selector.clone(),
            value.clone(),
            (**item).clone(),
            (**key).clone(),
            span,
        )?;
        keys.push((key, i));
    }
    // The checker proves a single Ord domain. Confirm the runtime values
    // agree with it before the stable sort, which expects a total ordering.
    if let Some((first, _)) = keys.first() {
        for (key, _) in &keys {
            order(first, key).map_err(|why| invariant(span, why))?;
        }
    }
    let mut failure = None;
    keys.sort_by(|(left, _), (right, _)| match order(left, right) {
        Ok(cmp) => cmp,
        Err(why) => {
            failure = Some(why);
            Ordering::Equal
        }
    });
    if let Some(why) = failure {
        return Err(invariant(span, why));
    }
    Ok(V::Sequence(
        keys.into_iter()
            .map(|(_, index)| exact_i128(index as i128 + 1, Domain::Natural))
            .collect(),
    ))
}

pub(in crate::hir_eval) fn handles(op: CoreOp) -> bool {
    matches!(
        op,
        CoreOp::ArrayWindows | CoreOp::ArraySelect | CoreOp::ArrayGrade | CoreOp::ArrayPermute
    )
}

pub(in crate::hir_eval) fn apply(
    op: CoreOp,
    args: &[V],
    types: &[Type],
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    use CoreOp::*;
    let [a, b] = args else {
        return Err(invariant(span, "invalid checked indexed sequence arity"));
    };
    match op {
        ArrayWindows => windows(a, b, span),
        ArraySelect => select(a, b, span),
        ArrayGrade => grade(a, b, types, eval, span),
        ArrayPermute => permute(a, b, span),
        _ => Err(invariant(
            span,
            "checked array operation has no implementation",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir_eval::repr;

    fn n(n: i128) -> V {
        exact_i128(n, Domain::Natural)
    }

    fn ns(items: &[i128]) -> V {
        V::Sequence(items.iter().copied().map(n).collect())
    }

    fn value(result: R<V>) -> V {
        result.unwrap_or_else(|error| panic!("{}", error.into_error()))
    }

    #[test]
    fn windows_are_full_overlapping_and_bounded() {
        let span = Span::at(0);
        let source = ns(&[1, 2, 3, 4]);
        assert_eq!(
            repr(&value(windows(&n(3), &source, span))),
            "[[1 2 3] [2 3 4]]"
        );
        assert_eq!(repr(&value(windows(&n(5), &source, span))), "[]");
        assert_eq!(repr(&value(windows(&n(i128::MAX), &source, span))), "[]");
        assert!(windows(&n(0), &source, span).is_err());
        assert!(windows(&n(400), &ns(&[1; 800]), span).is_err());
    }

    #[test]
    fn gather_is_all_or_nothing_and_checks_exact_permutations() {
        let span = Span::at(0);
        let source = ns(&[1, 2, 3]);
        assert_eq!(
            repr(&value(select(&ns(&[3, 1, 3]), &source, span))),
            "?1 [3 1 3]"
        );
        assert_eq!(repr(&value(select(&ns(&[3, 0]), &source, span))), "?0");
        assert_eq!(repr(&value(select(&ns(&[i128::MAX]), &source, span))), "?0");
        assert_eq!(repr(&value(select(&ns(&[]), &source, span))), "?1 []");
        assert_eq!(
            repr(&value(permute(&ns(&[3, 1, 2]), &source, span))),
            "[3 1 2]"
        );
        assert_eq!(repr(&value(permute(&ns(&[]), &ns(&[]), span))), "[]");
        for bad in [
            &[1, 2][..],
            &[1, 1, 2],
            &[0, 1, 2],
            &[1, 2, 4],
            &[1, 2, i128::MAX],
        ] {
            assert!(permute(&ns(bad), &source, span).is_err());
        }
    }
}
