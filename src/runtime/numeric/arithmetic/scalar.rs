//! Checked scalar primitives and their ordered reduction identities.
use super::{domain, exact_binary, exact_i128, Domain, NumericResult, DIV_BY_ZERO};
use crate::{
    hir::CoreOp,
    hir_eval::{as_bool, decimal, equal, order, wide, V},
    types::{Prim, Type},
};
use num_integer::Integer;
use num_traits::{ToPrimitive, Zero};

pub(in crate::hir_eval) fn finite(v: f64) -> NumericResult<V> {
    if v.is_finite() {
        Ok(V::Real(v))
    } else {
        Err("nonfinite real result")
    }
}

pub(in crate::hir_eval) fn scalar_binary(op: CoreOp, a: V, b: V, ty: &Type) -> NumericResult<V> {
    use CoreOp::*;
    Ok(match op {
        Add | Sub | Mul | Div | Quotient | Remainder | Pow => {
            if matches!(ty, Type::Prim(Prim::R)) {
                let r = |v| {
                    if let V::Real(x) = v {
                        Ok(x)
                    } else {
                        Err("expected a checked real operand")
                    }
                };
                if op == Pow {
                    let mut a = r(a)?;
                    let e = if let V::Exact(n) = &b {
                        u128::try_from(n.integral()?)
                            .ok()
                            .filter(|_| n.domain == Domain::Natural)
                            .ok_or("power requires a natural exponent")?
                    } else {
                        let e = wide::exponent(&b)?;
                        match a {
                            1.0 => return finite(1.0),
                            -1.0 => return finite(if e.is_odd() { -1.0 } else { 1.0 }),
                            x if x == 0.0 => {
                                return finite(if e.is_zero() {
                                    1.0
                                } else if e.is_odd() {
                                    x
                                } else {
                                    0.0
                                })
                            }
                            _ => {}
                        }
                        e.to_u128().ok_or(wide::RESOURCE)?
                    };
                    let mut exponent = e;
                    let mut value = 1.0;
                    while exponent > 0 {
                        if exponent & 1 == 1 {
                            value *= a;
                        }
                        exponent >>= 1;
                        if exponent != 0 {
                            a *= a;
                        }
                    }
                    return finite(value);
                }
                let (a, b) = (r(a)?, r(b)?);
                return finite(match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    Div if b != 0.0 => a / b,
                    Div => return Err(DIV_BY_ZERO),
                    _ => return Err("real operator not supported"),
                });
            }
            let k = domain(ty).ok_or("numeric domain is unresolved at typed execution")?;
            exact_binary(op, &a, &b, k)?
        }
        Equal => V::Bool(equal(&a, &b)),
        NotEqual => V::Bool(!equal(&a, &b)),
        Lt => V::Bool(order(&a, &b)?.is_lt()),
        Le => V::Bool(!order(&a, &b)?.is_gt()),
        Gt => V::Bool(order(&a, &b)?.is_gt()),
        Ge => V::Bool(!order(&a, &b)?.is_lt()),
        And => V::Bool(as_bool(&a)? & as_bool(&b)?),
        Or => V::Bool(as_bool(&a)? | as_bool(&b)?),
        _ => return Err("invalid binary operation in typed HIR"),
    })
}
pub(in crate::hir_eval) fn numeric_identity(ty: &Type, product: bool) -> NumericResult<V> {
    let identity = i128::from(product);
    if *ty == Type::Prim(Prim::R) {
        return Ok(V::Real(identity as f64));
    }
    let d = domain(ty).ok_or("reduction has no checked numeric domain")?;
    if d == Domain::Decimal {
        decimal::decimal_literal(if product { "1" } else { "0" })
    } else {
        Ok(exact_i128(identity, d))
    }
}

pub(in crate::hir_eval) fn numeric_reduce(
    op: CoreOp,
    items: impl IntoIterator<Item = V>,
    ty: &Type,
) -> NumericResult<V> {
    let product = op == CoreOp::Product;
    let choice = if product { CoreOp::Mul } else { CoreOp::Add };
    let mut accumulator = numeric_identity(ty, product)?;
    for item in items {
        accumulator = scalar_binary(choice, accumulator, item, ty)?;
    }
    Ok(accumulator)
}
