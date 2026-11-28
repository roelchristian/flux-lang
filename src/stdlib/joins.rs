//! Stable, bounded typed equijoins over checked row and key witnesses.
use super::{
    error, invariant, key_with_budget, relational::grouping::key_slot, resources, CoreOp, Eval,
    HashMap, Key, Rc, Span, Table, TableSchema, Type, KEY_WORK_LIMIT, R, V,
};

pub(super) fn handles(op: CoreOp) -> bool {
    use CoreOp::*;
    matches!(op, InnerJoin | LeftJoin | SemiJoin | AntiJoin)
}

struct KeyPlan<'a> {
    callback: &'a V,
    table: &'a Table,
    row_type: &'a Type,
    key_type: &'a Type,
    column: Option<usize>,
}

impl<'a> KeyPlan<'a> {
    fn new(callback: &'a V, table: &'a Table, row_type: &'a Type, key_type: &'a Type) -> Self {
        let column = (resources::optimized() && table.columnar())
            .then(|| key_slot(callback, table))
            .flatten();
        Self {
            callback,
            table,
            row_type,
            key_type,
            column,
        }
    }

    fn row(&self, index: usize, span: Span) -> R<V> {
        self.table
            .get(index)
            .ok_or_else(|| invariant(span, "checked join row index out of bounds"))
    }

    fn key(&self, eval: &mut Eval<'_>, index: usize, remaining: &mut usize, span: Span) -> R<Key> {
        let value = if let Some(slot) = self.column {
            // A checked bare selector is total on a valid typed table, and
            // its ordinary application charges one tick at the call span.
            eval.tick(span)?;
            self.table
                .cell(index, slot)
                .ok_or_else(|| invariant(span, "checked join column index out of bounds"))?
        } else {
            eval.apply_unary(
                self.callback.clone(),
                self.row(index, span)?,
                self.row_type.clone(),
                self.key_type.clone(),
                span,
            )?
        };
        key_with_budget(&value, remaining, 0).ok_or_else(|| {
            error(
                span,
                "join key exceeds supported hash complexity or resource limit",
            )
        })
    }
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
    let [left_fn, right_fn, V::Table(left), V::Table(right)] = args.as_slice() else {
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
        left.schema().clone()
    };
    let fixed_left = KeyPlan::new(left_fn, left, left_ty, left_key);
    let fixed_right = KeyPlan::new(right_fn, right, right_ty, right_key);
    let mut remaining = KEY_WORK_LIMIT;
    let mut index = HashMap::<Key, Vec<usize>>::new();
    for i in 0..right.len() {
        let key = fixed_right.key(eval, i, &mut remaining, span)?;
        let matches = index.entry(key).or_default();
        if paired {
            matches.push(i);
        }
    }
    let mut candidates = if !paired && left.columnar() {
        resources::reserve_for::<usize>(left.len())
            .ok()
            .and_then(|lease| {
                let mut indices = Vec::new();
                indices.try_reserve_exact(left.len()).ok()?;
                Some((indices, lease))
            })
    } else {
        None
    };
    let mut rows = Vec::new();
    for i in 0..left.len() {
        let key = fixed_left.key(eval, i, &mut remaining, span)?;
        let matches = index.get(&key);
        let count = match (op, matches) {
            (InnerJoin | LeftJoin, Some(ix)) => ix.len(),
            (LeftJoin | AntiJoin, None) | (SemiJoin, Some(_)) => 1,
            _ => 0,
        };
        let previous = candidates
            .as_ref()
            .map_or(rows.len(), |(indices, _)| indices.len());
        if count > resources::limits().max_elements - previous {
            return Err(error(span, "join output row limit exceeded"));
        }
        match (op, matches) {
            (InnerJoin | LeftJoin, Some(ix)) => {
                let row = fixed_left.row(i, span)?;
                for &j in ix {
                    eval.tick(span)?;
                    let matched = fixed_right.row(j, span)?;
                    let second = if op == LeftJoin {
                        V::Optional(Some(Rc::new(matched)))
                    } else {
                        matched
                    };
                    rows.push(V::Tuple(vec![row.clone(), second].into()));
                }
            }
            (LeftJoin, None) => rows.push(V::Tuple(
                vec![fixed_left.row(i, span)?, V::Optional(None)].into(),
            )),
            (SemiJoin, Some(_)) | (AntiJoin, None) => {
                if let Some((indices, _)) = candidates.as_mut() {
                    indices.push(i);
                } else {
                    rows.push(fixed_left.row(i, span)?);
                }
            }
            _ => {}
        }
    }
    if let Some((indices, lease)) = candidates {
        return left.gather(indices, lease, span);
    }

    Ok(Table::from_rows(rows.into(), layout))
}
