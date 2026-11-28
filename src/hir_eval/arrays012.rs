//! Checked row-major dense arrays. No shape is inferred from element values.
use super::*;

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
    if shape.len() > MAX_RANK {
        return Err(error(span, "array rank exceeds 16"));
    }
    if shape.iter().any(|&n| n > COLLECTION_LIMIT) {
        return Err(error(span, "array dimension exceeds the collection limit"));
    }
    if shape.contains(&0) {
        return Ok(0);
    }
    shape.iter().try_fold(1usize, |size, &n| {
        size.checked_mul(n)
            .filter(|&product| product <= COLLECTION_LIMIT)
            .ok_or_else(|| error(span, "array element count exceeds the collection limit"))
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

fn unary(eval: &mut Eval<'_>, f: &V, arg: V, ty: &Type, out: &Type, span: Span) -> R<V> {
    eval.apply_checked(
        f.clone(),
        vec![arg],
        Some(vec![ty.clone()]),
        Some(out.clone()),
        span,
    )
}

fn dyadic(
    eval: &mut Eval<'_>,
    f: &V,
    a: (V, &Type),
    b: (V, &Type),
    out: &Type,
    span: Span,
) -> R<V> {
    let residual = Type::Fun(Box::new(b.1.clone()), Box::new(out.clone()));
    let part = unary(eval, f, a.0, a.1, &residual, span)?;
    unary(eval, &part, b.0, b.1, out, span)
}

fn broadcast(a: &[usize], b: &[usize], span: Span) -> R<Rc<[usize]>> {
    let rank = a.len().max(b.len());
    let axis = |xs: &[usize], i: usize| i.checked_sub(rank - xs.len()).map_or(1, |j| xs[j]);
    (0..rank)
        .map(|i| {
            let (x, y) = (axis(a, i), axis(b, i));
            match (x, y) {
                (x, y) if x == y => Ok(x),
                (1, y) => Ok(y),
                (x, 1) => Ok(x),
                _ => Err(error(
                    span,
                    "incompatible trailing axes for array broadcasting",
                )),
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
        broadcast(&a.0.shape, &b.0.shape, span)?
    };
    let len = volume(&shape, span)?;
    let mut cells = Vec::with_capacity(len);
    if outer {
        if len != 0 {
            for x in a.0.cells.iter() {
                for y in b.0.cells.iter() {
                    cells.push(dyadic(
                        eval,
                        f,
                        (x.clone(), a.1),
                        (y.clone(), b.1),
                        out,
                        span,
                    )?);
                }
            }
        }
    } else if len != 0 {
        let ax = broadcast_strides(&a.0.shape, shape.len());
        let bx = broadcast_strides(&b.0.shape, shape.len());
        for offset in 0..len {
            let (mut x, mut y, mut index) = (0, 0, offset);
            for (i, &n) in shape.iter().enumerate().rev() {
                let coord = index % n;
                index /= n;
                x += ax[i] * coord;
                y += bx[i] * coord;
            }
            cells.push(dyadic(
                eval,
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
            out.push(unary(eval, f, dense, in_type, out_type, span)?);
        }
    }
    Dense::pack(frame, out.into(), span)
}

fn scan(
    f: &V,
    seed: &V,
    input: &Dense,
    item: &Type,
    acc: &Type,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    let (&width, frame) = input
        .shape
        .split_last()
        .ok_or_else(|| error(span, "array scan needs a last axis (rank zero has none)"))?;
    let mut shape = frame.to_vec();
    shape.push(width + 1);
    let n = volume(&shape, span)?;
    let mut out = Vec::with_capacity(n);
    // Output volume is zero when a leading axis is zero. An empty last
    // axis, on the other hand, contributes one seed per leading cell.
    for i in 0..n / (width + 1) {
        let mut current = seed.clone();
        out.push(current.clone());
        for value in &input.cells[i * width..(i + 1) * width] {
            current = dyadic(eval, f, (current, acc), (value.clone(), item), acc, span)?;
            out.push(current.clone());
        }
    }
    Dense::pack(shape.into(), out.into(), span)
}

pub(super) fn handles(op: CoreOp) -> bool {
    use CoreOp::*;
    matches!(
        op,
        ArrayReshape
            | ArrayShape
            | ArrayRavel
            | ArrayAt
            | ArrayMap
            | ArrayZip
            | ArrayOuter
            | ArrayRankMap
            | ArrayScan
    )
}

pub(super) fn apply(
    op: CoreOp,
    args: &[V],
    types: &[Type],
    output: &Type,
    eval: &mut Eval<'_>,
    span: Span,
) -> Option<R<V>> {
    handles(op).then(|| apply_dense(op, args, types, output, eval, span))
}

fn apply_dense(
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
        (ArrayShape, [xs], [_]) => Ok(V::Sequence(
            dense(xs, span)?
                .shape
                .iter()
                .map(|&n| exact_i128(n as i128, Domain::Natural))
                .collect(),
        )),
        (ArrayRavel, [xs], [_]) => Ok(V::Sequence(dense(xs, span)?.cells.clone())),
        (ArrayAt, [coords, xs], [_, _]) => at(coords, dense(xs, span)?, span),
        (ArrayMap, [f, xs], [_, t]) => {
            let xs = dense(xs, span)?;
            let input = element(t, span)?;
            let out = element(output, span)?;
            let cells = xs
                .cells
                .iter()
                .map(|v| unary(eval, f, v.clone(), input, out, span))
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
        (ArrayScan, [f, seed, xs], [_, _, t]) => scan(
            f,
            seed,
            dense(xs, span)?,
            element(t, span)?,
            element(output, span)?,
            eval,
            span,
        ),
        _ => Err(invariant(span, "invalid checked array operands")),
    }
}
