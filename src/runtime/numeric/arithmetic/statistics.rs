//! Exact and approximate ordered statistical reductions.
use super::{exact_binary, exact_i128, finite, real_of, Domain, NumericResult};
use crate::{
    hir::{CoreOp, Span},
    hir_eval::{as_seq, error, order, wide, R, V},
};
use std::{cmp::Ordering, rc::Rc};

pub(in crate::hir_eval) fn wide_standard_deviation(xs: &[V]) -> NumericResult<V> {
    let q = Domain::Rational;
    let origin = xs.first().ok_or("expected nonempty sequence")?;
    let count = exact_i128(xs.len() as i128, q);
    let mut sum = exact_i128(0, q);
    for x in xs {
        let x = exact_binary(CoreOp::Sub, x, origin, q)?;
        sum = exact_binary(CoreOp::Add, &sum, &x, q)?;
    }
    let mean = exact_binary(CoreOp::Div, &sum, &count, q)?;
    let mut ss = exact_i128(0, q);
    for x in xs {
        let x = exact_binary(CoreOp::Sub, x, origin, q)?;
        let x = exact_binary(CoreOp::Sub, &x, &mean, q)?;
        let square = exact_binary(CoreOp::Mul, &x, &x, q)?;
        ss = exact_binary(CoreOp::Add, &ss, &square, q)?;
    }
    let variance = exact_binary(CoreOp::Div, &ss, &count, q)?;
    finite(wide::sqrt(&wide::ratio(&variance)?)?)
}

pub(in crate::hir_eval) fn stats(op: CoreOp, v: V, span: Span) -> R<V> {
    let fail = |m| error(span, m);
    let xs = as_seq(&v).map_err(fail)?;
    if xs.is_empty() {
        return Ok(V::Optional(None));
    }
    let value = if op == CoreOp::StdDev && xs.iter().any(|v| matches!(v, V::Wide(_))) {
        wide_standard_deviation(xs).map_err(fail)?
    } else if op == CoreOp::StdDev {
        let mut mean = 0.0;
        let mut ss = 0.0;
        for (i, x) in xs.iter().enumerate() {
            let x = real_of(x).map_err(fail)?;
            let d = x - mean;
            mean += d / (i + 1) as f64;
            ss += d * (x - mean);
        }
        if !ss.is_finite() {
            return Err(error(span, "real standard deviation overflow"));
        }
        finite((ss.max(0.0) / xs.len() as f64).sqrt()).map_err(fail)?
    } else {
        let (sum, div) = if op == CoreOp::Median {
            let mut xs = xs.to_vec();
            if xs.iter().any(|v| wide::kind(v).is_none()) {
                return Err(error(span, "med expects exact numeric values"));
            }
            xs.sort_by(|a, b| order(a, b).unwrap_or(Ordering::Equal));
            let mid = xs.len() / 2;
            if xs.len() % 2 == 1 {
                (xs[mid].clone(), 1)
            } else {
                (
                    exact_binary(CoreOp::Add, &xs[mid - 1], &xs[mid], Domain::Rational)
                        .map_err(fail)?,
                    2,
                )
            }
        } else {
            let mut sum = exact_i128(0, Domain::Rational);
            for v in xs {
                sum = exact_binary(CoreOp::Add, &sum, v, Domain::Rational).map_err(fail)?;
            }
            (sum, xs.len() as i128)
        };
        exact_binary(
            CoreOp::Div,
            &sum,
            &exact_i128(div, Domain::Natural),
            Domain::Rational,
        )
        .map_err(fail)?
    };
    Ok(V::Optional(Some(Rc::new(value))))
}
