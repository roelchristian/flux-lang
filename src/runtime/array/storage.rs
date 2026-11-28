//! Immutable dense buffers and checked row-major views.
use super::{
    equal, error, exact_i128, invariant, resources, Domain, Exact, NumericResult, Rc, Span,
    MAX_RANK, R, V,
};

pub(super) enum Storage {
    Shared(Rc<[V]>),
    Generic(Vec<V>),
    Bool(Vec<bool>),
    Int(Vec<i64>),
    Nat(Vec<u64>),
    Real(Vec<f64>),
}

impl Storage {
    fn len(&self) -> usize {
        match self {
            Self::Shared(x) => x.len(),
            Self::Generic(x) => x.len(),
            Self::Bool(x) => x.len(),
            Self::Int(x) => x.len(),
            Self::Nat(x) => x.len(),
            Self::Real(x) => x.len(),
        }
    }

    fn value(&self, index: usize) -> V {
        match self {
            Self::Shared(x) => x[index].clone(),
            Self::Generic(x) => x[index].clone(),
            Self::Bool(x) => V::Bool(x[index]),
            Self::Int(x) => exact_i128(i128::from(x[index]), Domain::Integer),
            Self::Nat(x) => exact_i128(i128::from(x[index]), Domain::Natural),
            Self::Real(x) => V::Real(x[index]),
        }
    }
}

struct Buffer {
    data: Storage,
    _reservation: resources::MemoryReservation,
}

#[derive(Clone)]
pub(in crate::hir_eval) struct Dense {
    pub(super) shape: Rc<[usize]>,
    pub(super) strides: Rc<[usize]>,
    pub(super) offset: usize,
    len: usize,
    contiguous: bool,
    buffer: Rc<Buffer>,
}

macro_rules! compact_access {
    ($slice:ident, $view:ident, $reserved:ident, $variant:ident, $ty:ty) => {
        pub(in crate::hir_eval) fn $view(&self) -> Option<(&[$ty], &[usize], usize)> {
            if let Storage::$variant(cells) = &self.buffer.data {
                Some((cells, &self.strides, self.offset))
            } else {
                None
            }
        }

        pub(in crate::hir_eval) fn $slice(&self) -> Option<&[$ty]> {
            if self.contiguous {
                let (cells, _, offset) = self.$view()?;
                cells.get(offset..offset + self.len)
            } else {
                None
            }
        }

        pub(in crate::hir_eval) fn $reserved(
            shape: Rc<[usize]>,
            cells: Vec<$ty>,
            reservation: resources::MemoryReservation,
            span: Span,
        ) -> R<V> {
            Self::owned(shape, Storage::$variant(cells), reservation)
                .map_err(|message| error(span, message))
        }
    };
}

impl Dense {
    compact_access!(real_slice, real_view, pack_real_reserved, Real, f64);
    compact_access!(int_slice, int_view, pack_int_reserved, Int, i64);
    compact_access!(nat_slice, nat_view, pack_nat_reserved, Nat, u64);
    compact_access!(bool_slice, bool_view, pack_bool_reserved, Bool, bool);

    pub(in crate::hir_eval) fn shape(&self) -> &[usize] {
        &self.shape
    }

    pub(in crate::hir_eval) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn logical_offset(&self, mut index: usize) -> usize {
        debug_assert!(index < self.len);
        if self.contiguous {
            return self.offset + index;
        }
        let mut offset = self.offset;
        for (&width, &stride) in self.shape.iter().zip(self.strides.iter()).rev() {
            offset += (index % width) * stride;
            index /= width;
        }
        offset
    }

    pub(in crate::hir_eval) fn value(&self, index: usize) -> V {
        self.buffer.data.value(self.logical_offset(index))
    }

    pub(in crate::hir_eval) fn get(&self, index: usize) -> Option<V> {
        (index < self.len).then(|| self.value(index))
    }

    pub(in crate::hir_eval) fn iter(&self) -> impl ExactSizeIterator<Item = V> + '_ {
        (0..self.len).map(|index| self.value(index))
    }

    pub(in crate::hir_eval) fn equal(&self, other: &Self) -> bool {
        self.shape == other.shape && self.iter().zip(other.iter()).all(|(a, b)| equal(&a, &b))
    }

    pub(in crate::hir_eval) fn render(&self, show: impl Fn(&V) -> String) -> String {
        let shape = self
            .shape
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let cells = self.iter().map(|v| show(&v)).collect::<Vec<_>>().join(" ");
        format!("array [{shape}] [{cells}]")
    }

    pub(super) fn owned(
        shape: Rc<[usize]>,
        data: Storage,
        reservation: resources::MemoryReservation,
    ) -> NumericResult<V> {
        let len = checked_volume(&shape)?;
        if len != data.len() {
            return Err("array shape does not match the number of elements");
        }
        Ok(V::Dense(Rc::new(Self {
            strides: contiguous_strides(&shape, len),
            shape,
            offset: 0,
            len,
            contiguous: true,
            buffer: Rc::new(Buffer {
                data,
                _reservation: reservation,
            }),
        })))
    }

    // The checked HIR element type determines the operation; storage merely
    // compresses values whose tags and exact representation are unchanged.
    fn packed(shape: Rc<[usize]>, cells: Rc<[V]>) -> NumericResult<V> {
        let len = checked_volume(&shape)?;
        if len != cells.len() {
            return Err("array shape does not match the number of elements");
        }
        if resources::optimized() {
            macro_rules! pack_if_homogeneous {
                ($variant:ident, $ty:ty, $f:expr) => {{
                    let value = $f;
                    if cells.iter().all(|cell| value(cell).is_some()) {
                        let reservation = resources::reserve_for::<$ty>(len)?;
                        let data = cells.iter().map(|cell| value(cell).unwrap()).collect();
                        return Self::owned(shape, Storage::$variant(data), reservation);
                    }
                }};
            }
            match cells.first() {
                Some(V::Bool(_)) => pack_if_homogeneous!(Bool, bool, |v: &V| match v {
                    V::Bool(x) => Some(*x),
                    _ => None,
                }),
                Some(V::Real(_)) => pack_if_homogeneous!(Real, f64, |v: &V| match v {
                    V::Real(x) => Some(*x),
                    _ => None,
                }),
                Some(V::Exact(Exact {
                    domain: Domain::Integer,
                    ..
                })) => {
                    pack_if_homogeneous!(Int, i64, |v: &V| match v {
                        V::Exact(Exact {
                            n,
                            d: 1,
                            domain: Domain::Integer,
                        }) => i64::try_from(*n).ok(),
                        _ => None,
                    })
                }
                Some(V::Exact(Exact {
                    domain: Domain::Natural,
                    ..
                })) => {
                    pack_if_homogeneous!(Nat, u64, |v: &V| match v {
                        V::Exact(Exact {
                            n,
                            d: 1,
                            domain: Domain::Natural,
                        }) => u64::try_from(*n).ok(),
                        _ => None,
                    })
                }
                _ => {}
            }
        }
        let reservation = resources::reserve_for::<V>(len)?;
        Self::owned(shape, Storage::Shared(cells), reservation)
    }

    pub(in crate::hir_eval) fn pack(shape: Rc<[usize]>, cells: Rc<[V]>, span: Span) -> R<V> {
        Self::packed(shape, cells).map_err(|message| error(span, message))
    }

    pub(in crate::hir_eval) fn pack_values_reserved(
        shape: Rc<[usize]>,
        cells: Vec<V>,
        reservation: resources::MemoryReservation,
        span: Span,
    ) -> R<V> {
        Self::owned(shape, Storage::Generic(cells), reservation)
            .map_err(|message| error(span, message))
    }

    pub(in crate::hir_eval) fn view(
        &self,
        shape: Rc<[usize]>,
        strides: Rc<[usize]>,
        offset: usize,
        span: Span,
    ) -> R<V> {
        let len = volume(&shape, span)?;
        if shape.len() != strides.len() {
            return Err(invariant(span, "dense view stride rank mismatch"));
        }
        let contiguous = if len == 0 {
            true
        } else {
            let end = shape
                .iter()
                .zip(strides.iter())
                .try_fold(offset, |x, (&n, &stride)| {
                    x.checked_add((n - 1).checked_mul(stride)?)
                });
            if end.is_none_or(|end| end >= self.buffer.data.len()) {
                return Err(invariant(span, "dense view exceeds its storage"));
            }
            let mut suffix = 1;
            shape.iter().zip(strides.iter()).rev().all(|(&n, &stride)| {
                let exact = n == 1 || stride == suffix;
                suffix *= n;
                exact
            })
        };
        Ok(V::Dense(Rc::new(Self {
            shape,
            strides,
            offset: if len == 0 { 0 } else { offset },
            len,
            contiguous,
            buffer: self.buffer.clone(),
        })))
    }

    pub(super) fn reshape(&self, shape: Rc<[usize]>, span: Span) -> R<V> {
        let len = volume(&shape, span)?;
        if self.len != len {
            return Err(error(
                span,
                "array shape does not match the number of elements",
            ));
        }
        if self.contiguous {
            return self.view(
                shape.clone(),
                contiguous_strides(&shape, len),
                self.offset,
                span,
            );
        }

        macro_rules! copy_packed {
            ($variant:ident, $cells:ident, $ty:ty) => {{
                let reservation =
                    resources::reserve_for::<$ty>(len).map_err(|message| error(span, message))?;
                let data = (0..len)
                    .map(|index| $cells[self.logical_offset(index)])
                    .collect();
                Self::owned(shape, Storage::$variant(data), reservation)
                    .map_err(|message| error(span, message))
            }};
        }
        match &self.buffer.data {
            Storage::Real(cells) => copy_packed!(Real, cells, f64),
            Storage::Int(cells) => copy_packed!(Int, cells, i64),
            Storage::Nat(cells) => copy_packed!(Nat, cells, u64),
            Storage::Bool(cells) => copy_packed!(Bool, cells, bool),
            Storage::Shared(_) | Storage::Generic(_) => {
                let reservation =
                    resources::reserve_for::<V>(len).map_err(|message| error(span, message))?;
                Self::pack_values_reserved(shape, self.iter().collect(), reservation, span)
            }
        }
    }

    pub(super) fn sequence(&self, span: Span) -> R<V> {
        if self.offset == 0 && self.contiguous && self.len == self.buffer.data.len() {
            if let Storage::Shared(cells) = &self.buffer.data {
                return Ok(V::Sequence(cells.clone()));
            }
        }
        // V::Sequence cannot represent a view or keep a temporary lease.
        // Admit its materialization (including the Vec -> Rc copy) before
        // visiting cells, without claiming to track ordinary sequence lifetime.
        let count = self
            .len
            .checked_mul(2)
            .ok_or_else(|| error(span, "array ravel exceeds addressable size"))?;
        let _scratch =
            resources::reserve_for::<V>(count).map_err(|message| error(span, message))?;
        let cells: Vec<_> = self.iter().collect();
        Ok(V::Sequence(cells.into()))
    }
}

fn contiguous_strides(shape: &[usize], len: usize) -> Rc<[usize]> {
    if len == 0 {
        // Empty logical views have no accessible storage offset, so no
        // intermediate products of potentially huge remaining axes.
        return vec![0; shape.len()].into();
    }
    let mut out = vec![0; shape.len()];
    let mut suffix = 1;
    for (i, &width) in shape.iter().enumerate().rev() {
        out[i] = suffix;
        suffix *= width; // checked_volume already bounded this product.
    }
    out.into()
}

pub(super) fn volume(shape: &[usize], span: Span) -> R<usize> {
    checked_volume(shape).map_err(|message| error(span, message))
}

pub(in crate::hir_eval) fn checked_volume(shape: &[usize]) -> NumericResult<usize> {
    if shape.len() > MAX_RANK {
        return Err("array rank exceeds 16");
    }
    let limits = resources::limits();
    if shape.len() > limits.max_rank {
        return Err("array rank exceeds the configured limit");
    }
    if shape.iter().any(|&n| n > limits.max_elements) {
        return Err("array dimension exceeds the collection limit");
    }
    if shape.contains(&0) {
        return Ok(0);
    }
    shape.iter().try_fold(1usize, |size, &n| {
        size.checked_mul(n)
            .filter(|&product| product <= limits.max_elements)
            .ok_or("array element count exceeds the collection limit")
    })
}

#[cfg(test)]
mod storage_tests {
    use super::{
        super::{algebra, expect_dense},
        *,
    };
    use crate::hir_eval::Control;

    fn value(result: R<V>) -> V {
        result.map_err(Control::into_error).unwrap()
    }

    fn integers(dense: &Dense) -> Vec<i128> {
        dense
            .iter()
            .map(|v| match v {
                V::Exact(Exact {
                    n,
                    d: 1,
                    domain: Domain::Integer,
                }) => n,
                _ => panic!("expected a preserved integer tag"),
            })
            .collect()
    }

    #[test]
    fn views_share_one_lease_and_noncontiguous_reshape_copies_logical_order() {
        let _scope = resources::enter(resources::RuntimeOptions::default()).unwrap();
        let free = resources::available_memory();
        let base_value = value(Dense::pack(
            Rc::from([2, 3]),
            (1..=6).map(|n| exact_i128(n, Domain::Integer)).collect(),
            Span::default(),
        ));
        let base = expect_dense(&base_value).unwrap();
        assert_eq!(base.int_slice(), Some([1, 2, 3, 4, 5, 6].as_slice()));
        assert_eq!(resources::available_memory(), free - 6 * size_of::<i64>());

        let transposed = value(algebra::reorder(base, &[1, 0], Span::default()));
        let transposed = expect_dense(&transposed).unwrap();
        assert_eq!(transposed.shape(), &[3, 2]);
        assert_eq!(integers(transposed), [1, 4, 2, 5, 3, 6]);
        assert!(transposed.int_slice().is_none());

        let section = value(algebra::section(
            &[2, 1],
            &[2, 2],
            transposed,
            Span::default(),
        ));
        let section = expect_dense(&section).unwrap();
        assert!(Rc::ptr_eq(&base.buffer, &section.buffer));
        assert_eq!(integers(section), [2, 5, 3, 6]);
        assert_eq!(resources::available_memory(), free - 6 * size_of::<i64>());
        assert!(section.get(4).is_none());

        let reshaped = value(section.reshape(Rc::from([4]), Span::default()));
        let reshaped = expect_dense(&reshaped).unwrap();
        assert!(!Rc::ptr_eq(&base.buffer, &reshaped.buffer));
        assert_eq!(reshaped.int_slice(), Some([2, 5, 3, 6].as_slice()));
        assert_eq!(resources::available_memory(), free - 10 * size_of::<i64>());
        assert!(base
            .view(Rc::from([2]), Rc::from([usize::MAX]), 0, Span::default())
            .is_err());
    }

    #[test]
    fn generic_zero_volume_views_do_not_form_address_products() {
        let options = resources::RuntimeOptions {
            optimize: false,
            resources: resources::ResourcePolicy {
                max_elements: 10_000_000,
                ..resources::ResourcePolicy::default()
            },
        };
        let _scope = resources::enter(options).unwrap();
        let mut shape = [10_000_000; MAX_RANK];
        shape[5] = 0;
        let base_value = value(Dense::pack(Rc::from(shape), Rc::from([]), Span::default()));
        let base = expect_dense(&base_value).unwrap();
        let reversed: Vec<_> = (0..MAX_RANK).rev().collect();
        let reversed = value(algebra::reorder(base, &reversed, Span::default()));
        let reversed = expect_dense(&reversed).unwrap();
        shape.reverse();
        assert_eq!(reversed.shape(), &shape);
        assert_eq!(reversed.len(), 0);
        assert!(reversed.get(0).is_none());
        assert!(Rc::ptr_eq(&base.buffer, &reversed.buffer));
        assert!(
            matches!(value(reversed.sequence(Span::default())), V::Sequence(xs) if xs.is_empty())
        );
    }
}
