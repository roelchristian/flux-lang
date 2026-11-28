//! Stable, bounded typed equijoins over checked row and key witnesses.
use super::*;

pub(super) fn handles(op: CoreOp) -> bool {
    use CoreOp::*;
    matches!(op, InnerJoin | LeftJoin | SemiJoin | AntiJoin)
}

fn checked_key(
    eval: &mut Eval<'_>,
    f: &V,
    row: &V,
    row_ty: &Type,
    key_ty: &Type,
    remaining: &mut usize,
    span: Span,
) -> R<Key> {
    let value = eval.apply_unary(f.clone(), row.clone(), row_ty.clone(), key_ty.clone(), span)?;
    key_with_budget(&value, remaining, 0).ok_or_else(|| {
        error(
            span,
            "join key exceeds supported hash complexity or resource limit",
        )
    })
}

pub(super) fn apply(
    op: CoreOp,
    args: Vec<V>,
    types: &[Type],
    output: &Type,
    schema: Option<&Rc<TableSchema>>,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    use CoreOp::*;
    eval.tick(span)?;
    let [left_fn, right_fn, V::Table(left, left_schema), V::Table(right, _)] = args.as_slice()
    else {
        return Err(invariant(
            span,
            "join requires two key functions and two tables",
        ));
    };
    let [Type::Fun(_, left_key), Type::Fun(_, right_key), Type::Table(left_ty), Type::Table(right_ty)] =
        types
    else {
        return Err(invariant(
            span,
            "join is missing checked argument witnesses",
        ));
    };
    if left_key != right_key {
        return Err(invariant(span, "join checked key witnesses differ"));
    }
    if left.len() > resources::limits().max_elements
        || right.len() > resources::limits().max_elements
    {
        return Err(error(span, "join input row limit exceeded"));
    }
    let paired = matches!(op, InnerJoin | LeftJoin);
    let layout = if paired {
        eval.p
            .instance_schema(output)
            .or_else(|| schema.cloned())
            .ok_or_else(|| invariant(span, "paired join has no checked output schema"))?
    } else {
        left_schema.clone()
    };
    let mut remaining = KEY_WORK_LIMIT;
    let mut index = HashMap::<Key, Vec<usize>>::new();
    for (i, row) in right.iter().enumerate() {
        let key = checked_key(
            eval,
            right_fn,
            row,
            right_ty,
            right_key,
            &mut remaining,
            span,
        )?;
        let matches = index.entry(key).or_default();
        if paired {
            matches.push(i);
        }
    }
    let mut rows = Vec::new();
    for row in left.iter() {
        let key = checked_key(eval, left_fn, row, left_ty, left_key, &mut remaining, span)?;
        let matches = index.get(&key);
        let count = match (op, matches) {
            (InnerJoin | LeftJoin, Some(ix)) => ix.len(),
            (LeftJoin | AntiJoin, None) | (SemiJoin, Some(_)) => 1,
            _ => 0,
        };
        if count > resources::limits().max_elements - rows.len() {
            return Err(error(span, "join output row limit exceeded"));
        }
        match (op, matches) {
            (InnerJoin | LeftJoin, Some(ix)) => {
                for &i in ix {
                    eval.tick(span)?;
                    let second = if op == LeftJoin {
                        V::Optional(Some(Rc::new(right[i].clone())))
                    } else {
                        right[i].clone()
                    };
                    rows.push(V::Tuple(vec![row.clone(), second].into()));
                }
            }
            (LeftJoin, None) => rows.push(V::Tuple(vec![row.clone(), V::Optional(None)].into())),
            (SemiJoin, Some(_)) | (AntiJoin, None) => rows.push(row.clone()),
            _ => {}
        }
    }
    Ok(V::Table(rows.into(), layout))
}
