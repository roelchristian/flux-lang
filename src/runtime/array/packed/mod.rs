//! Packed kernel dispatch using checked operation, type and lift evidence.
use super::super::{
    array, array::Dense, error, exact_i128, resources, resources::MemoryReservation, wide, CoreOp,
    Domain, Eval, Function, LiftMode, NumericResult, Prim, Rc, Span, Type, DIV_BY_ZERO, R, V,
};

mod contraction;
mod pointwise;
mod reduction;
mod scan;

pub(in crate::hir_eval) use contraction::{dot, matmul};
pub(in crate::hir_eval) use pointwise::{binary, dense_work, unary};
pub(in crate::hir_eval) use reduction::intrinsic;
pub(in crate::hir_eval) use scan::{mapped, seeded, unseeded, ScanPlan};

type View<'a, T> = (&'a [T], &'a [usize], usize);
type Fast<T> = Option<R<T>>;
const MAX_RANK: usize = 16;

trait Packed: Copy {
    fn scalar(v: &V) -> Option<Self>;
    fn view(d: &Dense) -> Option<View<'_, Self>>;
    fn slice(d: &Dense) -> Option<&[Self]>;
    fn value(self) -> V;
    fn pack(shape: Rc<[usize]>, cells: Vec<Self>, lease: MemoryReservation, span: Span) -> R<V>;
}

macro_rules! packed {
    ($t:ty, $view:ident, $slice:ident, $pack:ident, $scalar:expr, $value:expr) => {
        impl Packed for $t {
            fn scalar(v: &V) -> Option<Self> {
                ($scalar)(v)
            }

            fn view(d: &Dense) -> Option<View<'_, Self>> {
                d.$view()
            }

            fn slice(d: &Dense) -> Option<&[Self]> {
                d.$slice()
            }

            fn value(self) -> V {
                ($value)(self)
            }

            fn pack(
                shape: Rc<[usize]>,
                cells: Vec<Self>,
                lease: MemoryReservation,
                span: Span,
            ) -> R<V> {
                Dense::$pack(shape, cells, lease, span)
            }
        }
    };
}

packed!(
    f64,
    real_view,
    real_slice,
    pack_real_reserved,
    |v: &V| match v {
        V::Real(r) => Some(*r),
        _ => None,
    },
    V::Real
);
packed!(
    i64,
    int_view,
    int_slice,
    pack_int_reserved,
    |v: &V| match v {
        V::Exact(n) if n.d == 1 && matches!(n.domain, Domain::Integer | Domain::Natural) =>
            i64::try_from(n.n).ok(),
        _ => None,
    },
    |x| exact_i128(i128::from(x), Domain::Integer)
);
packed!(
    u64,
    nat_view,
    nat_slice,
    pack_nat_reserved,
    |v: &V| match v {
        V::Exact(n) if n.d == 1 && n.domain == Domain::Natural => u64::try_from(n.n).ok(),
        _ => None,
    },
    |x| exact_i128(i128::from(x), Domain::Natural)
);
packed!(
    bool,
    bool_view,
    bool_slice,
    pack_bool_reserved,
    |v: &V| match v {
        V::Bool(b) => Some(*b),
        _ => None,
    },
    V::Bool
);

enum Source<'a, T> {
    Scalar(T),
    Array(&'a Dense),
}

impl<'a, T: Packed> Source<'a, T> {
    fn new(v: &'a V, array: bool) -> Option<Self> {
        if array {
            let V::Dense(d) = v else { return None };
            // Only the requested representation is accepted.
            T::view(d)?;
            Some(Self::Array(d))
        } else {
            T::scalar(v).map(Self::Scalar)
        }
    }
}

enum Reader<'a, T> {
    Constant(T),
    Linear(&'a [T]),
    Strided {
        buffer: &'a [T],
        offset: usize,
        strides: [usize; MAX_RANK],
    },
}
impl<'a, T: Packed> Reader<'a, T> {
    fn new(source: Source<'a, T>, output: &[usize]) -> Option<Self> {
        let array = match source {
            Source::Scalar(value) => return Some(Self::Constant(value)),
            Source::Array(array) => array,
        };
        if array.shape() == output {
            if let Some(slice) = T::slice(array) {
                return Some(Self::Linear(slice));
            }
        }
        let (buffer, physical, offset) = T::view(array)?;
        let mut strides = [0; MAX_RANK];
        let pad = output.len() - array.shape().len();
        for (i, (&width, &stride)) in array.shape().iter().zip(physical).enumerate() {
            if width != 1 {
                strides[pad + i] = stride;
            }
        }
        Some(Self::Strided {
            buffer,
            offset,
            strides,
        })
    }

    fn at(&self, mut index: usize, shape: &[usize]) -> T {
        match self {
            Self::Constant(x) => *x,
            Self::Linear(values) => values[index],
            Self::Strided {
                buffer,
                offset,
                strides,
            } => {
                let mut physical = *offset;
                for (&width, &stride) in shape.iter().zip(strides).rev() {
                    physical += (index % width) * stride;
                    index /= width;
                }
                buffer[physical]
            }
        }
    }
}

// Enumerate one checked broadcast layout in logical row-major order.
struct Cells<'a, T> {
    reader: Reader<'a, T>,
    shape: &'a [usize],
    coords: [usize; MAX_RANK],
    offset: usize,
    index: usize,
    len: usize,
}

impl<'a, T: Packed> Cells<'a, T> {
    fn new(reader: Reader<'a, T>, shape: &'a [usize], len: usize) -> Self {
        let offset = match &reader {
            Reader::Strided { offset, .. } => *offset,
            _ => 0,
        };
        Self {
            reader,
            shape,
            coords: [0; MAX_RANK],
            offset,
            index: 0,
            len,
        }
    }
}

impl<T: Packed> Iterator for Cells<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.index == self.len {
            return None;
        }
        let value = match &self.reader {
            Reader::Constant(value) => *value,
            Reader::Linear(buffer) => buffer[self.index],
            Reader::Strided {
                buffer, strides, ..
            } => {
                let value = buffer[self.offset];
                if self.index + 1 < self.len {
                    for i in (0..self.shape.len()).rev() {
                        if self.coords[i] + 1 < self.shape[i] {
                            self.coords[i] += 1;
                            self.offset += strides[i];
                            break;
                        }
                        self.offset -= self.coords[i] * strides[i];
                        self.coords[i] = 0;
                    }
                }
                value
            }
        };
        self.index += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.index;
        (remaining, Some(remaining))
    }
}

impl<T: Packed> ExactSizeIterator for Cells<'_, T> {}

fn dense_cells<T: Packed>(dense: &Dense) -> Option<Cells<'_, T>> {
    let shape = dense.shape();
    Some(Cells::new(
        Reader::new(Source::Array(dense), shape)?,
        shape,
        dense.len(),
    ))
}

pub(in crate::hir_eval) fn real_cells(
    dense: &Dense,
) -> Option<impl ExactSizeIterator<Item = f64> + '_> {
    dense_cells(dense)
}

pub(in crate::hir_eval) fn bool_cells(
    dense: &Dense,
) -> Option<impl ExactSizeIterator<Item = bool> + '_> {
    dense_cells(dense)
}

fn finite_real(x: f64) -> NumericResult<f64> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err("nonfinite real result")
    }
}

fn real_binary(op: CoreOp, a: f64, b: f64) -> NumericResult<Option<f64>> {
    use CoreOp::*;
    finite_real(match op {
        Add => a + b,
        Sub => a - b,
        Mul => a * b,
        Div if b == 0.0 => return Err(DIV_BY_ZERO),
        Div => a / b,
        _ => return Ok(None),
    })
    .map(Some)
}

fn compact_exact() -> bool {
    // At stricter policies each intermediate, not only the final packed
    // value, must pass the generic arithmetic bit-limit checks.
    wide::max_bits() >= u64::from(u64::BITS)
}

#[cfg(test)]
mod tests {
    use super::{resources::RuntimeOptions, *};
    use crate::hir_eval::real_of;

    #[test]
    fn packed_contractions_read_physical_offsets_with_or_without_scratch() {
        // 224 bytes cover the two 12-cell buffers plus the 4-cell result,
        // but neither of the optional copies. The larger budget admits both.
        for memory in [224, 1024] {
            let mut options = RuntimeOptions::default();
            options.resources.max_memory = memory;
            let _scope = resources::enter(options).unwrap();
            let span = Span::default();
            let packed = || {
                let value = Dense::pack(
                    Rc::from([3, 4]),
                    (1..=12).map(|x| V::Real(f64::from(x))).collect(),
                    span,
                );
                let Ok(V::Dense(dense)) = value else {
                    panic!("expected packed matrix")
                };
                dense
            };
            let view = |array: &Dense, shape, strides, offset| {
                let Ok(V::Dense(dense)) = array.view(shape, strides, offset, span) else {
                    panic!("expected strided view")
                };
                assert!(dense.real_slice().is_none());
                dense
            };
            let a = packed();
            let b = packed();
            let a_dot = view(&a, Rc::from([3]), Rc::from([5]), 1);
            let b_dot = view(&b, Rc::from([3]), Rc::from([4]), 2);
            let ty = Type::Prim(Prim::R);
            assert!(matches!(dot(&a_dot, &b_dot, &ty, span), Some(Ok(V::Real(x))) if x == 187.0));
            let a_matrix = view(&a, Rc::from([2, 3]), Rc::from([1, 4]), 1);
            let b_matrix = view(&b, Rc::from([3, 2]), Rc::from([4, 1]), 1);
            let result = matmul(&a_matrix, &b_matrix, Rc::from([2, 2]), &ty, span);
            let Some(Ok(V::Dense(result))) = result else {
                panic!("packed matrix contraction must work without optional scratch")
            };
            assert_eq!(result.real_slice().unwrap(), &[140.0, 158.0, 158.0, 179.0]);
            assert_eq!(resources::available_memory(), memory - 224);
        }
    }

    #[test]
    fn integer_to_real_kernels_pack_with_the_same_binary64_rounding() {
        let _scope = resources::enter(RuntimeOptions::default()).unwrap();
        let span = Span::default();
        let mut state = 0u64;
        // Include the halfway cases, and test thousands of deterministic
        // magnitudes and significands independently of the packed cast.
        let integers = [
            0,
            1,
            (1 << 53) - 1,
            (1 << 53) + 1,
            (1 << 53) + 3,
            i64::MAX as u64,
            1 << 63,
            u64::MAX,
        ]
        .into_iter()
        .chain((0..4096).map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            state
        }))
        .collect::<Vec<_>>();
        for domain in [Domain::Natural, Domain::Integer] {
            let cells: Rc<[V]> = integers
                .iter()
                .map(|&x| {
                    let number = if domain == Domain::Natural {
                        i128::from(x)
                    } else {
                        i128::from(x as i64)
                    };
                    exact_i128(number, domain)
                })
                .collect();
            let Ok(input) = Dense::pack(Rc::from([cells.len()]), cells.clone(), span) else {
                panic!("expected packed integer vector")
            };
            let output = unary(
                CoreOp::ToReal,
                &[input],
                &Type::dense_array(Type::Prim(Prim::R)),
                Some(LiftMode::LeftArray),
                span,
            );
            let Some(Ok(V::Dense(dense))) = output else {
                panic!("expected packed real conversion")
            };
            for (&actual, source) in dense.real_slice().unwrap().iter().zip(cells.iter()) {
                assert_eq!(actual.to_bits(), real_of(source).unwrap().to_bits());
            }
        }
    }
}
