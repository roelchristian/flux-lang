//! Checked packed pointwise operations and broadcast work.
use super::{
    array, compact_exact, error, finite_real, real_binary, resources, CoreOp, Fast, LiftMode,
    NumericResult, Packed, Prim, Rc, Reader, Source, Span, Type, V,
};

fn shape(args: &[V], mode: LiftMode) -> NumericResult<Option<Rc<[usize]>>> {
    match (mode, args) {
        (LiftMode::LeftArray, [V::Dense(a), ..]) => Ok(Some(Rc::from(a.shape()))),
        (LiftMode::RightArray, [_, V::Dense(b)]) => Ok(Some(Rc::from(b.shape()))),
        (LiftMode::ArrayPair, [V::Dense(a), V::Dense(b)]) => {
            array::broadcast(a.shape(), b.shape()).map(Some)
        }
        (LiftMode::LeftArray | LiftMode::RightArray | LiftMode::ArrayPair, _) => {
            Err("expected a checked dense array operand")
        }
        _ => Ok(None),
    }
}

pub(in crate::hir_eval) fn dense_work(
    args: &[V],
    mode: Option<LiftMode>,
) -> NumericResult<Option<usize>> {
    match mode {
        Some(mode) => shape(args, mode)?
            .map(|shape| array::checked_volume(&shape))
            .transpose(),
        None => Ok(None),
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
    let len = match array::checked_volume(&dims) {
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

fn mode_if_dense(mode: Option<LiftMode>) -> Option<LiftMode> {
    mode.filter(|mode| {
        matches!(
            mode,
            LiftMode::LeftArray | LiftMode::RightArray | LiftMode::ArrayPair
        ) && resources::optimized()
    })
}

pub(in crate::hir_eval) fn binary(
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

pub(in crate::hir_eval) fn unary(
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
