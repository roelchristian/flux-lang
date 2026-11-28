//! Checked shared dense storage and row-major logical views.
use super::{resources, *};

#[path = "algebra.rs"]
mod algebra;
use algebra::{axis, last_axis, LaneOp};

const MAX_RANK: usize = resources::RANK_CEILING;

enum Storage {
    Shared(Rc<[V]>),
    Generic(Vec<V>),
    Bool(Vec<bool>),
    Int(Vec<i64>),
    Nat(Vec<u64>),
    Real(Vec<f64>),
}

impl Storage {
    fn len(&self) -> usize {
        match self {
            Self::Shared(x) => x.len(),
            Self::Generic(x) => x.len(),
            Self::Bool(x) => x.len(),
            Self::Int(x) => x.len(),
            Self::Nat(x) => x.len(),
            Self::Real(x) => x.len(),
        }
    }

    fn value(&self, index: usize) -> V {
        match self {
            Self::Shared(x) => x[index].clone(),
            Self::Generic(x) => x[index].clone(),
            Self::Bool(x) => V::Bool(x[index]),
            Self::Int(x) => exact_i128(i128::from(x[index]), Domain::Integer),
            Self::Nat(x) => exact_i128(i128::from(x[index]), Domain::Natural),
            Self::Real(x) => V::Real(x[index]),
        }
    }
}

struct Buffer {
    data: Storage,
    _reservation: resources::MemoryReservation,
}

#[derive(Clone)]
pub(super) struct Dense {
    shape: Rc<[usize]>,
    strides: Rc<[usize]>,
    offset: usize,
    len: usize,
    contiguous: bool,
    buffer: Rc<Buffer>,
}

macro_rules! compact_access {
    ($slice:ident, $view:ident, $reserved:ident, $variant:ident, $ty:ty) => {
        pub(super) fn $view(&self) -> Option<(&[$ty], &[usize], usize)> {
            if let Storage::$variant(cells) = &self.buffer.data {
                Some((cells, &self.strides, self.offset))
            } else {
                None
            }
        }

        pub(super) fn $slice(&self) -> Option<&[$ty]> {
            if self.contiguous {
                let (cells, _, offset) = self.$view()?;
                cells.get(offset..offset + self.len)
            } else {
                None
            }
        }

        pub(super) fn $reserved(
            shape: Rc<[usize]>,
            cells: Vec<$ty>,
            reservation: resources::MemoryReservation,
            span: Span,
        ) -> R<V> {
            Self::owned(shape, Storage::$variant(cells), reservation)
                .map_err(|message| error(span, message))
        }
    };
}

impl Dense {
    compact_access!(real_slice, real_view, pack_real_reserved, Real, f64);
    compact_access!(int_slice, int_view, pack_int_reserved, Int, i64);
    compact_access!(nat_slice, nat_view, pack_nat_reserved, Nat, u64);
    compact_access!(bool_slice, bool_view, pack_bool_reserved, Bool, bool);

    pub(super) fn shape(&self) -> &[usize] {
        &self.shape
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    fn logical_offset(&self, mut index: usize) -> usize {
        debug_assert!(index < self.len);
        if self.contiguous {
            return self.offset + index;
        }
        let mut offset = self.offset;
        for (&width, &stride) in self.shape.iter().zip(self.strides.iter()).rev() {
            offset += (index % width) * stride;
            index /= width;
        }
        offset
    }

    pub(super) fn value(&self, index: usize) -> V {
        self.buffer.data.value(self.logical_offset(index))
    }

    pub(super) fn get(&self, index: usize) -> Option<V> {
        (index < self.len).then(|| self.value(index))
    }

    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = V> + '_ {
        (0..self.len).map(|index| self.value(index))
    }

    pub(super) fn equal(&self, other: &Self) -> bool {
        self.shape == other.shape && self.iter().zip(other.iter()).all(|(a, b)| equal(&a, &b))
    }

    pub(super) fn render(&self, show: impl Fn(&V) -> String) -> String {
        let shape = self
            .shape
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let cells = self.iter().map(|v| show(&v)).collect::<Vec<_>>().join(" ");
        format!("array [{shape}] [{cells}]")
    }

    fn owned(
        shape: Rc<[usize]>,
        data: Storage,
        reservation: resources::MemoryReservation,
    ) -> NumericResult<V> {
        let len = checked_volume(&shape)?;
        if len != data.len() {
            return Err("array shape does not match the number of elements");
        }
        Ok(V::Dense(Rc::new(Self {
            strides: contiguous_strides(&shape, len),
            shape,
            offset: 0,
            len,
            contiguous: true,
            buffer: Rc::new(Buffer {
                data,
                _reservation: reservation,
            }),
        })))
    }

    // The checked HIR element type determines the operation; storage merely
    // compresses values whose tags and exact representation are unchanged.
    fn packed(shape: Rc<[usize]>, cells: Rc<[V]>) -> NumericResult<V> {
        let len = checked_volume(&shape)?;
        if len != cells.len() {
            return Err("array shape does not match the number of elements");
        }
        if resources::optimized() {
            macro_rules! pack_if_homogeneous {
                ($variant:ident, $ty:ty, $f:expr) => {{
                    let value = $f;
                    if cells.iter().all(|cell| value(cell).is_some()) {
                        let reservation = resources::reserve_for::<$ty>(len)?;
                        let data = cells.iter().map(|cell| value(cell).unwrap()).collect();
                        return Self::owned(shape, Storage::$variant(data), reservation);
                    }
                }};
            }
            match cells.first() {
                Some(V::Bool(_)) => pack_if_homogeneous!(Bool, bool, |v: &V| match v {
                    V::Bool(x) => Some(*x),
                    _ => None,
                }),
                Some(V::Real(_)) => pack_if_homogeneous!(Real, f64, |v: &V| match v {
                    V::Real(x) => Some(*x),
                    _ => None,
                }),
                Some(V::Exact(Exact {
                    domain: Domain::Integer,
                    ..
                })) => {
                    pack_if_homogeneous!(Int, i64, |v: &V| match v {
                        V::Exact(Exact {
                            n,
                            d: 1,
                            domain: Domain::Integer,
                        }) => i64::try_from(*n).ok(),
                        _ => None,
                    })
                }
                Some(V::Exact(Exact {
                    domain: Domain::Natural,
                    ..
                })) => {
                    pack_if_homogeneous!(Nat, u64, |v: &V| match v {
                        V::Exact(Exact {
                            n,
                            d: 1,
                            domain: Domain::Natural,
                        }) => u64::try_from(*n).ok(),
                        _ => None,
                    })
                }
                _ => {}
            }
        }
        let reservation = resources::reserve_for::<V>(len)?;
        Self::owned(shape, Storage::Shared(cells), reservation)
    }

    pub(super) fn pack(shape: Rc<[usize]>, cells: Rc<[V]>, span: Span) -> R<V> {
        Self::packed(shape, cells).map_err(|message| error(span, message))
    }

    pub(super) fn pack_values_reserved(
        shape: Rc<[usize]>,
        cells: Vec<V>,
        reservation: resources::MemoryReservation,
        span: Span,
    ) -> R<V> {
        Self::owned(shape, Storage::Generic(cells), reservation)
            .map_err(|message| error(span, message))
    }

    pub(super) fn view(
        &self,
        shape: Rc<[usize]>,
        strides: Rc<[usize]>,
        offset: usize,
        span: Span,
    ) -> R<V> {
        let len = volume(&shape, span)?;
        if shape.len() != strides.len() {
            return Err(invariant(span, "dense view stride rank mismatch"));
        }
        let contiguous = if len == 0 {
            true
        } else {
            let end = shape
                .iter()
                .zip(strides.iter())
                .try_fold(offset, |x, (&n, &stride)| {
                    x.checked_add((n - 1).checked_mul(stride)?)
                });
            if end.is_none_or(|end| end >= self.buffer.data.len()) {
                return Err(invariant(span, "dense view exceeds its storage"));
            }
            let mut suffix = 1;
            shape.iter().zip(strides.iter()).rev().all(|(&n, &stride)| {
                let exact = n == 1 || stride == suffix;
                suffix *= n;
                exact
            })
        };
        Ok(V::Dense(Rc::new(Self {
            shape,
            strides,
            offset: if len == 0 { 0 } else { offset },
            len,
            contiguous,
            buffer: self.buffer.clone(),
        })))
    }

    fn reshape(&self, shape: Rc<[usize]>, span: Span) -> R<V> {
        let len = volume(&shape, span)?;
        if self.len != len {
            return Err(error(
                span,
                "array shape does not match the number of elements",
            ));
        }
        if self.contiguous {
            return self.view(
                shape.clone(),
                contiguous_strides(&shape, len),
                self.offset,
                span,
            );
        }

        macro_rules! copy_packed {
            ($variant:ident, $cells:ident, $ty:ty) => {{
                let reservation =
                    resources::reserve_for::<$ty>(len).map_err(|message| error(span, message))?;
                let data = (0..len)
                    .map(|index| $cells[self.logical_offset(index)])
                    .collect();
                Self::owned(shape, Storage::$variant(data), reservation)
                    .map_err(|message| error(span, message))
            }};
        }
        match &self.buffer.data {
            Storage::Real(cells) => copy_packed!(Real, cells, f64),
            Storage::Int(cells) => copy_packed!(Int, cells, i64),
            Storage::Nat(cells) => copy_packed!(Nat, cells, u64),
            Storage::Bool(cells) => copy_packed!(Bool, cells, bool),
            Storage::Shared(_) | Storage::Generic(_) => {
                let reservation =
                    resources::reserve_for::<V>(len).map_err(|message| error(span, message))?;
                Self::pack_values_reserved(shape, self.iter().collect(), reservation, span)
            }
        }
    }

    fn sequence(&self, span: Span) -> R<V> {
        if self.offset == 0 && self.contiguous && self.len == self.buffer.data.len() {
            if let Storage::Shared(cells) = &self.buffer.data {
                return Ok(V::Sequence(cells.clone()));
            }
        }
        // V::Sequence cannot represent a view or keep a temporary lease.
        // Admit its materialization (including the Vec -> Rc copy) before
        // visiting cells, without claiming to track ordinary sequence lifetime.
        let count = self
            .len
            .checked_mul(2)
            .ok_or_else(|| error(span, "array ravel exceeds addressable size"))?;
        let _scratch =
            resources::reserve_for::<V>(count).map_err(|message| error(span, message))?;
        let cells: Vec<_> = self.iter().collect();
        Ok(V::Sequence(cells.into()))
    }
}

fn contiguous_strides(shape: &[usize], len: usize) -> Rc<[usize]> {
    if len == 0 {
        // Empty logical views have no accessible storage offset, so no
        // intermediate products of potentially huge remaining axes.
        return vec![0; shape.len()].into();
    }
    let mut out = vec![0; shape.len()];
    let mut suffix = 1;
    for (i, &width) in shape.iter().enumerate().rev() {
        out[i] = suffix;
        suffix *= width; // checked_volume already bounded this product.
    }
    out.into()
}

fn volume(shape: &[usize], span: Span) -> R<usize> {
    checked_volume(shape).map_err(|message| error(span, message))
}

pub(super) fn checked_volume(shape: &[usize]) -> NumericResult<usize> {
    if shape.len() > MAX_RANK {
        return Err("array rank exceeds 16");
    }
    let limits = resources::limits();
    if shape.len() > limits.max_rank {
        return Err("array rank exceeds the configured limit");
    }
    if shape.iter().any(|&n| n > limits.max_elements) {
        return Err("array dimension exceeds the collection limit");
    }
    if shape.contains(&0) {
        return Ok(0);
    }
    shape.iter().try_fold(1usize, |size, &n| {
        size.checked_mul(n)
            .filter(|&product| product <= limits.max_elements)
            .ok_or("array element count exceeds the collection limit")
    })
}

fn shape(arg: &V, span: Span) -> R<Rc<[usize]>> {
    let xs = as_seq(arg).map_err(|e| invariant(span, e))?;
    if xs.len() > MAX_RANK {
        return Err(error(span, "array rank exceeds 16"));
    }
    if xs.len() > resources::limits().max_rank {
        return Err(error(span, "array rank exceeds the configured limit"));
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
        if arr.len() == 0 {
            return Ok(V::Optional(None));
        }
        off = off * dim + coord - 1;
    }
    Ok(V::Optional(arr.get(off).map(Rc::new)))
}

pub(super) fn broadcast(a: &[usize], b: &[usize]) -> NumericResult<Rc<[usize]>> {
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

fn collect_values<E>(
    len: usize,
    items: impl IntoIterator<Item = Result<V, E>>,
) -> Result<Vec<V>, E> {
    // Result's iterator collector loses the exact lower bound; reserve exactly
    // the amount already pre-admitted rather than growing a rounded capacity.
    let mut values = Vec::with_capacity(len);
    for item in items {
        values.push(item?);
    }
    Ok(values)
}

pub(super) fn lift_binary(
    a: V,
    b: V,
    mode: LiftMode,
    f: &impl Fn(V, V) -> NumericResult<V>,
) -> NumericResult<V> {
    match mode {
        LiftMode::LeftArray => {
            let a = expect_dense(&a)?;
            let reservation = resources::reserve_for::<V>(a.len())?;
            let cells = collect_values(a.len(), a.iter().map(|x| f(x, b.clone())))?;
            Dense::owned(a.shape.clone(), Storage::Generic(cells), reservation)
        }
        LiftMode::RightArray => {
            let b = expect_dense(&b)?;
            let reservation = resources::reserve_for::<V>(b.len())?;
            let cells = collect_values(b.len(), b.iter().map(|x| f(a.clone(), x)))?;
            Dense::owned(b.shape.clone(), Storage::Generic(cells), reservation)
        }
        LiftMode::ArrayPair => {
            let (a, b) = (expect_dense(&a)?, expect_dense(&b)?);
            let shape = broadcast(&a.shape, &b.shape)?;
            let len = checked_volume(&shape)?;
            let reservation = resources::reserve_for::<V>(len)?;
            let pairs = broadcast_indices(&shape, &a.shape, &b.shape, len);
            let cells = collect_values(len, pairs.map(|(i, j)| f(a.value(i), b.value(j))))?;
            Dense::owned(shape, Storage::Generic(cells), reservation)
        }
        _ => Err("invalid dense lifting mode in typed HIR"),
    }
}

pub(super) fn lift_unary(a: V, f: &impl Fn(V) -> NumericResult<V>) -> NumericResult<V> {
    let a = expect_dense(&a)?;
    let reservation = resources::reserve_for::<V>(a.len())?;
    let cells = collect_values(a.len(), a.iter().map(f))?;
    Dense::owned(a.shape.clone(), Storage::Generic(cells), reservation)
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
    let reservation = resources::reserve_for::<V>(len).map_err(|message| error(span, message))?;
    let mut cells = Vec::with_capacity(len);
    if outer {
        if len != 0 {
            for x in a.0.iter() {
                for y in b.0.iter() {
                    cells.push(eval.apply_dyadic(f, (x.clone(), a.1), (y, b.1), out, span)?);
                }
            }
        }
    } else {
        for (x, y) in broadcast_indices(&shape, &a.0.shape, &b.0.shape, len) {
            cells.push(eval.apply_dyadic(
                f,
                (a.0.value(x), a.1),
                (b.0.value(y), b.1),
                out,
                span,
            )?);
        }
    }
    Dense::pack_values_reserved(shape, cells, reservation, span)
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
    let reservation = resources::reserve_for::<V>(n).map_err(|message| error(span, message))?;
    let mut out = Vec::with_capacity(n);
    if n != 0 {
        let cell_shape: Rc<[usize]> = Rc::from(&input.shape[split..]);
        let cell_len = volume(&cell_shape, span)?;
        let cell_strides: Rc<[usize]> = Rc::from(&input.strides[split..]);
        for i in 0..n {
            let offset = if cell_len == 0 {
                0
            } else {
                input.logical_offset(i * cell_len)
            };
            let dense = input.view(cell_shape.clone(), cell_strides.clone(), offset, span)?;
            out.push(eval.apply_unary(
                f.clone(),
                dense,
                in_type.clone(),
                out_type.clone(),
                span,
            )?);
        }
    }
    Dense::pack_values_reserved(frame, out, reservation, span)
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
            if input.array_element().is_some() {
                dense(xs, span)?.reshape(dims, span)
            } else {
                Dense::pack(dims, seq(xs, span)?, span)
            }
        }
        (ArrayShape, [xs], [_]) => Ok(V::Sequence(
            dense(xs, span)?
                .shape
                .iter()
                .map(|&n| exact_i128(n as i128, Domain::Natural))
                .collect(),
        )),
        (Count, [xs], [_]) => Ok(exact_i128(dense(xs, span)?.len() as i128, Domain::Natural)),
        (ArrayRavel | Flatten, [xs], [_]) => dense(xs, span)?.sequence(span),
        (ArrayAt | Lookup, [coords, xs], [_, _]) => at(coords, dense(xs, span)?, span),
        (ArrayMap | Map, [f, xs], [_, t]) => {
            let xs = dense(xs, span)?;
            let input = element(t, span)?;
            let out = element(output, span)?;
            let reservation =
                resources::reserve_for::<V>(xs.len()).map_err(|message| error(span, message))?;
            let items = xs
                .iter()
                .map(|v| eval.apply_unary(f.clone(), v, input.clone(), out.clone(), span));
            let cells = collect_values(xs.len(), items)?;
            Dense::pack_values_reserved(xs.shape.clone(), cells, reservation, span)
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

#[cfg(test)]
mod storage_tests {
    use super::*;

    fn value(result: R<V>) -> V {
        result.map_err(Control::into_error).unwrap()
    }

    fn integers(dense: &Dense) -> Vec<i128> {
        dense
            .iter()
            .map(|v| match v {
                V::Exact(Exact {
                    n,
                    d: 1,
                    domain: Domain::Integer,
                }) => n,
                _ => panic!("expected a preserved integer tag"),
            })
            .collect()
    }

    #[test]
    fn views_share_one_lease_and_noncontiguous_reshape_copies_logical_order() {
        let _scope = resources::enter(resources::RuntimeOptions::default()).unwrap();
        let free = resources::available_memory();
        let base_value = value(Dense::pack(
            Rc::from([2, 3]),
            (1..=6).map(|n| exact_i128(n, Domain::Integer)).collect(),
            Span::default(),
        ));
        let base = expect_dense(&base_value).unwrap();
        assert_eq!(base.int_slice(), Some([1, 2, 3, 4, 5, 6].as_slice()));
        assert_eq!(resources::available_memory(), free - 6 * size_of::<i64>());

        let transposed = value(algebra::reorder(base, &[1, 0], Span::default()));
        let transposed = expect_dense(&transposed).unwrap();
        assert_eq!(transposed.shape(), &[3, 2]);
        assert_eq!(integers(transposed), [1, 4, 2, 5, 3, 6]);
        assert!(transposed.int_slice().is_none());

        let section = value(algebra::section(
            &[2, 1],
            &[2, 2],
            transposed,
            Span::default(),
        ));
        let section = expect_dense(&section).unwrap();
        assert!(Rc::ptr_eq(&base.buffer, &section.buffer));
        assert_eq!(integers(section), [2, 5, 3, 6]);
        assert_eq!(resources::available_memory(), free - 6 * size_of::<i64>());
        assert!(section.get(4).is_none());

        let reshaped = value(section.reshape(Rc::from([4]), Span::default()));
        let reshaped = expect_dense(&reshaped).unwrap();
        assert!(!Rc::ptr_eq(&base.buffer, &reshaped.buffer));
        assert_eq!(reshaped.int_slice(), Some([2, 5, 3, 6].as_slice()));
        assert_eq!(resources::available_memory(), free - 10 * size_of::<i64>());
        assert!(base
            .view(Rc::from([2]), Rc::from([usize::MAX]), 0, Span::default())
            .is_err());
    }

    #[test]
    fn generic_zero_volume_views_do_not_form_address_products() {
        let options = resources::RuntimeOptions {
            optimize: false,
            resources: resources::ResourcePolicy {
                max_elements: 10_000_000,
                ..resources::ResourcePolicy::default()
            },
        };
        let _scope = resources::enter(options).unwrap();
        let mut shape = [10_000_000; MAX_RANK];
        shape[5] = 0;
        let base_value = value(Dense::pack(Rc::from(shape), Rc::from([]), Span::default()));
        let base = expect_dense(&base_value).unwrap();
        let reversed: Vec<_> = (0..MAX_RANK).rev().collect();
        let reversed = value(algebra::reorder(base, &reversed, Span::default()));
        let reversed = expect_dense(&reversed).unwrap();
        shape.reverse();
        assert_eq!(reversed.shape(), &shape);
        assert_eq!(reversed.len(), 0);
        assert!(reversed.get(0).is_none());
        assert!(Rc::ptr_eq(&base.buffer, &reversed.buffer));
        assert!(
            matches!(value(reversed.sequence(Span::default())), V::Sequence(xs) if xs.is_empty())
        );
    }
}
