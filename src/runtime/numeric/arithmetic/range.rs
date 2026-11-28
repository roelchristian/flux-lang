//! Bounded exact ranges and shared logical-work charging.
use super::{exact_i128, Domain, NumericResult};
use crate::hir_eval::{resources, wide, V};
use num_rational::BigRational;
use num_traits::ToPrimitive;
use std::rc::Rc;

#[cfg(test)]
pub(in crate::hir_eval) fn range(lo: &V, hi: &V, d: Domain) -> NumericResult<V> {
    range_with_fuel(lo, hi, d, None)
}

pub(in crate::hir_eval) fn subtract_work(fuel: &mut usize, units: usize) -> NumericResult<()> {
    *fuel = fuel
        .checked_sub(units)
        .ok_or("prototype execution limit exhausted")?;
    Ok(())
}

const TOO_BIG: &str = "prototype range collection limit exceeded";

fn range_count(delta: Option<usize>) -> NumericResult<usize> {
    delta
        .and_then(|d| d.checked_add(1))
        .filter(|&n| n <= resources::limits().max_elements)
        .ok_or(TOO_BIG)
}

/// Logical length for an inline exact range, independent of physical storage.
pub(in crate::hir_eval) fn inline_range_len(lo: i128, hi: i128) -> NumericResult<usize> {
    if lo > hi {
        return Ok(0);
    }
    range_count(hi.checked_sub(lo).and_then(|n| usize::try_from(n).ok()))
}

pub(in crate::hir_eval) fn range_with_fuel(
    lo: &V,
    hi: &V,
    d: Domain,
    fuel: Option<&mut usize>,
) -> NumericResult<V> {
    if let (V::Exact(lo), V::Exact(hi)) = (lo, hi) {
        let (lo, hi) = (lo.integral()?, hi.integral()?);
        let len = inline_range_len(lo, hi)?;
        if len == 0 {
            return Ok(V::Sequence(Rc::from([])));
        }
        return collect_range(len, fuel, (lo..=hi).map(|i| Ok(exact_i128(i, d))));
    }
    let (mut lo, hi) = (wide::integral(lo)?, wide::integral(hi)?);
    if lo > hi {
        return Ok(V::Sequence(Rc::from([])));
    }
    let len = range_count((&hi - &lo).to_usize())?;
    // Keep the inherited aggregate cap on BigInt endpoint copies.
    if lo.bits().max(hi.bits()).saturating_mul(len as u64) > 64 * 1024 * 1024 * 8 {
        return Err("wide range resource limit exceeded (64 MiB of integer data)");
    }
    collect_range(
        len,
        fuel,
        (0..len).map(|_| {
            let v = wide::ratio_value(BigRational::from_integer(lo.clone()), d);
            lo += 1u8;
            v
        }),
    )
}

fn collect_range(
    len: usize,
    fuel: Option<&mut usize>,
    values: impl Iterator<Item = NumericResult<V>>,
) -> NumericResult<V> {
    if let Some(fuel) = fuel {
        subtract_work(fuel, len)?;
    }
    // Building the final Rc<[V]> can briefly overlap the source Vec.
    // The completed ordinary sequence is not kept in the dense ledger.
    let _scratch =
        resources::reserve_for::<V>(len.checked_mul(2).ok_or("range allocation is too large")?)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| "range allocation failed")?;
    for value in values {
        out.push(value?);
    }
    Ok(V::Sequence(out.into()))
}
