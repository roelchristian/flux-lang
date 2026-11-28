//! Packed scalar, pointwise and contraction kernels. The checked operation,
//! result type and lift mode choose the algorithm; storage may only decline
//! the fast path. There is no arithmetic reassociation or implicit FMA.
use super::{arrays012::Dense, resources::MemoryReservation, *};

type View<'a, T> = (&'a [T], &'a [usize], usize);
type Fast<T> = Option<R<T>>;
const MAX_RANK: usize = 16;

trait Packed: Copy {
    fn scalar(v: &V) -> Option<Self>;
    fn view(d: &Dense) -> Option<View<'_, Self>>;
    fn slice(d: &Dense) -> Option<&[Self]>;
    fn value(self) -> V;
    fn pack(shape: Rc<[usize]>, cells: Vec<Self>, lease: MemoryReservation, span: Span) -> R<V>;
}

macro_rules! packed {
    ($t:ty, $view:ident, $slice:ident, $pack:ident, $scalar:expr, $value:expr) => {
        impl Packed for $t {
            fn scalar(v: &V) -> Option<Self> {
                ($scalar)(v)
            }

            fn view(d: &Dense) -> Option<View<'_, Self>> {
                d.$view()
            }

            fn slice(d: &Dense) -> Option<&[Self]> {
                d.$slice()
            }

            fn value(self) -> V {
                ($value)(self)
            }

            fn pack(
                shape: Rc<[usize]>,
                cells: Vec<Self>,
                lease: MemoryReservation,
                span: Span,
            ) -> R<V> {
                Dense::$pack(shape, cells, lease, span)
            }
        }
    };
}

packed!(
    f64,
    real_view,
    real_slice,
    pack_real_reserved,
    |v: &V| match v {
        V::Real(r) => Some(*r),
        _ => None,
    },
    V::Real
);
packed!(
    i64,
    int_view,
    int_slice,
    pack_int_reserved,
    |v: &V| match v {
        V::Exact(n) if n.d == 1 && matches!(n.domain, Domain::Integer | Domain::Natural) =>
            i64::try_from(n.n).ok(),
        _ => None,
    },
    |x| exact_i128(i128::from(x), Domain::Integer)
);
packed!(
    u64,
    nat_view,
    nat_slice,
    pack_nat_reserved,
    |v: &V| match v {
        V::Exact(n) if n.d == 1 && n.domain == Domain::Natural => u64::try_from(n.n).ok(),
        _ => None,
    },
    |x| exact_i128(i128::from(x), Domain::Natural)
);
packed!(
    bool,
    bool_view,
    bool_slice,
    pack_bool_reserved,
    |v: &V| match v {
        V::Bool(b) => Some(*b),
        _ => None,
    },
    V::Bool
);

fn shape(args: &[V], mode: LiftMode) -> NumericResult<Option<Rc<[usize]>>> {
    match (mode, args) {
        (LiftMode::LeftArray, [V::Dense(a), ..]) => Ok(Some(Rc::from(a.shape()))),
        (LiftMode::RightArray, [_, V::Dense(b)]) => Ok(Some(Rc::from(b.shape()))),
        (LiftMode::ArrayPair, [V::Dense(a), V::Dense(b)]) => {
            arrays012::broadcast(a.shape(), b.shape()).map(Some)
        }
        (LiftMode::LeftArray | LiftMode::RightArray | LiftMode::ArrayPair, _) => {
            Err("expected a checked dense array operand")
        }
        _ => Ok(None),
    }
}

pub(super) fn dense_work(args: &[V], mode: Option<LiftMode>) -> NumericResult<Option<usize>> {
    match mode {
        Some(mode) => shape(args, mode)?
            .map(|shape| arrays012::checked_volume(&shape))
            .transpose(),
        None => Ok(None),
    }
}

enum Source<'a, T> {
    Scalar(T),
    Array(&'a Dense),
}

impl<'a, T: Packed> Source<'a, T> {
    fn new(v: &'a V, array: bool) -> Option<Self> {
        if array {
            let V::Dense(d) = v else { return None };
            // Only the requested representation is accepted.
            T::view(d)?;
            Some(Self::Array(d))
        } else {
            T::scalar(v).map(Self::Scalar)
        }
    }
}

enum Reader<'a, T> {
    Constant(T),
    Linear(&'a [T]),
    Strided {
        buffer: &'a [T],
        offset: usize,
        strides: [usize; MAX_RANK],
    },
}
impl<'a, T: Packed> Reader<'a, T> {
    fn new(source: Source<'a, T>, output: &[usize]) -> Option<Self> {
        let array = match source {
            Source::Scalar(value) => return Some(Self::Constant(value)),
            Source::Array(array) => array,
        };
        if array.shape() == output {
            if let Some(slice) = T::slice(array) {
                return Some(Self::Linear(slice));
            }
        }
        let (buffer, physical, offset) = T::view(array)?;
        let mut strides = [0; MAX_RANK];
        let pad = output.len() - array.shape().len();
        for (i, (&width, &stride)) in array.shape().iter().zip(physical).enumerate() {
            if width != 1 {
                strides[pad + i] = stride;
            }
        }
        Some(Self::Strided {
            buffer,
            offset,
            strides,
        })
    }

    fn at(&self, mut index: usize, shape: &[usize]) -> T {
        match self {
            Self::Constant(x) => *x,
            Self::Linear(values) => values[index],
            Self::Strided {
                buffer,
                offset,
                strides,
            } => {
                let mut physical = *offset;
                for (&width, &stride) in shape.iter().zip(strides).rev() {
                    physical += (index % width) * stride;
                    index /= width;
                }
                buffer[physical]
            }
        }
    }
}

fn pointwise<T: Packed, U: Packed>(
    args: &[V],
    mode: LiftMode,
    span: Span,
    mut f: impl FnMut(T, Option<T>) -> NumericResult<Option<U>>,
) -> Fast<V> {
    let dims = match shape(args, mode) {
        Ok(Some(shape)) => shape,
        Ok(None) => return None,
        Err(message) => return Some(Err(error(span, message))),
    };
    let (a, b) = match args {
        [a] => (a, None),
        [a, b] => (a, Some(b)),
        _ => return None,
    };
    let left = Reader::new(
        Source::new(a, matches!(mode, LiftMode::LeftArray | LiftMode::ArrayPair))?,
        &dims,
    )?;
    let right = if let Some(b) = b {
        Some(Reader::new(
            Source::new(
                b,
                matches!(mode, LiftMode::RightArray | LiftMode::ArrayPair),
            )?,
            &dims,
        )?)
    } else {
        None
    };
    let len = match arrays012::checked_volume(&dims) {
        Ok(n) => n,
        Err(message) => return Some(Err(error(span, message))),
    };
    let lease = match resources::reserve_for::<U>(len) {
        Ok(lease) => lease,
        Err(message) => return Some(Err(error(span, message))),
    };
    let mut values = Vec::with_capacity(len);
    for i in 0..len {
        match f(left.at(i, &dims), right.as_ref().map(|r| r.at(i, &dims))) {
            Ok(Some(value)) => values.push(value),
            Ok(None) => return None, // checked exact overflow: widen via generic HIR
            Err(message) => return Some(Err(error(span, message))),
        }
    }
    Some(U::pack(dims, values, lease, span))
}

fn finite_real(x: f64) -> NumericResult<f64> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err("nonfinite real result")
    }
}

fn real_binary(op: CoreOp, a: f64, b: f64) -> NumericResult<Option<f64>> {
    use CoreOp::*;
    finite_real(match op {
        Add => a + b,
        Sub => a - b,
        Mul => a * b,
        Div if b == 0.0 => return Err(DIV_BY_ZERO),
        Div => a / b,
        _ => return Ok(None),
    })
    .map(Some)
}

fn compare<T: PartialOrd>(op: CoreOp, a: T, b: T) -> NumericResult<Option<bool>> {
    use CoreOp::*;
    Ok(Some(match op {
        Equal => a == b,
        NotEqual => a != b,
        Lt => a < b,
        Le => a <= b,
        Gt => a > b,
        Ge => a >= b,
        _ => return Ok(None),
    }))
}

fn compact_exact() -> bool {
    // At stricter policies each intermediate, not only the final packed
    // value, must pass the generic arithmetic bit-limit checks.
    wide05::max_bits() >= u64::from(u64::BITS)
}

fn mode_if_dense(mode: Option<LiftMode>) -> Option<LiftMode> {
    mode.filter(|mode| {
        matches!(
            mode,
            LiftMode::LeftArray | LiftMode::RightArray | LiftMode::ArrayPair
        ) && resources::optimized()
    })
}

pub(super) fn binary(
    op: CoreOp,
    args: &[V],
    ty: &Type,
    mode: Option<LiftMode>,
    span: Span,
) -> Fast<V> {
    use CoreOp::*;
    let mode = mode_if_dense(mode)?;
    match (op, ty.array_element()?) {
        (Add | Sub | Mul | Div, Type::Prim(Prim::R)) => pointwise(args, mode, span, |a, b| {
            real_binary(op, a, b.expect("checked arity"))
        }),
        (Add | Sub | Mul, Type::Prim(Prim::Z)) if compact_exact() => {
            pointwise::<i64, i64>(args, mode, span, |a, b| {
                let b = b.expect("checked arity");
                Ok(match op {
                    Add => a.checked_add(b),
                    Sub => a.checked_sub(b),
                    Mul => a.checked_mul(b),
                    _ => unreachable!(),
                })
            })
        }
        (Add | Sub | Mul, Type::Prim(Prim::N)) if compact_exact() => {
            pointwise::<u64, u64>(args, mode, span, |a, b| {
                let b = b.expect("checked arity");
                Ok(match op {
                    Add => a.checked_add(b),
                    Sub => Some(a.checked_sub(b).ok_or("natural subtraction underflow")?),
                    Mul => a.checked_mul(b),
                    _ => unreachable!(),
                })
            })
        }
        (And | Or, Type::Prim(Prim::B)) => pointwise::<bool, bool>(args, mode, span, |a, b| {
            let b = b.expect("checked arity");
            Ok(Some(if op == And { a & b } else { a | b }))
        }),
        (Equal | NotEqual | Lt | Le | Gt | Ge, Type::Prim(Prim::B)) => {
            pointwise::<f64, bool>(args, mode, span, |a, b| {
                compare(op, a, b.expect("checked arity"))
            })
            .or_else(|| {
                pointwise::<i64, bool>(args, mode, span, |a, b| {
                    compare(op, a, b.expect("checked arity"))
                })
            })
            .or_else(|| {
                pointwise::<u64, bool>(args, mode, span, |a, b| {
                    compare(op, a, b.expect("checked arity"))
                })
            })
        }
        _ => None,
    }
}

pub(super) fn unary(
    op: CoreOp,
    args: &[V],
    ty: &Type,
    mode: Option<LiftMode>,
    span: Span,
) -> Fast<V> {
    use CoreOp::*;
    let mode = mode_if_dense(mode)?;
    match (op, ty.array_element()?) {
        (ToReal, Type::Prim(Prim::R)) => {
            // An integer cast rounds once to binary64, just as Exact::real
            // does when its denominator is one.
            pointwise(args, mode, span, |a: f64, _| finite_real(a).map(Some))
                .or_else(|| pointwise(args, mode, span, |a: i64, _| Ok(Some(a as f64))))
                .or_else(|| pointwise(args, mode, span, |a: u64, _| Ok(Some(a as f64))))
        }
        (Negate | Abs | MathExp | MathSin, Type::Prim(Prim::R)) => {
            pointwise(args, mode, span, |a: f64, _| {
                finite_real(match op {
                    Negate => -a,
                    Abs => a.abs(),
                    MathExp => libm::exp(a),
                    MathSin => libm::sin(a),
                    _ => unreachable!(),
                })
                .map(Some)
            })
        }
        (Negate | Abs, Type::Prim(Prim::Z)) if compact_exact() => {
            pointwise(args, mode, span, |a: i64, _| {
                Ok(if op == Negate {
                    a.checked_neg()
                } else {
                    a.checked_abs()
                })
            })
            .or_else(|| {
                if op == Negate {
                    pointwise(args, mode, span, |a: u64, _| {
                        Ok(i64::try_from(-i128::from(a)).ok())
                    })
                } else {
                    None
                }
            })
        }
        (Abs, Type::Prim(Prim::N)) if compact_exact() => {
            pointwise(args, mode, span, |a: u64, _| Ok(Some(a)))
        }
        (Not, Type::Prim(Prim::B)) => pointwise(args, mode, span, |a: bool, _| Ok(Some(!a))),
        _ => None,
    }
}

fn lanes<T: Packed, U>(
    input: &Dense,
    axis: usize,
    shape: &[usize],
    mut lane: impl FnMut(&Reader<'_, T>, usize, usize, usize) -> NumericResult<Option<U>>,
) -> Option<NumericResult<Vec<U>>> {
    let (len, width) = match arrays012::checked_volume(shape) {
        Ok(n) => (n, input.shape()[axis]),
        Err(message) => return Some(Err(message)),
    };
    // The caller established the nonempty output volume before selecting a kernel.
    let stride = input.shape()[axis + 1..].iter().product::<usize>();
    let reader = Reader::new(Source::<T>::Array(input), input.shape())?;
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let base = (i / stride) * (width * stride) + i % stride;
        match lane(&reader, base, stride, width) {
            Ok(Some(value)) => result.push(value),
            Ok(None) => return None,
            Err(message) => return Some(Err(message)),
        }
    }
    Some(Ok(result))
}

fn reduce<T: Packed>(
    input: &Dense,
    axis: usize,
    output: Rc<[usize]>,
    start: T,
    span: Span,
    mut fold: impl FnMut(T, T) -> NumericResult<Option<T>>,
) -> Fast<V> {
    T::view(input)?;
    let len = match arrays012::checked_volume(&output) {
        Ok(n) => n,
        Err(message) => return Some(Err(error(span, message))),
    };
    let lease = match resources::reserve_for::<T>(len) {
        Ok(lease) => lease,
        Err(message) => return Some(Err(error(span, message))),
    };
    let values = lanes(input, axis, &output, |reader, base, stride, width| {
        let mut acc = start;
        for i in 0..width {
            let x = reader.at(base + i * stride, input.shape());
            match fold(acc, x)? {
                Some(next) => acc = next,
                None => return Ok(None),
            }
        }
        Ok(Some(acc))
    })?
    .map_err(|message| error(span, message));
    Some(values.and_then(|values| T::pack(output, values, lease, span)))
}

fn extremum<T: Packed + PartialOrd>(
    input: &Dense,
    axis: usize,
    output: Rc<[usize]>,
    min: bool,
    span: Span,
) -> Fast<V> {
    T::view(input)?;
    let len = match arrays012::checked_volume(&output) {
        Ok(n) => n,
        Err(message) => return Some(Err(error(span, message))),
    };
    let lease = match resources::reserve_for::<V>(len) {
        Ok(lease) => lease,
        Err(message) => return Some(Err(error(span, message))),
    };
    let values = lanes::<T, V>(input, axis, &output, |reader, base, stride, width| {
        if width == 0 {
            return Ok(Some(V::Optional(None)));
        }
        let mut best = reader.at(base, input.shape());
        for i in 1..width {
            let x = reader.at(base + i * stride, input.shape());
            if (min && x < best) || (!min && x > best) {
                best = x;
            }
        }
        Ok(Some(V::Optional(Some(Rc::new(best.value())))))
    })?
    .map_err(|message| error(span, message));
    Some(values.and_then(|values| Dense::pack_values_reserved(output, values, lease, span)))
}

pub(super) fn intrinsic(
    input: &Dense,
    axis: usize,
    output: Rc<[usize]>,
    op: CoreOp,
    ty: &Type,
    span: Span,
) -> Fast<V> {
    use CoreOp::*;
    if !resources::optimized() {
        return None;
    }
    match (op, ty) {
        (Sum | Product, Type::Prim(Prim::R)) => {
            let f = if op == Sum { Add } else { Mul };
            reduce(
                input,
                axis,
                output,
                f64::from(u8::from(op == Product)),
                span,
                |a, b| real_binary(f, a, b),
            )
        }
        (Sum | Product, Type::Prim(Prim::Z)) if compact_exact() => reduce(
            input,
            axis,
            output,
            i64::from(op == Product),
            span,
            |a, b| {
                Ok(if op == Sum {
                    a.checked_add(b)
                } else {
                    a.checked_mul(b)
                })
            },
        ),
        (Sum | Product, Type::Prim(Prim::N)) if compact_exact() => reduce(
            input,
            axis,
            output,
            u64::from(op == Product),
            span,
            |a, b| {
                Ok(if op == Sum {
                    a.checked_add(b)
                } else {
                    a.checked_mul(b)
                })
            },
        ),
        (All | Any, Type::Prim(Prim::B)) => reduce(input, axis, output, op == All, span, |a, b| {
            Ok(Some(if op == All { a & b } else { a | b }))
        }),
        (Min | Max, Type::Optional(element)) => match element.as_ref() {
            Type::Prim(Prim::R) => extremum::<f64>(input, axis, output, op == Min, span),
            Type::Prim(Prim::Z) => extremum::<i64>(input, axis, output, op == Min, span),
            Type::Prim(Prim::N) => extremum::<u64>(input, axis, output, op == Min, span),
            _ => None,
        },
        _ => None,
    }
}

trait Contract: Packed {
    const ZERO: Self;
    fn mac(sum: Self, left: Self, right: Self) -> NumericResult<Option<Self>>;
}
impl Contract for f64 {
    const ZERO: Self = 0.0;

    fn mac(sum: Self, left: Self, right: Self) -> NumericResult<Option<Self>> {
        let product = finite_real(left * right)?;
        finite_real(sum + product).map(Some)
    }
}
impl Contract for i64 {
    const ZERO: Self = 0;

    fn mac(sum: Self, left: Self, right: Self) -> NumericResult<Option<Self>> {
        Ok(left.checked_mul(right).and_then(|x| sum.checked_add(x)))
    }
}
impl Contract for u64 {
    const ZERO: Self = 0;

    fn mac(sum: Self, left: Self, right: Self) -> NumericResult<Option<Self>> {
        Ok(left.checked_mul(right).and_then(|x| sum.checked_add(x)))
    }
}

fn contraction<T: Contract>(
    left: impl Iterator<Item = T>,
    right: impl Iterator<Item = T>,
) -> NumericResult<Option<T>> {
    let mut sum = T::ZERO;
    for (a, b) in left.zip(right) {
        let Some(result) = T::mac(sum, a, b)? else {
            return Ok(None);
        };
        sum = result;
    }
    Ok(Some(sum))
}

#[derive(Clone, Copy)]
struct Lane<'a, T> {
    data: &'a [T],
    stride: usize,
}

impl<'a, T: Copy> Lane<'a, T> {
    fn new(data: &'a [T], start: usize, stride: usize) -> Self {
        Self {
            data: &data[start..],
            stride,
        }
    }

    fn slice(self, width: usize) -> Option<&'a [T]> {
        (self.stride == 1).then(|| &self.data[..width])
    }

    fn iter(self, width: usize) -> impl ExactSizeIterator<Item = T> + 'a {
        (0..width).map(move |i| self.data[i * self.stride])
    }
}

fn contract_lanes<T: Contract>(
    left: Lane<'_, T>,
    right: Lane<'_, T>,
    width: usize,
) -> NumericResult<Option<T>> {
    // Select the traversal outside the loop so unit-stride operands retain
    // the unboxed, bounds-check-free slice path of ordinary packed inputs.
    match (left.slice(width), right.slice(width)) {
        (Some(a), Some(b)) => contraction(a.iter().copied(), b.iter().copied()),
        (Some(a), None) => contraction(a.iter().copied(), right.iter(width)),
        (None, Some(b)) => contraction(left.iter(width), b.iter().copied()),
        (None, None) => contraction(left.iter(width), right.iter(width)),
    }
}

fn dot_packed<T: Contract>(left: &Dense, right: &Dense, span: Span) -> Fast<V> {
    let (a, [a_stride], a_start) = T::view(left)? else {
        return None;
    };
    let (b, [b_stride], b_start) = T::view(right)? else {
        return None;
    };
    if left.len() == 0 {
        return Some(Ok(T::ZERO.value()));
    }
    let a = Lane::new(a, a_start, *a_stride);
    let b = Lane::new(b, b_start, *b_stride);
    match contract_lanes(a, b, left.len()) {
        Ok(Some(value)) => Some(Ok(value.value())),
        Ok(None) => None,
        Err(message) => Some(Err(error(span, message))),
    }
}

pub(super) fn dot(left: &Dense, right: &Dense, ty: &Type, span: Span) -> Fast<V> {
    if !resources::optimized() {
        return None;
    }
    match ty {
        Type::Prim(Prim::R) => dot_packed::<f64>(left, right, span),
        Type::Prim(Prim::Z) if compact_exact() => dot_packed::<i64>(left, right, span),
        Type::Prim(Prim::N) if compact_exact() => dot_packed::<u64>(left, right, span),
        _ => None,
    }
}

fn matrix_packed<T: Contract>(
    left: &Dense,
    right: &Dense,
    shape: Rc<[usize]>,
    span: Span,
) -> Fast<V> {
    let &[m, k] = left.shape() else { return None };
    let &[_, n] = right.shape() else { return None };
    let right_len = right.len();
    let (left, &[left_row, left_col], left_start) = T::view(left)? else {
        return None;
    };
    let (right, &[right_row, right_col], right_start) = T::view(right)? else {
        return None;
    };
    let len = match arrays012::checked_volume(&shape) {
        Ok(len) => len,
        Err(message) => return Some(Err(error(span, message))),
    };
    let output = match resources::reserve_for::<T>(len) {
        Ok(token) => token,
        Err(message) => return Some(Err(error(span, message))),
    };
    if k == 0 {
        return Some(T::pack(shape, vec![T::ZERO; len], output, span));
    }
    // A column-contiguous view already provides the useful locality. Other
    // views can borrow a tracked copy, or traverse the original columns
    // directly if the memory allowance cannot accommodate scratch.
    let scratch = (m > 1 && n > 1 && k > 1 && right_row != 1)
        .then(|| resources::reserve_for::<T>(right_len).ok())
        .flatten();
    let columns = scratch.as_ref().map(|_| {
        let mut columns = Vec::with_capacity(right_len);
        for j in 0..n {
            columns.extend(Lane::new(right, right_start + j * right_col, right_row).iter(k));
        }
        columns
    });
    // A transposed or sectioned left operand may also have scattered rows.
    // Gather one row at a time only when the output can reuse it.
    let row_scratch = (n > 1 && k > 1 && left_col != 1)
        .then(|| resources::reserve_for::<T>(k).ok())
        .flatten();
    let mut row_values = row_scratch.as_ref().map(|_| Vec::with_capacity(k));
    let mut values = Vec::with_capacity(len);
    for i in 0..m {
        let row = Lane::new(left, left_start + i * left_row, left_col);
        let row = if let Some(ref mut cells) = row_values {
            cells.clear();
            cells.extend(row.iter(k));
            Lane::new(cells, 0, 1)
        } else {
            row
        };
        for j in 0..n {
            let column = if let Some(columns) = columns.as_ref() {
                Lane::new(columns, j * k, 1)
            } else {
                Lane::new(right, right_start + j * right_col, right_row)
            };
            match contract_lanes(row, column, k) {
                Ok(Some(value)) => values.push(value),
                Ok(None) => return None,
                Err(message) => return Some(Err(error(span, message))),
            }
        }
    }
    Some(T::pack(shape, values, output, span))
}

pub(super) fn matmul(
    left: &Dense,
    right: &Dense,
    output: Rc<[usize]>,
    ty: &Type,
    span: Span,
) -> Fast<V> {
    if !resources::optimized() {
        return None;
    }
    match ty {
        Type::Prim(Prim::R) => matrix_packed::<f64>(left, right, output, span),
        Type::Prim(Prim::Z) if compact_exact() => matrix_packed::<i64>(left, right, output, span),
        Type::Prim(Prim::N) if compact_exact() => matrix_packed::<u64>(left, right, output, span),
        _ => None,
    }
}

pub(super) struct ScanPlan<'a> {
    input: &'a Dense,
    axis: usize,
    output: Rc<[usize]>,
    scan: bool,
    span: Span,
}

impl<'a> ScanPlan<'a> {
    pub(super) fn new(
        input: &'a Dense,
        axis: usize,
        output: Rc<[usize]>,
        scan: bool,
        span: Span,
    ) -> Self {
        Self {
            input,
            axis,
            output,
            scan,
            span,
        }
    }
}

fn seeded_packed<T: Packed>(
    plan: ScanPlan<'_>,
    seed: &V,
    mut step: impl FnMut(T, T) -> NumericResult<Option<T>>,
) -> Fast<V> {
    let ScanPlan {
        input,
        axis,
        output,
        scan,
        span,
    } = plan;
    let seed = T::scalar(seed)?;
    let input_shape = input.shape();
    let reader = Reader::new(Source::<T>::Array(input), input_shape)?;
    let len = match arrays012::checked_volume(&output) {
        Ok(n) => n,
        Err(message) => return Some(Err(error(span, message))),
    };
    let width = input_shape[axis];
    let stride = input_shape[axis + 1..].iter().product::<usize>();
    let lanes = if scan { len / (width + 1) } else { len };
    let lease = match resources::reserve_for::<T>(len) {
        Ok(lease) => lease,
        Err(message) => return Some(Err(error(span, message))),
    };
    let mut values = if scan && stride != 1 {
        vec![seed; len]
    } else {
        Vec::with_capacity(len)
    };
    let emit = |values: &mut Vec<T>, index, value| {
        if scan && stride != 1 {
            values[index] = value;
        } else {
            values.push(value);
        }
    };
    for lane in 0..lanes {
        let outer = (lane / stride) * stride;
        let within = lane % stride;
        let source = outer * width + within;
        let destination = outer * (width + 1) + within;
        let mut acc = seed;
        if scan {
            emit(&mut values, destination, acc);
        }
        for i in 0..width {
            let x = reader.at(source + i * stride, input_shape);
            acc = match step(acc, x) {
                Ok(Some(x)) => x,
                Ok(None) => return None,
                Err(message) => return Some(Err(error(span, message))),
            };
            if scan {
                emit(&mut values, destination + (i + 1) * stride, acc);
            }
        }
        if !scan {
            values.push(acc);
        }
    }
    Some(T::pack(output, values, lease, span))
}

/// The generic path charges two unary applications plus a core dispatch per
/// callback. A recognized unbound primitive can charge that same total only
/// on success. If headroom is short, replay generic to preserve whether work
/// exhaustion or an earlier numeric error comes first.
pub(super) fn seeded(
    plan: ScanPlan<'_>,
    types: (&Type, &Type),
    operands: (&V, &V),
    eval: &mut Eval<'_>,
) -> Fast<V> {
    use CoreOp::*;
    let (item, accumulator) = types;
    let (callback, seed) = operands;
    if !resources::optimized() || item != accumulator {
        return None;
    }
    let V::Fun(callback) = callback else {
        return None;
    };
    let Function::Core(op, ..) = callback.code else {
        return None;
    };
    if !callback.bound.is_empty() {
        return None;
    }
    let work = plan.input.len().checked_mul(3)?;
    let span = plan.span;
    if work > eval.fuel {
        return None;
    }
    let result = match (op, item) {
        (Add | Mul, Type::Prim(Prim::R)) => seeded_packed(plan, seed, |a, b| real_binary(op, a, b)),
        (Add | Mul, Type::Prim(Prim::Z)) if compact_exact() => {
            seeded_packed(plan, seed, |a: i64, b| {
                Ok(if op == Add {
                    a.checked_add(b)
                } else {
                    a.checked_mul(b)
                })
            })
        }
        (Add | Mul, Type::Prim(Prim::N)) if compact_exact() => {
            seeded_packed(plan, seed, |a: u64, b| {
                Ok(if op == Add {
                    a.checked_add(b)
                } else {
                    a.checked_mul(b)
                })
            })
        }
        (And | Or, Type::Prim(Prim::B)) => seeded_packed(plan, seed, |a: bool, b| {
            Ok(Some(if op == And { a & b } else { a | b }))
        }),
        _ => return None,
    }?;
    Some(result.and_then(|value| {
        eval.charge_work(work, span)?;
        Ok(value)
    }))
}

#[cfg(test)]
mod tests {
    use super::{resources::RuntimeOptions, *};

    #[test]
    fn packed_contractions_read_physical_offsets_with_or_without_scratch() {
        // 224 bytes cover the two 12-cell buffers plus the 4-cell result,
        // but neither of the optional copies. The larger budget admits both.
        for memory in [224, 1024] {
            let mut options = RuntimeOptions::default();
            options.resources.max_memory = memory;
            let _scope = resources::enter(options).unwrap();
            let span = Span::default();
            let packed = || {
                let value = Dense::pack(
                    Rc::from([3, 4]),
                    (1..=12).map(|x| V::Real(f64::from(x))).collect(),
                    span,
                );
                let Ok(V::Dense(dense)) = value else {
                    panic!("expected packed matrix")
                };
                dense
            };
            let view = |array: &Dense, shape, strides, offset| {
                let Ok(V::Dense(dense)) = array.view(shape, strides, offset, span) else {
                    panic!("expected strided view")
                };
                assert!(dense.real_slice().is_none());
                dense
            };
            let a = packed();
            let b = packed();
            let a_dot = view(&a, Rc::from([3]), Rc::from([5]), 1);
            let b_dot = view(&b, Rc::from([3]), Rc::from([4]), 2);
            let ty = Type::Prim(Prim::R);
            assert!(matches!(dot(&a_dot, &b_dot, &ty, span), Some(Ok(V::Real(x))) if x == 187.0));
            let a_matrix = view(&a, Rc::from([2, 3]), Rc::from([1, 4]), 1);
            let b_matrix = view(&b, Rc::from([3, 2]), Rc::from([4, 1]), 1);
            let result = matmul(&a_matrix, &b_matrix, Rc::from([2, 2]), &ty, span);
            let Some(Ok(V::Dense(result))) = result else {
                panic!("packed matrix contraction must work without optional scratch")
            };
            assert_eq!(result.real_slice().unwrap(), &[140.0, 158.0, 158.0, 179.0]);
            assert_eq!(resources::available_memory(), memory - 224);
        }
    }

    #[test]
    fn integer_to_real_kernels_pack_with_the_same_binary64_rounding() {
        let _scope = resources::enter(RuntimeOptions::default()).unwrap();
        let span = Span::default();
        let mut state = 0u64;
        // Include the halfway cases, and test thousands of deterministic
        // magnitudes and significands independently of the packed cast.
        let integers = [
            0,
            1,
            (1 << 53) - 1,
            (1 << 53) + 1,
            (1 << 53) + 3,
            i64::MAX as u64,
            1 << 63,
            u64::MAX,
        ]
        .into_iter()
        .chain((0..4096).map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            state
        }))
        .collect::<Vec<_>>();
        for domain in [Domain::Natural, Domain::Integer] {
            let cells: Rc<[V]> = integers
                .iter()
                .map(|&x| {
                    let number = if domain == Domain::Natural {
                        i128::from(x)
                    } else {
                        i128::from(x as i64)
                    };
                    exact_i128(number, domain)
                })
                .collect();
            let Ok(input) = Dense::pack(Rc::from([cells.len()]), cells.clone(), span) else {
                panic!("expected packed integer vector")
            };
            let output = unary(
                CoreOp::ToReal,
                &[input],
                &Type::dense_array(Type::Prim(Prim::R)),
                Some(LiftMode::LeftArray),
                span,
            );
            let Some(Ok(V::Dense(dense))) = output else {
                panic!("expected packed real conversion")
            };
            for (&actual, source) in dense.real_slice().unwrap().iter().zip(cells.iter()) {
                assert_eq!(actual.to_bits(), real_of(source).unwrap().to_bits());
            }
        }
    }
}
