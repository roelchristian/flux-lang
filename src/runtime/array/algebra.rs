//! Shape-changing operations and deterministic axis kernels for dense arrays.
use super::{
    as_bool, error, exact_index, invariant, numeric_identity, numeric_reduce, order, packed,
    resources, scalar_binary, volume, CoreOp, Dense, Eval, NumericResult, Rc, Span, Type, MAX_RANK,
    R, V,
};

pub(super) fn reorder(input: &Dense, permutation: &[usize], span: Span) -> R<V> {
    let shape = permutation.iter().map(|&i| input.shape[i]).collect();
    let strides = permutation.iter().map(|&i| input.strides[i]).collect();
    input.view(shape, strides, input.offset, span)
}

pub(super) fn axes(permutation: &[usize], input: &Dense, span: Span) -> R<V> {
    let rank = input.shape.len();
    let bad = || {
        error(
            span,
            "arr.axes requires a complete one-based axis permutation",
        )
    };
    if permutation.len() != rank {
        return Err(bad());
    }
    let mut seen = [false; MAX_RANK];
    let mut checked = Vec::with_capacity(rank);
    for &one_based in permutation {
        let axis = one_based
            .checked_sub(1)
            .filter(|&i| i < rank)
            .ok_or_else(bad)?;
        if std::mem::replace(&mut seen[axis], true) {
            return Err(bad());
        }
        checked.push(axis);
    }
    reorder(input, &checked, span)
}

pub(super) fn section(starts: &[usize], counts: &[usize], input: &Dense, span: Span) -> R<V> {
    let rank = input.shape.len();
    if starts.len() != rank || counts.len() != rank {
        return Err(error(
            span,
            "arr.sect starts and counts must match the array rank",
        ));
    }
    let starts = starts
        .iter()
        .zip(input.shape.iter())
        .map(|(&i, &width)| {
            i.checked_sub(1)
                .map(|i| i.min(width))
                .ok_or_else(|| error(span, "arr.sect starts must be one-based"))
        })
        .collect::<R<Vec<_>>>()?;
    let shape: Rc<[usize]> = input
        .shape
        .iter()
        .zip(&starts)
        .zip(counts)
        .map(|((&width, &start), &count)| count.min(width - start))
        .collect();
    let mut offset = input.offset;
    if !shape.contains(&0) {
        for (&pos, &stride) in starts.iter().zip(input.strides.iter()) {
            offset = pos
                .checked_mul(stride)
                .and_then(|delta| offset.checked_add(delta))
                .ok_or_else(|| invariant(span, "dense section offset exceeds addressable size"))?;
        }
    }
    input.view(shape, input.strides.clone(), offset, span)
}

pub(super) fn last_axis(input: &Dense, span: Span) -> R<usize> {
    input.shape.len().checked_sub(1).ok_or_else(|| {
        error(
            span,
            "array reduction or scan needs a last axis (rank zero has none)",
        )
    })
}

pub(super) fn axis(index: &V, input: &Dense, span: Span) -> R<usize> {
    exact_index(index)
        .map_err(|e| error(span, e))?
        .checked_sub(1)
        .filter(|&i| i < input.shape.len())
        .ok_or_else(|| error(span, "array axis must be within the one-based rank"))
}

pub(super) enum LaneOp<'a> {
    Intrinsic(CoreOp, &'a Type),
    Reduce {
        callback: &'a V,
        item: &'a Type,
    },
    Seeded {
        callback: &'a V,
        seed: &'a V,
        item: &'a Type,
        accumulator: &'a Type,
        scan: bool,
    },
}

pub(super) fn along_axis(
    input: &Dense,
    axis: usize,
    op: LaneOp<'_>,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    let width = input.shape[axis];
    let scan = matches!(op, LaneOp::Seeded { scan: true, .. });
    let mut shape = input.shape.to_vec();
    if scan {
        shape[axis] += 1;
    } else {
        shape.remove(axis);
    }
    // Check total output volume and widened axes before allocating or
    // calling anything. Empty shapes need no strides (their other axes can
    // be large), but a zero-width axis may still have nonempty output lanes.
    let n = volume(&shape, span)?;
    if n == 0 {
        return Dense::pack(shape.into(), Rc::from([]), span);
    }
    let shape: Rc<[usize]> = shape.into();
    if let LaneOp::Intrinsic(op, ty) = op {
        eval.charge_work(input.len().saturating_add(n), span)?;
        if let Some(result) = packed::intrinsic(input, axis, shape.clone(), op, ty, span) {
            return result;
        }
    }
    if let LaneOp::Seeded {
        callback,
        seed,
        item,
        accumulator,
        scan,
    } = op
    {
        let plan = packed::ScanPlan::new(input, axis, shape.clone(), scan, span);
        if let Some(result) = packed::seeded(plan, (item, accumulator), (callback, seed), eval) {
            return result;
        }
    }
    let lease = resources::reserve_for::<V>(n).map_err(|message| error(span, message))?;
    let lanes = if scan { n / (width + 1) } else { n };
    let stride = input.shape[axis + 1..].iter().product::<usize>();
    let base = |lane, len| (lane / stride) * len * stride + lane % stride;
    let scatter = scan && stride != 1;
    let mut result = if scatter {
        vec![V::Unit; n]
    } else {
        Vec::with_capacity(n)
    };
    let emit = |result: &mut Vec<V>, index, value| {
        if scatter {
            result[index] = value;
        } else {
            result.push(value);
        }
    };
    for lane in 0..lanes {
        let start = base(lane, width);
        let mut items = (0..width).map(|i| input.value(start + i * stride));
        let value = match op {
            LaneOp::Intrinsic(op, ty) => match op {
                CoreOp::Sum | CoreOp::Product => {
                    numeric_reduce(op, items, ty).map_err(|e| error(span, e))?
                }
                CoreOp::Min | CoreOp::Max => {
                    let Some(mut best) = items.next() else {
                        result.push(V::Optional(None));
                        continue;
                    };
                    for item in items {
                        let cmp = order(&best, &item).map_err(|e| error(span, e))?;
                        if if op == CoreOp::Min {
                            cmp.is_gt()
                        } else {
                            cmp.is_lt()
                        } {
                            best = item;
                        }
                    }
                    V::Optional(Some(Rc::new(best)))
                }
                CoreOp::All | CoreOp::Any => {
                    let all = op == CoreOp::All;
                    V::Bool(items.try_fold(all, |acc, item| {
                        let item = as_bool(&item).map_err(|e| error(span, e))?;
                        Ok(if all { acc & item } else { acc | item })
                    })?)
                }
                _ => return Err(invariant(span, "invalid intrinsic axis reduction")),
            },
            LaneOp::Reduce { callback, item } => {
                let first = items.next();
                let acc = first
                    .map(|first| {
                        items.try_fold(first, |acc, value| {
                            eval.apply_dyadic(callback, (acc, item), (value, item), item, span)
                        })
                    })
                    .transpose()?;
                V::Optional(acc.map(Rc::new))
            }
            LaneOp::Seeded {
                callback,
                seed,
                item,
                accumulator,
                scan,
            } => {
                let mut acc = seed.clone();
                let output = base(lane, width + 1);
                if scan {
                    emit(&mut result, output, acc.clone());
                }
                for (i, value) in items.enumerate() {
                    acc = eval.apply_dyadic(
                        callback,
                        (acc, accumulator),
                        (value, item),
                        accumulator,
                        span,
                    )?;
                    if scan {
                        emit(&mut result, output + (i + 1) * stride, acc.clone());
                    }
                }
                acc
            }
        };
        if !scan {
            result.push(value);
        }
    }
    Dense::pack_values_reserved(shape, result, lease, span)
}

fn contracted(
    a: impl Iterator<Item = V>,
    b: impl Iterator<Item = V>,
    ty: &Type,
) -> NumericResult<V> {
    let mut sum = numeric_identity(ty, false)?;
    for (x, y) in a.zip(b) {
        let product = scalar_binary(CoreOp::Mul, x, y, ty)?;
        sum = scalar_binary(CoreOp::Add, sum, product, ty)?;
    }
    Ok(sum)
}

fn charge_contraction(eval: &mut Eval<'_>, work: Option<usize>, span: Span) -> R<()> {
    eval.charge_work(
        work.ok_or_else(|| error(span, "prototype execution limit exhausted"))?,
        span,
    )
}

pub(super) fn dot(left: &Dense, right: &Dense, ty: &Type, eval: &mut Eval<'_>, span: Span) -> R<V> {
    let ([a], [b]) = (left.shape.as_ref(), right.shape.as_ref()) else {
        return Err(error(span, "arr.dot requires rank-one arrays"));
    };
    if a != b {
        return Err(error(span, "arr.dot requires equal vector lengths"));
    }
    charge_contraction(eval, Some(*a), span)?;
    if let Some(result) = packed::dot(left, right, ty, span) {
        return result;
    }
    contracted(left.iter(), right.iter(), ty).map_err(|e| error(span, e))
}

pub(super) fn matmul(
    left: &Dense,
    right: &Dense,
    ty: &Type,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    let ([m, k], [other_k, n]) = (left.shape.as_ref(), right.shape.as_ref()) else {
        return Err(error(span, "arr.mul requires rank-two arrays"));
    };
    if k != other_k {
        return Err(error(span, "arr.mul inner dimensions differ"));
    }
    let shape: Rc<[usize]> = Rc::from([*m, *n]);
    let len = volume(&shape, span)?;
    charge_contraction(eval, len.checked_mul(*k), span)?;
    if len == 0 {
        return Dense::pack(shape, Rc::from([]), span);
    }
    if let Some(result) = packed::matmul(left, right, shape.clone(), ty, span) {
        return result;
    }
    let lease = resources::reserve_for::<V>(len).map_err(|message| error(span, message))?;
    let mut cells = Vec::with_capacity(len);
    for i in 0..*m {
        for j in 0..*n {
            let left_row = (0..*k).map(|p| left.value(i * k + p));
            let right_column = (0..*k).map(|p| right.value(p * n + j));
            cells.push(contracted(left_row, right_column, ty).map_err(|e| error(span, e))?);
        }
    }
    Dense::pack_values_reserved(shape, cells, lease, span)
}

pub(super) fn diag(input: &Dense, span: Span) -> R<V> {
    let [m, n] = input.shape.as_ref() else {
        return Err(error(span, "arr.diag requires a rank-two array"));
    };
    let len = *m.min(n);
    let stride = if len <= 1 {
        0
    } else {
        input.strides[0]
            .checked_add(input.strides[1])
            .ok_or_else(|| error(span, "array diagonal stride exceeds the resource limit"))?
    };
    input.view(Rc::from([len]), Rc::from([stride]), input.offset, span)
}
