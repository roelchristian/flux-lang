//! Seeded intrinsic reductions and optional extrema.
use super::{
    array, compact_exact, error, real_binary, resources, CoreOp, Dense, Fast, NumericResult,
    Packed, Prim, Rc, Reader, Source, Span, Type, V,
};

fn lanes<T: Packed, U>(
    input: &Dense,
    axis: usize,
    shape: &[usize],
    mut lane: impl FnMut(&Reader<'_, T>, usize, usize, usize) -> NumericResult<Option<U>>,
) -> Option<NumericResult<Vec<U>>> {
    let (len, width) = match array::checked_volume(shape) {
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
    let len = match array::checked_volume(&output) {
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
    let len = match array::checked_volume(&output) {
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

pub(in crate::hir_eval) fn intrinsic(
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
