//! Packed fold and scan for eligible builtin callbacks.
use super::{
    array, compact_exact, error, real_binary, resources, CoreOp, Dense, Eval, Fast, Function,
    NumericResult, Packed, Prim, Rc, Reader, Source, Span, Type, V,
};

pub(in crate::hir_eval) struct ScanPlan<'a> {
    input: &'a Dense,
    axis: usize,
    output: Rc<[usize]>,
    scan: bool,
    span: Span,
}

impl<'a> ScanPlan<'a> {
    pub(in crate::hir_eval) fn new(
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
    let len = match array::checked_volume(&output) {
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
pub(in crate::hir_eval) fn seeded(
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
