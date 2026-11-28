//! Immutable typed column payloads and their allocation leases.
use crate::{
    hir_eval::{exact_i128, resources, Domain, V},
    types::{Prim, Type},
};
use std::{mem::size_of, rc::Rc};

enum Data {
    Shared(Rc<[V]>),
    Boxed(Vec<V>),
    Bool(Vec<bool>),
    Int(Vec<i64>),
    Nat(Vec<u64>),
    Real(Vec<f64>),
    Text(Vec<Option<Rc<str>>>),
    Char(Vec<char>),
}

impl Data {
    fn for_type(ty: &Type) -> Self {
        match ty {
            Type::Prim(Prim::B) => Self::Bool(Vec::new()),
            Type::Prim(Prim::Z) => Self::Int(Vec::new()),
            Type::Prim(Prim::N) => Self::Nat(Vec::new()),
            Type::Prim(Prim::R) => Self::Real(Vec::new()),
            Type::Prim(Prim::S) => Self::Text(Vec::new()),
            Type::Prim(Prim::C) => Self::Char(Vec::new()),
            _ => Self::Boxed(Vec::new()),
        }
    }

    fn cell_bytes(&self) -> usize {
        match self {
            Self::Shared(_) | Self::Boxed(_) => size_of::<V>(),
            Self::Bool(_) => size_of::<bool>(),
            Self::Int(_) => size_of::<i64>(),
            Self::Nat(_) => size_of::<u64>(),
            Self::Real(_) => size_of::<f64>(),
            Self::Text(_) => size_of::<Option<Rc<str>>>(),
            Self::Char(_) => size_of::<char>(),
        }
    }

    fn allocate(&mut self, height: usize) -> Option<()> {
        match self {
            Self::Shared(_) => return None,
            Self::Boxed(xs) => xs.try_reserve_exact(height),
            Self::Bool(xs) => xs.try_reserve_exact(height),
            Self::Int(xs) => xs.try_reserve_exact(height),
            Self::Nat(xs) => xs.try_reserve_exact(height),
            Self::Real(xs) => xs.try_reserve_exact(height),
            Self::Text(xs) => xs.try_reserve_exact(height),
            Self::Char(xs) => xs.try_reserve_exact(height),
        }
        .ok()
    }

    fn value(&self, i: usize) -> V {
        match self {
            Self::Shared(xs) => xs[i].clone(),
            Self::Boxed(xs) => xs[i].clone(),
            Self::Bool(xs) => V::Bool(xs[i]),
            Self::Int(xs) => exact_i128(i128::from(xs[i]), Domain::Integer),
            Self::Nat(xs) => exact_i128(i128::from(xs[i]), Domain::Natural),
            Self::Real(xs) => V::Real(xs[i]),
            Self::Text(xs) => V::String(xs[i].clone().expect("present string cell")),
            Self::Char(xs) => V::Char(xs[i]),
        }
    }

    // A null payload gets an unused placeholder. In particular, a false
    // boolean, signed zero, or an empty string does not encode absence.
    fn try_push(&mut self, value: Option<&V>) -> Option<()> {
        match (self, value) {
            (Self::Bool(xs), Some(V::Bool(x))) => xs.push(*x),
            (Self::Bool(xs), None) => xs.push(false),
            (Self::Int(xs), Some(V::Exact(x))) if x.d == 1 && x.domain == Domain::Integer => {
                xs.push(x.n.try_into().ok()?)
            }
            (Self::Int(xs), None) => xs.push(0),
            (Self::Nat(xs), Some(V::Exact(x))) if x.d == 1 && x.domain == Domain::Natural => {
                xs.push(x.n.try_into().ok()?)
            }
            (Self::Nat(xs), None) => xs.push(0),
            (Self::Real(xs), Some(V::Real(x))) => xs.push(*x),
            (Self::Real(xs), None) => xs.push(0.0),
            (Self::Text(xs), Some(V::String(x))) => xs.push(Some(x.clone())),
            (Self::Text(xs), None) => xs.push(None),
            (Self::Char(xs), Some(V::Char(x))) => xs.push(*x),
            (Self::Char(xs), None) => xs.push('\0'),
            _ => return None,
        }
        Some(())
    }
}

pub(super) struct Column {
    data: Data,
    validity: Option<Vec<u64>>,
    _memory: resources::MemoryReservation,
}

impl Column {
    pub(super) fn new(ty: &Type, height: usize) -> Option<Self> {
        let (element, optional) = match ty {
            Type::Optional(element) => (element.as_ref(), true),
            ty => (ty, false),
        };
        let mut data = Data::for_type(element);
        // Nested optionals and unsupported types keep their whole tagged
        // values; a one-bit validity mask cannot represent both layers.
        let nullable = optional && !matches!(data, Data::Boxed(_));
        let words = if nullable { height.div_ceil(64) } else { 0 };
        let bytes = height
            .checked_mul(data.cell_bytes())?
            .checked_add(words.checked_mul(size_of::<u64>())?)?;
        let memory = resources::reserve(bytes).ok()?;
        data.allocate(height)?;
        let validity = if nullable {
            let mut bitmap = Vec::new();
            bitmap.try_reserve_exact(words).ok()?;
            bitmap.resize(words, 0);
            Some(bitmap)
        } else {
            None
        };
        Some(Self {
            data,
            validity,
            _memory: memory,
        })
    }

    pub(super) fn from_shared(values: Rc<[V]>) -> Option<Self> {
        let memory = resources::reserve_for::<V>(values.len()).ok()?;
        Some(Self {
            data: Data::Shared(values),
            validity: None,
            _memory: memory,
        })
    }

    pub(super) fn shared(&self) -> Option<&Rc<[V]>> {
        match &self.data {
            Data::Shared(values) => Some(values),
            _ => None,
        }
    }

    pub(super) fn value(&self, index: usize) -> V {
        if let Some(validity) = &self.validity {
            return if validity[index / 64] & (1 << (index % 64)) == 0 {
                V::Optional(None)
            } else {
                V::Optional(Some(Rc::new(self.data.value(index))))
            };
        }
        self.data.value(index)
    }

    pub(super) fn push(&mut self, row: usize, value: V, height: usize) -> Option<()> {
        if let Data::Boxed(values) = &mut self.data {
            values.push(value);
            return Some(());
        }
        let item = match (&self.validity, &value) {
            (None, value) => Some(Some(value)),
            (Some(_), V::Optional(None)) => Some(None),
            (Some(_), V::Optional(Some(inner))) => Some(Some(inner.as_ref())),
            (Some(_), _) => None,
        };
        if let Some(item) = item {
            if self.data.try_push(item).is_some() {
                if let Some(validity) = &mut self.validity {
                    if item.is_some() {
                        validity[row / 64] |= 1 << (row % 64);
                    }
                }
                return Some(());
            }
        }
        // A wide integer (or other value that cannot be compacted) never
        // narrows or wraps. Reserve the boxed replacement while the old
        // payload is live; preserve each earlier optional and exact tag.
        let memory = resources::reserve_for::<V>(height).ok()?;
        let mut values = Vec::new();
        values.try_reserve_exact(height).ok()?;
        values.extend((0..row).map(|index| self.value(index)));
        values.push(value);
        *self = Self {
            data: Data::Boxed(values),
            validity: None,
            _memory: memory,
        };
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir_eval::{repr, RuntimeOptions};

    #[test]
    fn validity_crosses_words_without_conflating_absence_or_signed_zero() {
        let _scope = resources::enter(RuntimeOptions::default()).unwrap();
        let mut column = Column::new(&Type::Optional(Box::new(Type::Prim(Prim::R))), 66).unwrap();
        for row in 0..66 {
            let cell = match row {
                63 => V::Optional(Some(Rc::new(V::Real(-0.0)))),
                65 => V::Optional(Some(Rc::new(V::Real(0.0)))),
                _ => V::Optional(None),
            };
            column.push(row, cell, 66).unwrap();
        }
        assert!(matches!(column.data, Data::Real(_)));
        for (row, expected) in [(62, "?0"), (63, "?1 -0"), (64, "?0"), (65, "?1 0")] {
            assert_eq!(repr(&column.value(row)), expected);
        }
    }

    #[test]
    fn wide_promotion_and_nested_optionals_retain_full_value_tags() {
        let _scope = resources::enter(RuntimeOptions::default()).unwrap();
        let ty = Type::Optional(Box::new(Type::Prim(Prim::Z)));
        let present = |value| V::Optional(Some(Rc::new(value)));
        let mut promoted = Column::new(&ty, 3).unwrap();
        for (row, value) in [
            V::Optional(None),
            present(exact_i128(0, Domain::Integer)),
            present(exact_i128(i128::from(i64::MAX) + 1, Domain::Integer)),
        ]
        .into_iter()
        .enumerate()
        {
            promoted.push(row, value, 3).unwrap();
        }
        assert!(matches!(promoted.data, Data::Boxed(_)));
        assert!(promoted.validity.is_none());
        assert_eq!(repr(&promoted.value(0)), "?0");
        assert_eq!(repr(&promoted.value(1)), "?1 0");
        assert_eq!(repr(&promoted.value(2)), "?1 9223372036854775808");

        let mut nested = Column::new(&Type::Optional(Box::new(ty)), 2).unwrap();
        nested.push(0, V::Optional(None), 2).unwrap();
        nested.push(1, present(V::Optional(None)), 2).unwrap();
        assert!(matches!(nested.data, Data::Boxed(_)));
        assert_eq!(repr(&nested.value(0)), "?0");
        assert_eq!(repr(&nested.value(1)), "?1 ?0");
    }
}
