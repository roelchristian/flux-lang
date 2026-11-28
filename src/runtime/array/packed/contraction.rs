//! Packed dot products and matrix contractions.
use super::{
    array, compact_exact, error, finite_real, resources, Dense, Fast, NumericResult, Packed, Prim,
    Rc, Span, Type, V,
};

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

pub(in crate::hir_eval) fn dot(left: &Dense, right: &Dense, ty: &Type, span: Span) -> Fast<V> {
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
    let len = match array::checked_volume(&shape) {
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

pub(in crate::hir_eval) fn matmul(
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
