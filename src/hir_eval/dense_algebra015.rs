//! Shape-changing operations and deterministic axis kernels for dense arrays.
use super::*;

fn materialize(
    input: &Dense,
    shape: Rc<[usize]>,
    start: usize,
    strides: &[usize],
    span: Span,
) -> R<V> {
    let len = volume(&shape, span)?;
    let cells = (0..len)
        .map(|mut index| {
            let mut offset = start;
            for (&width, &stride) in shape.iter().zip(strides).rev() {
                offset += (index % width) * stride;
                index /= width;
            }
            input.cells[offset].clone()
        })
        .collect();
    Dense::pack(shape, cells, span)
}

pub(super) fn reorder(input: &Dense, permutation: &[usize], span: Span) -> R<V> {
    let rank = input.shape.len();
    if permutation.iter().copied().eq(0..rank) {
        return Dense::pack(input.shape.clone(), input.cells.clone(), span);
    }
    let shape: Rc<[usize]> = permutation.iter().map(|&i| input.shape[i]).collect();
    if input.cells.is_empty() {
        return Dense::pack(shape, input.cells.clone(), span);
    }
    let old = broadcast_strides(&input.shape, rank);
    let strides: Vec<_> = permutation.iter().map(|&i| old[i]).collect();
    materialize(input, shape, 0, &strides, span)
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
    if shape.contains(&0) {
        return Dense::pack(shape, Rc::from([]), span);
    }
    let strides = broadcast_strides(&input.shape, rank);
    let start = starts
        .iter()
        .zip(strides)
        .map(|(i, stride)| i * stride)
        .sum();
    materialize(input, shape, start, &strides[..rank], span)
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
        let mut items = (0..width).map(|i| &input.cells[start + i * stride]);
        let value = match op {
            LaneOp::Intrinsic(op, ty) => match op {
                CoreOp::Sum | CoreOp::Product => {
                    numeric_reduce(op, items.cloned(), ty).map_err(|e| error(span, e))?
                }
                CoreOp::Min | CoreOp::Max => collections042::extrema_iter(op, items, span)?,
                CoreOp::All | CoreOp::Any => collections042::bool_reduce_iter(op, items, span)?,
                _ => return Err(invariant(span, "invalid intrinsic axis reduction")),
            },
            LaneOp::Reduce { callback, item } => {
                let first = items.next();
                let acc = first
                    .map(|first| {
                        items.try_fold(first.clone(), |acc, value| {
                            eval.apply_dyadic(
                                callback,
                                (acc, item),
                                (value.clone(), item),
                                item,
                                span,
                            )
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
                        (value.clone(), item),
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
    Dense::pack(shape.into(), result.into(), span)
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
    eval.fuel = work
        .and_then(|work| eval.fuel.checked_sub(work))
        .ok_or_else(|| error(span, "prototype execution limit exhausted"))?;
    Ok(())
}

pub(super) fn dot(left: &Dense, right: &Dense, ty: &Type, eval: &mut Eval<'_>, span: Span) -> R<V> {
    let ([a], [b]) = (left.shape.as_ref(), right.shape.as_ref()) else {
        return Err(error(span, "arr.dot requires rank-one arrays"));
    };
    if a != b {
        return Err(error(span, "arr.dot requires equal vector lengths"));
    }
    charge_contraction(eval, Some(*a), span)?;
    contracted(left.cells.iter().cloned(), right.cells.iter().cloned(), ty)
        .map_err(|e| error(span, e))
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
    let mut cells = Vec::with_capacity(len);
    for i in 0..*m {
        let left_row = &left.cells[i * k..(i + 1) * k];
        for j in 0..*n {
            let right_column = (0..*k).map(|i| right.cells[i * n + j].clone());
            cells.push(
                contracted(left_row.iter().cloned(), right_column, ty)
                    .map_err(|e| error(span, e))?,
            );
        }
    }
    Dense::pack(shape, cells.into(), span)
}

pub(super) fn diag(input: &Dense, span: Span) -> R<V> {
    let [m, n] = input.shape.as_ref() else {
        return Err(error(span, "arr.diag requires a rank-two array"));
    };
    let len = *m.min(n);
    let diagonal = (0..len).map(|i| input.cells[i * (n + 1)].clone()).collect();
    Dense::pack(Rc::from([len]), diagonal, span)
}
