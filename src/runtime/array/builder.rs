//! Direct compact ingress for a checked integer range consumed by `form`.
use super::{shape, volume, Dense};
use crate::{
    hir::Span,
    hir_eval::{
        error, numeric::arithmetic::inline_range_len, resources, Domain, NumericResult, R, V,
    },
};

pub(in crate::hir_eval) enum PackedRange {
    Int(i64, i64),
    Nat(u64, u64),
}

pub(in crate::hir_eval) struct AdmittedRange {
    source: PackedRange,
    len: usize,
    lease: resources::MemoryReservation,
    origin: Span,
}

impl PackedRange {
    pub(in crate::hir_eval) fn from_values(lo: &V, hi: &V, domain: Domain) -> Option<Self> {
        let (V::Exact(lo), V::Exact(hi)) = (lo, hi) else {
            return None;
        };
        let (lo, hi) = (lo.integral().ok()?, hi.integral().ok()?);
        // The domain comes from HIR, not the runtime storage tags. Refuse
        // either endpoint that the selected representation cannot hold.
        match domain {
            Domain::Integer => Some(Self::Int(lo.try_into().ok()?, hi.try_into().ok()?)),
            Domain::Natural => Some(Self::Nat(lo.try_into().ok()?, hi.try_into().ok()?)),
            _ => None,
        }
    }

    pub(in crate::hir_eval) fn checked_len(&self) -> NumericResult<usize> {
        match self {
            Self::Int(lo, hi) => inline_range_len(i128::from(*lo), i128::from(*hi)),
            Self::Nat(lo, hi) => inline_range_len(i128::from(*lo), i128::from(*hi)),
        }
    }

    /// Account for the actual payload at the original range boundary so
    /// rejected admission still precedes any outer form work or fault.
    pub(in crate::hir_eval) fn admit(self, len: usize, origin: Span) -> R<AdmittedRange> {
        // Both native integer representations occupy eight bytes.
        let lease = resources::reserve_for::<u64>(len).map_err(|reason| error(origin, reason))?;
        Ok(AdmittedRange {
            source: self,
            len,
            lease,
            origin,
        })
    }
}

impl AdmittedRange {
    /// Seal the destination buffer under one live lease. There is no staged
    /// boxed sequence, and no intermediate vector of the compact cells.
    pub(in crate::hir_eval) fn form(self, dims: &V, span: Span) -> R<V> {
        let Self {
            source,
            len,
            lease,
            origin,
        } = self;
        let shape = shape(dims, span)?;
        if volume(&shape, span)? != len {
            return Err(error(
                span,
                "array shape does not match the number of elements",
            ));
        }
        fn allocate<T>(len: usize, span: Span) -> R<Vec<T>> {
            let mut cells = Vec::new();
            cells
                .try_reserve_exact(len)
                .map_err(|_| error(span, "range allocation failed"))?;
            Ok(cells)
        }
        match source {
            PackedRange::Int(lo, hi) => {
                let mut cells = allocate(len, origin)?;
                cells.extend(lo..=hi);
                Dense::pack_int_reserved(shape, cells, lease, span)
            }
            PackedRange::Nat(lo, hi) => {
                let mut cells = allocate(len, origin)?;
                cells.extend(lo..=hi);
                Dense::pack_nat_reserved(shape, cells, lease, span)
            }
        }
    }
}
