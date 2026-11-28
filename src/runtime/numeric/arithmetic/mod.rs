//! Shared scalar numerical semantics for lifted operations, kernels and I/O.
mod exact;
mod lift;
mod range;
mod scalar;
mod statistics;

pub(in crate::hir_eval) type NumericResult<T> = Result<T, &'static str>;
pub(in crate::hir_eval) use exact::{
    as_ratio, domain, exact_binary, exact_i128, exact_index, exact_unary, parse_exact,
    parse_exact_value, rational, real_of, to_exact, Domain, Exact, DIV_BY_ZERO, OVERFLOW,
};
pub(in crate::hir_eval) use lift::{lift, lift_one};
#[cfg(test)]
pub(in crate::hir_eval) use range::range;
pub(in crate::hir_eval) use range::{range_with_fuel, subtract_work};
pub(in crate::hir_eval) use scalar::{finite, numeric_identity, numeric_reduce, scalar_binary};
pub(in crate::hir_eval) use statistics::stats;
#[cfg(test)]
pub(in crate::hir_eval) use statistics::wide_standard_deviation;
