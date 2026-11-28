//! Packed array traversal for eligible builtin callbacks.
use super::{
    array, compact_exact, error, real_binary, resources, CoreOp, Dense, Eval, Fast, Function,
    LiftMode, NumericResult, Packed, Prim, Rc, Reader, Source, Span, Type, V,
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

fn unbound_primitive(callback: &V) -> Option<CoreOp> {
    let V::Fun(callback) = callback else {
        return None;
    };
    let Function::Core(op, ..) = callback.code else {
        return None;
    };
    callback.bound.is_empty().then_some(op)
}

fn unseeded_packed<T: Packed>(
    plan: ScanPlan<'_>,
    mut step: impl FnMut(T, T) -> NumericResult<Option<T>>,
) -> Fast<V> {
    let ScanPlan {
        input,
        axis,
        output,
        span,
        ..
    } = plan;
    let shape = input.shape();
    let width = shape[axis];
    let stride = shape[axis + 1..].iter().product::<usize>();
    let reader = Reader::new(Source::<T>::Array(input), shape)?;
    let lanes = input.len() / width;
    let lease = match resources::reserve_for::<V>(lanes) {
        Ok(lease) => lease,
        Err(message) => return Some(Err(error(span, message))),
    };
    let mut values = Vec::with_capacity(lanes);
    for lane in 0..lanes {
        let base = (lane / stride) * width * stride + lane % stride;
        let mut acc = reader.at(base, shape);
        for i in 1..width {
            // No callback has run yet. Replay generic on numeric failure or
            // widening with its original fuel and diagnostic ordering.
            let Ok(Some(next)) = step(acc, reader.at(base + i * stride, shape)) else {
                return None;
            };
            acc = next;
        }
        values.push(V::Optional(Some(Rc::new(acc.value()))));
    }
    Some(Dense::pack_values_reserved(output, values, lease, span))
}

/// Take each lane's first item as its accumulator, not a numeric identity.
/// Empty and singleton lanes stay with the generic zero-callback path.
pub(in crate::hir_eval) fn unseeded(
    plan: ScanPlan<'_>,
    item: &Type,
    callback: &V,
    eval: &mut Eval<'_>,
) -> Fast<V> {
    use CoreOp::*;
    let width = plan.input.shape()[plan.axis];
    if !resources::optimized() || width <= 1 {
        return None;
    }
    let op = unbound_primitive(callback)?;
    // Two checked applications and one scalar core tick per combination.
    // Bare core callbacks do not enter a user evaluation frame.
    let lanes = plan.input.len() / width;
    let work = (plan.input.len() - lanes).checked_mul(3)?;
    if work > eval.fuel {
        return None;
    }
    let span = plan.span;
    let result = match (op, item) {
        (Add | Mul, Type::Prim(Prim::R)) => unseeded_packed(plan, |a, b| real_binary(op, a, b)),
        (Add | Mul, Type::Prim(Prim::Z)) if compact_exact() => {
            unseeded_packed(plan, |a: i64, b| {
                Ok(if op == Add {
                    a.checked_add(b)
                } else {
                    a.checked_mul(b)
                })
            })
        }
        (Add | Mul, Type::Prim(Prim::N)) if compact_exact() => {
            unseeded_packed(plan, |a: u64, b| {
                Ok(if op == Add {
                    a.checked_add(b)
                } else {
                    a.checked_mul(b)
                })
            })
        }
        (And | Or, Type::Prim(Prim::B)) => unseeded_packed(plan, |a: bool, b| {
            Ok(Some(if op == And { a & b } else { a | b }))
        }),
        _ => return None,
    }?;
    Some(result.and_then(|value| {
        eval.charge_work(work, span)?;
        Ok(value)
    }))
}

/// Map a checked, unbound r->r primitive by reusing its packed unary kernel.
/// If that kernel declines, the generic evaluator invokes each callback once.
pub(in crate::hir_eval) fn mapped(
    input: &V,
    callback: &V,
    types: (&Type, &Type),
    eval: &mut Eval<'_>,
    span: Span,
) -> Fast<V> {
    let real = Type::Prim(Prim::R);
    let (item, output) = types;
    if !resources::optimized() || item != &real || output.array_element() != Some(&real) {
        return None;
    }
    let V::Dense(dense) = input else {
        return None;
    };
    let op = unbound_primitive(callback)?;
    if !matches!(
        op,
        CoreOp::Negate | CoreOp::Abs | CoreOp::MathSin | CoreOp::MathExp
    ) {
        return None;
    }
    // One checked application and one scalar core tick per cell.
    let work = dense.len().checked_mul(2)?;
    if work > eval.fuel {
        return None;
    }
    let value = super::unary(
        op,
        std::slice::from_ref(input),
        output,
        Some(LiftMode::LeftArray),
        span,
    )?
    .ok()?;
    Some(eval.charge_work(work, span).map(|()| value))
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
    let op = unbound_primitive(callback)?;
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
    use super::*;
    use crate::{
        semantic::{prepare, Mode, Policy},
        ResourcePolicy, RuntimeOptions,
    };

    #[test]
    fn packed_unseeded_keeps_first_element_sign_and_left_order() {
        let _scope = resources::enter(RuntimeOptions::default()).unwrap();
        let span = Span::default();
        let Ok(V::Dense(input)) = Dense::pack(
            Rc::from([3, 2]),
            [-0.0, 1e16, -0.0, 1.0, -0.0, -1e16]
                .into_iter()
                .map(V::Real)
                .collect(),
            span,
        ) else {
            panic!("expected dense input");
        };
        let plan = ScanPlan::new(&input, 0, Rc::from([2]), false, span);
        let Some(Ok(V::Dense(output))) =
            unseeded_packed(plan, |a, b| real_binary(CoreOp::Add, a, b))
        else {
            panic!("expected packed lane reduction");
        };
        for (actual, expected) in output.iter().zip([-0.0_f64, 0.0]) {
            let V::Optional(Some(item)) = actual else {
                panic!("expected lane accumulator");
            };
            let V::Real(actual) = *item else {
                panic!("expected real accumulator");
            };
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn primitive_callbacks_match_work_cutoffs_and_numeric_failure_spans() {
        let cases = [
            (
                r#"a := form [2n 2n] [-0.0r -0.0r 1r 2r];
!^{ ! print (,. (@/ &+ a), ,. (@: &-: a)); };"#,
                Ok("([?1 -0 ?1 3], [0 0 -1 -2])\n"),
            ),
            (
                r#"a := form [3n] [1e308r 1e308r 0r];
!^{ ! print ,. (@/ &+ a); };"#,
                Err("nonfinite"),
            ),
            (
                r#"<< "math";
a := form [3n] [0r 800r 1r];
!^{ ! print ,. (@: &math.exp a); };"#,
                Err("nonfinite"),
            ),
        ];
        for (source, expected) in cases {
            let program = crate::parser::parse(source).unwrap();
            let checked = prepare(&program, Mode::Script, Policy::TypedOnly).unwrap();
            let run = |optimize, max_work| {
                checked
                    .run_with_options(RuntimeOptions {
                        optimize,
                        resources: ResourcePolicy {
                            max_work,
                            ..Default::default()
                        },
                    })
                    .map(|out| (out.status, out.stdout))
            };
            for work in 1..80 {
                assert_eq!(run(true, work), run(false, work), "work={work}: {source}");
            }
            match expected {
                Ok(output) => assert_eq!(run(true, 1000).unwrap(), (0, output.to_owned())),
                Err(fragment) => assert!(run(true, 1000).unwrap_err().message.contains(fragment)),
            }
        }
    }
}
