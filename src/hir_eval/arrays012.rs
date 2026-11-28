//! Checked row-major dense arrays. No shape is inferred from element values.
use super::*;

#[path = "dense_algebra015.rs"]
mod algebra;
use algebra::{axis, last_axis, LaneOp};

const MAX_RANK: usize = 16;

pub(super) struct Dense {
    shape: Rc<[usize]>,
    cells: Rc<[V]>,
}

impl Dense {
    pub(super) fn equal(&self, other: &Self) -> bool {
        self.shape == other.shape
            && self
                .cells
                .iter()
                .zip(other.cells.iter())
                .all(|(a, b)| equal(a, b))
    }

    pub(super) fn render(&self, show: impl Fn(&V) -> String) -> String {
        let shape = self
            .shape
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let cells = self.cells.iter().map(show).collect::<Vec<_>>().join(" ");
        format!("array [{shape}] [{cells}]")
    }

    fn pack(shape: Rc<[usize]>, cells: Rc<[V]>, span: Span) -> R<V> {
        if volume(&shape, span)? != cells.len() {
            return Err(error(
                span,
                "array shape does not match the number of elements",
            ));
        }
        Ok(V::Dense(Rc::new(Self { shape, cells })))
    }
}

fn volume(shape: &[usize], span: Span) -> R<usize> {
    checked_volume(shape).map_err(|message| error(span, message))
}

fn checked_volume(shape: &[usize]) -> NumericResult<usize> {
    if shape.len() > MAX_RANK {
        return Err("array rank exceeds 16");
    }
    if shape.iter().any(|&n| n > COLLECTION_LIMIT) {
        return Err("array dimension exceeds the collection limit");
    }
    if shape.contains(&0) {
        return Ok(0);
    }
    shape.iter().try_fold(1usize, |size, &n| {
        size.checked_mul(n)
            .filter(|&product| product <= COLLECTION_LIMIT)
            .ok_or("array element count exceeds the collection limit")
    })
}

fn shape(arg: &V, span: Span) -> R<Rc<[usize]>> {
    let xs = as_seq(arg).map_err(|e| invariant(span, e))?;
    if xs.len() > MAX_RANK {
        return Err(error(span, "array rank exceeds 16"));
    }
    xs.iter()
        .map(|n| exact_index(n).map_err(|e| error(span, e)))
        .collect::<Result<Rc<[usize]>, _>>()
}

fn seq(arg: &V, span: Span) -> R<Rc<[V]>> {
    match arg {
        V::Sequence(xs) => Ok(xs.clone()),
        _ => Err(invariant(
            span,
            "checked array operation expected a sequence",
        )),
    }
}

fn dense(arg: &V, span: Span) -> R<&Dense> {
    match arg {
        V::Dense(xs) => Ok(xs),
        _ => Err(invariant(span, "checked array operation expected an array")),
    }
}

fn element(ty: &Type, span: Span) -> R<&Type> {
    ty.array_element()
        .ok_or_else(|| invariant(span, "array operation is missing checked element type"))
}

fn at(arg: &V, arr: &Dense, span: Span) -> R<V> {
    let coords = as_seq(arg).map_err(|e| invariant(span, e))?;
    let mut off = 0usize;
    if coords.len() != arr.shape.len() {
        return Ok(V::Optional(None));
    }
    for (coord, &dim) in coords.iter().zip(arr.shape.iter()) {
        let coord = exact_index(coord).map_err(|e| error(span, e))?;
        if coord == 0 || coord > dim {
            return Ok(V::Optional(None));
        }
        // With an empty array, earlier prefixes can be too large for
        // offsets; the zero axis already proves there is no element.
        if arr.cells.is_empty() {
            return Ok(V::Optional(None));
        }
        off = off * dim + coord - 1;
    }
    Ok(V::Optional(arr.cells.get(off).cloned().map(Rc::new)))
}

fn broadcast(a: &[usize], b: &[usize]) -> NumericResult<Rc<[usize]>> {
    let rank = a.len().max(b.len());
    let axis = |xs: &[usize], i: usize| i.checked_sub(rank - xs.len()).map_or(1, |j| xs[j]);
    (0..rank)
        .map(|i| {
            let (x, y) = (axis(a, i), axis(b, i));
            match (x, y) {
                (x, y) if x == y => Ok(x),
                (1, y) => Ok(y),
                (x, 1) => Ok(x),
                _ => Err("incompatible trailing axes for array broadcasting"),
            }
        })
        .collect()
}

fn broadcast_strides(shape: &[usize], rank: usize) -> [usize; MAX_RANK] {
    let mut out = [0; MAX_RANK];
    let mut suffix = 1;
    for (i, &dim) in shape.iter().enumerate().rev() {
        if dim != 1 {
            out[rank - shape.len() + i] = suffix;
        }
        suffix *= dim;
    }
    out
}

/// Enumerate offsets in output row-major order. Do not construct strides
/// for zero-volume arrays: other axes of an empty shape can be very large.
fn broadcast_indices<'a>(
    shape: &'a [usize],
    left: &[usize],
    right: &[usize],
    len: usize,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    let axes = if len == 0 {
        ([0; MAX_RANK], [0; MAX_RANK])
    } else {
        (
            broadcast_strides(left, shape.len()),
            broadcast_strides(right, shape.len()),
        )
    };
    (0..len).map(move |mut index| {
        let (mut x, mut y) = (0, 0);
        for (i, &n) in shape.iter().enumerate().rev() {
            let coord = index % n;
            index /= n;
            x += axes.0[i] * coord;
            y += axes.1[i] * coord;
        }
        (x, y)
    })
}

fn expect_dense(arg: &V) -> NumericResult<&Dense> {
    match arg {
        V::Dense(array) => Ok(array),
        _ => Err("expected a checked dense array operand"),
    }
}

pub(super) fn lift_binary(
    a: V,
    b: V,
    mode: LiftMode,
    f: &impl Fn(V, V) -> NumericResult<V>,
) -> NumericResult<V> {
    let (shape, cells) = match mode {
        LiftMode::LeftArray => {
            let a = expect_dense(&a)?;
            let cells = a
                .cells
                .iter()
                .cloned()
                .map(|x| f(x, b.clone()))
                .collect::<NumericResult<Rc<_>>>()?;
            (a.shape.clone(), cells)
        }
        LiftMode::RightArray => {
            let b = expect_dense(&b)?;
            let cells = b
                .cells
                .iter()
                .cloned()
                .map(|x| f(a.clone(), x))
                .collect::<NumericResult<Rc<_>>>()?;
            (b.shape.clone(), cells)
        }
        LiftMode::ArrayPair => {
            let (a, b) = (expect_dense(&a)?, expect_dense(&b)?);
            let shape = broadcast(&a.shape, &b.shape)?;
            let len = checked_volume(&shape)?;
            let cells = broadcast_indices(&shape, &a.shape, &b.shape, len)
                .map(|(i, j)| f(a.cells[i].clone(), b.cells[j].clone()))
                .collect::<NumericResult<Rc<_>>>()?;
            (shape, cells)
        }
        _ => return Err("invalid dense lifting mode in typed HIR"),
    };
    Ok(V::Dense(Rc::new(Dense { shape, cells })))
}

pub(super) fn lift_unary(a: V, f: &impl Fn(V) -> NumericResult<V>) -> NumericResult<V> {
    let a = expect_dense(&a)?;
    let cells = a
        .cells
        .iter()
        .cloned()
        .map(f)
        .collect::<NumericResult<Rc<_>>>()?;
    Ok(V::Dense(Rc::new(Dense {
        shape: a.shape.clone(),
        cells,
    })))
}

fn pairwise(
    outer: bool,
    f: &V,
    a: (&Dense, &Type),
    b: (&Dense, &Type),
    out: &Type,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    let shape = if outer {
        a.0.shape.iter().chain(b.0.shape.iter()).copied().collect()
    } else {
        broadcast(&a.0.shape, &b.0.shape).map_err(|message| error(span, message))?
    };
    let len = volume(&shape, span)?;
    let mut cells = Vec::with_capacity(len);
    if outer {
        if len != 0 {
            for x in a.0.cells.iter() {
                for y in b.0.cells.iter() {
                    cells.push(eval.apply_dyadic(
                        f,
                        (x.clone(), a.1),
                        (y.clone(), b.1),
                        out,
                        span,
                    )?);
                }
            }
        }
    } else {
        for (x, y) in broadcast_indices(&shape, &a.0.shape, &b.0.shape, len) {
            cells.push(eval.apply_dyadic(
                f,
                (a.0.cells[x].clone(), a.1),
                (b.0.cells[y].clone(), b.1),
                out,
                span,
            )?);
        }
    }
    Dense::pack(shape, cells.into(), span)
}

fn rankmap(
    rank: usize,
    f: &V,
    input: &Dense,
    in_type: &Type,
    out_type: &Type,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    let split = input
        .shape
        .len()
        .checked_sub(rank)
        .ok_or_else(|| error(span, "requested cell rank exceeds array rank"))?;
    let frame: Rc<[usize]> = Rc::from(&input.shape[..split]);
    let n = volume(&frame, span)?;
    let mut out = Vec::with_capacity(n);
    if n != 0 {
        let cell_shape: Rc<[usize]> = Rc::from(&input.shape[split..]);
        let cell_len = volume(&cell_shape, span)?;
        for i in 0..n {
            let dense = Dense::pack(
                cell_shape.clone(),
                Rc::from(&input.cells[i * cell_len..(i + 1) * cell_len]),
                span,
            )?;
            out.push(eval.apply_unary(
                f.clone(),
                dense,
                in_type.clone(),
                out_type.clone(),
                span,
            )?);
        }
    }
    Dense::pack(frame, out.into(), span)
}

/// Position of the collection argument for operations overloaded on checked
/// array/sequence evidence. No choice of semantics is made from a value tag.
pub(super) fn overloaded_operand(op: CoreOp) -> Option<usize> {
    use CoreOp::*;
    match op {
        Count | Flatten | Transpose | Sum | Product | Max | Min | All | Any => Some(0),
        Lookup | Map | Reduce => Some(1),
        Fold | Scan => Some(2),
        _ => None,
    }
}

pub(super) fn handles(op: CoreOp) -> bool {
    use CoreOp::*;
    matches!(
        op,
        ArrayReshape
            | ArrayForm
            | ArrayShape
            | ArrayRavel
            | ArrayAt
            | ArrayMap
            | ArrayZip
            | ArrayOuter
            | ArrayRankMap
            | ArrayScan
            | ArrayAxes
            | ArraySection
            | ArrayReduce
            | ArrayFold
            | ArrayScanAxis
            | ArrayDot
            | ArrayMatMul
            | ArrayDiag
    )
}

pub(super) fn apply(
    op: CoreOp,
    args: &[V],
    types: &[Type],
    output: &Type,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    use CoreOp::*;
    eval.tick(span)?;
    match (op, args, types) {
        (ArrayReshape, [dims, xs], [_, _]) => Dense::pack(shape(dims, span)?, seq(xs, span)?, span),
        (ArrayForm, [dims, xs], [_, input]) => {
            let dims = shape(dims, span)?;
            let cells = if input.array_element().is_some() {
                dense(xs, span)?.cells.clone()
            } else {
                seq(xs, span)?
            };
            Dense::pack(dims, cells, span)
        }
        (ArrayShape, [xs], [_]) => Ok(V::Sequence(
            dense(xs, span)?
                .shape
                .iter()
                .map(|&n| exact_i128(n as i128, Domain::Natural))
                .collect(),
        )),
        (Count, [xs], [_]) => Ok(exact_i128(
            dense(xs, span)?.cells.len() as i128,
            Domain::Natural,
        )),
        (ArrayRavel | Flatten, [xs], [_]) => Ok(V::Sequence(dense(xs, span)?.cells.clone())),
        (ArrayAt | Lookup, [coords, xs], [_, _]) => at(coords, dense(xs, span)?, span),
        (ArrayMap | Map, [f, xs], [_, t]) => {
            let xs = dense(xs, span)?;
            let input = element(t, span)?;
            let out = element(output, span)?;
            let cells = xs
                .cells
                .iter()
                .map(|v| eval.apply_unary(f.clone(), v.clone(), input.clone(), out.clone(), span))
                .collect::<Result<Rc<[V]>, _>>()?;
            Dense::pack(xs.shape.clone(), cells, span)
        }
        (ArrayZip | ArrayOuter, [f, a, b], [_, ta, tb]) => pairwise(
            op == ArrayOuter,
            f,
            (dense(a, span)?, element(ta, span)?),
            (dense(b, span)?, element(tb, span)?),
            element(output, span)?,
            eval,
            span,
        ),
        (ArrayRankMap, [rank, f, xs], [_, _, t]) => rankmap(
            exact_index(rank).map_err(|e| error(span, e))?,
            f,
            dense(xs, span)?,
            t,
            element(output, span)?,
            eval,
            span,
        ),
        (Transpose, [xs], [_]) => {
            let input = dense(xs, span)?;
            let permutation: Vec<_> = (0..input.shape.len()).rev().collect();
            algebra::reorder(input, &permutation, span)
        }
        (ArrayAxes, [permutation, xs], [_, _]) => {
            let indices = as_seq(permutation).map_err(|e| invariant(span, e))?;
            let input = dense(xs, span)?;
            if indices.len() != input.shape.len() {
                return Err(error(
                    span,
                    "arr.axes requires a complete one-based axis permutation",
                ));
            }
            let indices = indices
                .iter()
                .map(|index| exact_index(index).map_err(|e| error(span, e)))
                .collect::<R<Vec<_>>>()?;
            algebra::axes(&indices, input, span)
        }
        (ArraySection, [starts, counts, xs], [_, _, _]) => {
            let starts = shape(starts, span)?;
            let counts = shape(counts, span)?;
            algebra::section(&starts, &counts, dense(xs, span)?, span)
        }
        (Sum | Product | Max | Min | All | Any, [xs], [_]) => {
            let input = dense(xs, span)?;
            let kernel = LaneOp::Intrinsic(op, element(output, span)?);
            algebra::along_axis(input, last_axis(input, span)?, kernel, eval, span)
        }
        (Reduce, [f, xs], [_, t]) | (ArrayReduce, [_, f, xs], [_, _, t]) => {
            let input = dense(xs, span)?;
            let axis = if op == ArrayReduce {
                axis(&args[0], input, span)?
            } else {
                last_axis(input, span)?
            };
            let kernel = LaneOp::Reduce {
                callback: f,
                item: element(t, span)?,
            };
            algebra::along_axis(input, axis, kernel, eval, span)
        }
        (Fold | Scan | ArrayScan, [f, seed, xs], [_, _, t])
        | (ArrayFold | ArrayScanAxis, [_, f, seed, xs], [_, _, _, t]) => {
            let input = dense(xs, span)?;
            let explicit_axis = matches!(op, ArrayFold | ArrayScanAxis);
            let axis = if explicit_axis {
                axis(&args[0], input, span)?
            } else {
                last_axis(input, span)?
            };
            let kernel = LaneOp::Seeded {
                callback: f,
                seed,
                item: element(t, span)?,
                accumulator: element(output, span)?,
                scan: matches!(op, Scan | ArrayScan | ArrayScanAxis),
            };
            algebra::along_axis(input, axis, kernel, eval, span)
        }
        (ArrayDot, [a, b], [_, _]) => {
            algebra::dot(dense(a, span)?, dense(b, span)?, output, eval, span)
        }
        (ArrayMatMul, [a, b], [_, _]) => algebra::matmul(
            dense(a, span)?,
            dense(b, span)?,
            element(output, span)?,
            eval,
            span,
        ),
        (ArrayDiag, [a], [_]) => algebra::diag(dense(a, span)?, span),
        _ => Err(invariant(span, "invalid checked array operands")),
    }
}
