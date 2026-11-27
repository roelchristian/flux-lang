//! Collection operations used by the typed evaluator. The checker determines
//! admissible collection kinds and result types; this layer checks runtime
//! bounds and shapes, and carries the typed table layout into empty results.
use super::*;
use std::collections::HashSet;

const DISTINCT_LIMIT: usize = 8192;

fn exact(n: i128) -> Exact {
    Exact::integer(n, Domain::Rational)
}

fn some(v: V) -> V {
    V::Optional(Some(Rc::new(v)))
}

fn index(v: &V, span: Span) -> R<usize> {
    let n = as_exact(v)
        .and_then(Exact::integral)
        .map_err(|e| error(span, e))?;
    if n < 0 {
        return Err(error(span, "expected a nonnegative integer"));
    }
    // Any natural wider than usize is past the end of a representable collection.
    Ok(usize::try_from(n).unwrap_or(usize::MAX))
}

fn fold_scan(eval: &mut Eval<'_>, args: &[V], scan: bool, span: Span) -> R<V> {
    let [f, mut acc, sequence] = [args[0].clone(), args[1].clone(), args[2].clone()];
    let items = as_seq(&sequence).map_err(|e| error(span, e))?;
    let mut prefix = if scan {
        let mut out = Vec::with_capacity(items.len() + 1);
        out.push(acc.clone());
        out
    } else {
        Vec::new()
    };
    for item in items {
        let part = eval.apply(f.clone(), vec![acc], span)?;
        acc = eval.apply(part, vec![item.clone()], span)?;
        if scan {
            prefix.push(acc.clone());
        }
    }
    Ok(if scan { wrap(prefix, None) } else { acc })
}

fn concat(args: &[V], span: Span) -> R<V> {
    let (left, a) = collection(args[0].clone(), span)?;
    let (right, b) = collection(args[1].clone(), span)?;
    if a != b {
        return Err(error(
            span,
            "concatenation requires matching collection kinds and table schemas",
        ));
    }
    let mut rows = Vec::with_capacity(left.len().saturating_add(right.len()));
    rows.extend(left.iter().cloned());
    rows.extend(right.iter().cloned());
    Ok(wrap(rows, a))
}

fn pair(a: &V, b: &V) -> V {
    V::Tuple(Rc::from([a.clone(), b.clone()]))
}

fn combinations(op: CoreOp, args: &[V], schema: Option<&Rc<TableSchema>>, span: Span) -> R<V> {
    let (left, a) = collection(args[0].clone(), span)?;
    let (right, b) = collection(args[1].clone(), span)?;
    if op == CoreOp::Zip {
        if a.is_some() || b.is_some() {
            return Err(error(span, "zip requires two sequences"));
        }
        if left.len() != right.len() {
            return Err(error(span, "zip requires equal lengths"));
        }
        return Ok(wrap(
            left.iter()
                .zip(right.iter())
                .map(|(x, y)| pair(x, y))
                .collect(),
            None,
        ));
    }
    let output_schema = match (a, b) {
        (None, None) => None,
        (Some(_), Some(_)) => Some(layout(schema, span)?),
        _ => return Err(error(span, "incompatible collection kinds")),
    };
    let count = left.len().saturating_mul(right.len());
    if count > COLLECTION_LIMIT {
        return Err(error(span, "prototype cross-product limit exceeded"));
    }
    let mut out = Vec::with_capacity(count);
    for left in left.iter() {
        out.extend(right.iter().map(|right| pair(left, right)));
    }
    Ok(wrap(out, output_schema))
}

fn distinct(v: &V, span: Span) -> R<V> {
    let xs = as_seq(v).map_err(|e| error(span, e))?;
    if xs.len() > DISTINCT_LIMIT {
        return Err(error(
            span,
            "prototype distinct limit (8192 items) exceeded",
        ));
    }
    let mut out = Vec::with_capacity(xs.len());
    let mut scalars = HashSet::new();
    for entry in xs {
        // The scalar hash uses exactly equal's scalar equivalences (including
        // numeric kinds and signed real zero). Composite equality falls back
        // to ordered structural comparisons.
        let novel = key(entry).map_or_else(
            || !out.iter().any(|old| equal(old, entry)),
            |k| scalars.insert(k),
        );
        if novel {
            out.push(entry.clone());
        }
    }
    Ok(wrap(out, None))
}

fn lookup(args: &[V], span: Span) -> R<V> {
    let key = &args[0];
    let found = match &args[1] {
        V::Dict(xs) => xs
            .iter()
            .find(|(k, _)| equal(k, key))
            .map(|(_, v)| v.clone()),
        v => {
            let (xs, _) = collection(v.clone(), span)?;
            index(key, span)?
                .checked_sub(1)
                .and_then(|i| xs.get(i).cloned())
        }
    };
    Ok(found.map_or(V::Optional(None), some))
}

fn window(op: CoreOp, args: &[V], span: Span) -> R<V> {
    use CoreOp::*;
    let first = index(&args[0], span)?;
    let (count, input) = if op == Slice {
        (index(&args[1], span)?, &args[2])
    } else {
        (0, &args[1])
    };
    let limits = |n: usize| -> R<(usize, usize)> {
        Ok(match op {
            Take => (0, first.min(n)),
            Drop => (first.min(n), n),
            Slice => {
                let start = first
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "slice start is one-based"))?
                    .min(n);
                (start, start.saturating_add(count).min(n))
            }
            _ => return Err(invariant(span, "invalid collection-window operation")),
        })
    };
    if let V::String(s) = input {
        let (a, b) = limits(s.chars().count())?;
        return Ok(V::String(Rc::from(
            s.chars().skip(a).take(b - a).collect::<String>(),
        )));
    }
    let (rows, schema) = collection(input.clone(), span)?;
    let (a, b) = limits(rows.len())?;
    Ok(wrap(rows[a..b].to_vec(), schema))
}

fn reverse(v: &V, span: Span) -> R<V> {
    if let V::String(s) = v {
        return Ok(V::String(Rc::from(s.chars().rev().collect::<String>())));
    }
    let (items, schema) = collection(v.clone(), span)?;
    Ok(wrap(items.iter().rev().cloned().collect(), schema))
}

fn matrix(op: CoreOp, v: &V, span: Span) -> R<V> {
    let xs = as_seq(v).map_err(|e| error(span, e))?;
    if op == CoreOp::Flatten {
        let mut out = Vec::new();
        for row in xs {
            out.extend(as_seq(row).map_err(|e| error(span, e))?.iter().cloned());
        }
        return Ok(wrap(out, None));
    }
    let rows = xs
        .iter()
        .map(as_seq)
        .collect::<NumericResult<Vec<_>>>()
        .map_err(|e| error(span, e))?;
    let cols = rows.first().map_or(0, |r| r.len());
    if rows.iter().any(|r| r.len() != cols) {
        return Err(error(span, "transpose requires a rectangular sequence"));
    }
    let mut out = (0..cols)
        .map(|_| Vec::with_capacity(rows.len()))
        .collect::<Vec<_>>();
    for row in rows {
        for (col, cell) in out.iter_mut().zip(row) {
            col.push(cell.clone());
        }
    }
    Ok(wrap(
        out.into_iter().map(|col| wrap(col, None)).collect(),
        None,
    ))
}

fn extrema(op: CoreOp, v: &V, span: Span) -> R<V> {
    let seq = as_seq(v).map_err(|e| error(span, e))?;
    let Some((mut best, tail)) = seq.split_first() else {
        return Ok(V::Optional(None));
    };
    for item in tail {
        let c = order(best, item).map_err(|e| error(span, e))?;
        if if op == CoreOp::Max {
            c.is_lt()
        } else {
            c.is_gt()
        } {
            best = item;
        }
    }
    Ok(some(best.clone()))
}

fn bool_reduce(op: CoreOp, v: &V, span: Span) -> R<V> {
    let seq = as_seq(v).map_err(|e| error(span, e))?;
    let all = op == CoreOp::All;
    let mut result = all;
    for value in seq {
        let value = as_bool(value).map_err(|e| error(span, e))?;
        result = if all {
            result && value
        } else {
            result || value
        };
    }
    Ok(V::Bool(result))
}

fn variance(v: &V) -> NumericResult<V> {
    use CoreOp::*;
    let seq = as_seq(v)?;
    if seq.is_empty() {
        return Ok(V::Optional(None));
    }
    let count = exact(seq.len() as i128);
    let mut sum = exact(0);
    for v in seq {
        sum = sum.compute(Add, as_exact(v)?, Domain::Rational)?;
    }
    let mean = sum.compute(Div, count, Domain::Rational)?;
    let mut squares = exact(0);
    for v in seq {
        let t = as_exact(v)?.compute(Sub, mean, Domain::Rational)?;
        squares = squares.compute(Add, t.compute(Mul, t, Domain::Rational)?, Domain::Rational)?;
    }
    Ok(some(V::Exact(squares.compute(
        Div,
        count,
        Domain::Rational,
    )?)))
}

fn percentile(p: &V, v: &V) -> NumericResult<V> {
    use CoreOp::*;
    let p = as_exact(p)?;
    let hundred = exact(100);
    if p.n < 0 || p.order(hundred).is_gt() {
        return Err("percentile must be between 0 and 100");
    }
    let mut xs = as_seq(v)?
        .iter()
        .map(as_exact)
        .collect::<NumericResult<Vec<_>>>()?;
    if xs.is_empty() {
        return Ok(V::Optional(None));
    }
    xs.sort_by(|a, b| a.order(*b));
    let h = p
        .compute(Mul, exact((xs.len() - 1) as i128), Domain::Rational)?
        .compute(Div, hundred, Domain::Rational)?;
    let i = (h.n / h.d) as usize;
    let x = Exact::new(xs[i].n, xs[i].d, Domain::Rational)?;
    let remainder = h.n % h.d;
    let result = if remainder == 0 {
        x
    } else {
        let y = Exact::new(xs[i + 1].n, xs[i + 1].d, Domain::Rational)?;
        let delta = y.compute(Sub, x, Domain::Rational)?;
        let weight = Exact::new(remainder, h.d, Domain::Rational)?;
        x.compute(
            Add,
            delta.compute(Mul, weight, Domain::Rational)?,
            Domain::Rational,
        )?
    };
    Ok(some(V::Exact(result)))
}

pub(super) fn apply(
    op: CoreOp,
    args: &[V],
    _ty: &Type,
    schema: Option<&Rc<TableSchema>>,
    eval: &mut Eval<'_>,
    span: Span,
) -> Option<R<V>> {
    use CoreOp::*;
    Some(match op {
        Fold | Scan => fold_scan(eval, args, op == Scan, span),
        Concat => concat(args, span),
        Distinct => distinct(&args[0], span),
        Lookup => lookup(args, span),
        Zip | Cartesian => combinations(op, args, schema, span),
        Take | Drop | Slice => window(op, args, span),
        Reverse => reverse(&args[0], span),
        Flatten | Transpose => matrix(op, &args[0], span),
        Variance => variance(&args[0]).map_err(|e| error(span, e)),
        Percentile => percentile(&args[0], &args[1]).map_err(|e| error(span, e)),
        Max | Min => extrema(op, &args[0], span),
        All | Any => bool_reduce(op, &args[0], span),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(xs: &[i128]) -> V {
        wrap(
            xs.iter()
                .map(|&n| V::Exact(Exact::integer(n, Domain::Integer)))
                .collect(),
            None,
        )
    }

    #[test]
    fn extrema_and_stable_distinct() {
        let span = Span::at(0);
        let values = nums(&[3, 1, 3, 2]);
        assert_eq!(
            repr(&extrema(CoreOp::Min, &values, span).ok().unwrap()),
            "?1 1"
        );
        assert_eq!(
            repr(&extrema(CoreOp::Max, &values, span).ok().unwrap()),
            "?1 3"
        );
        assert_eq!(repr(&distinct(&values, span).ok().unwrap()), "[3 1 2]");
    }

    #[test]
    fn empty_cartesian_preserves_checked_output_schema() {
        let span = Span::at(0);
        let shape = Rc::new(TableSchema::Positional(vec![Type::Prim(Prim::Z)]));
        let layout = Rc::new(TableSchema::Positional(vec![
            Type::Tuple(vec![Type::Prim(Prim::Z)]),
            Type::Tuple(vec![Type::Prim(Prim::Z)]),
        ]));
        let input = V::Table(Rc::from([]), shape);
        let result = combinations(
            CoreOp::Cartesian,
            &[input.clone(), input],
            Some(&layout),
            span,
        );
        assert!(matches!(result, Ok(V::Table(xs, s)) if xs.is_empty() && Rc::ptr_eq(&s, &layout)));
    }

    #[test]
    fn enclosing_exact_statistics_are_deterministic() {
        let values = nums(&[0, 5, 10]);
        assert_eq!(repr(&variance(&values).unwrap()), "?1 50/3");
        assert_eq!(
            repr(&percentile(&V::Exact(exact(25)), &values).unwrap()),
            "?1 5/2",
        );
    }
}
