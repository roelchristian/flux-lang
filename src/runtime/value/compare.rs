//! Equality, ordering and hash-compatible structural keys.
use super::V;
use crate::hir_eval::{wide, NumericResult};
use crate::{hir::TableSchema, types::Type};
use num_rational::BigRational;
use num_traits::ToPrimitive;
use std::{cmp::Ordering, rc::Rc};

// A schema's stored order controls presentation, not structural equality.
// Preserve order for tuple slots and retain lexical identity for nominal types.
fn same_field_types(a: &[(String, Type)], b: &[(String, Type)]) -> bool {
    a.len() == b.len()
        && a.iter().all(|(name, ty)| {
            b.iter()
                .find(|(other, _)| name == other)
                .is_some_and(|(_, rhs)| same_type_layout(ty, rhs))
        })
}
fn same_tuple_types(a: &[Type], b: &[Type]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_type_layout(a, b))
}
fn same_type_layout(a: &Type, b: &Type) -> bool {
    use Type::*;
    match (a, b) {
        (Record(a), Record(b)) => a.tail == b.tail && same_field_types(&a.fields, &b.fields),
        (Tuple(a), Tuple(b)) => same_tuple_types(a, b),
        (Seq(a), Seq(b))
        | (Table(a), Table(b))
        | (Optional(a), Optional(b))
        | (Action(a), Action(b)) => same_type_layout(a, b),
        (Dict(a, b), Dict(c, d)) | (Result(a, b), Result(c, d)) | (Fun(a, b), Fun(c, d)) => {
            same_type_layout(a, c) && same_type_layout(b, d)
        }
        _ => a == b,
    }
}
fn same_table_layout(a: &TableSchema, b: &TableSchema) -> bool {
    a == b
        || match (a, b) {
            (TableSchema::Named(a), TableSchema::Named(b)) => same_field_types(a, b),
            (TableSchema::Positional(a), TableSchema::Positional(b)) => same_tuple_types(a, b),
            _ => false,
        }
}
pub(in crate::hir_eval) fn equal(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Dense(a), V::Dense(b)) => a.equal(b),
        (V::Exact(a), V::Exact(b)) => a.n == b.n && a.d == b.d,
        (V::Exact(_) | V::Wide(_), V::Exact(_) | V::Wide(_)) => {
            order(a, b).is_ok_and(|v| v.is_eq())
        }
        (V::Real(a), V::Real(b)) => a == b,
        (V::Bool(a), V::Bool(b)) => a == b,
        (V::String(a), V::String(b)) => a == b,
        (V::Char(a), V::Char(b)) => a == b,
        (V::Unit, V::Unit) | (V::Optional(None), V::Optional(None)) => true,
        (V::Optional(Some(a)), V::Optional(Some(b))) => equal(a, b),
        (V::Tuple(a), V::Tuple(b)) | (V::Sequence(a), V::Sequence(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| equal(a, b))
        }
        (V::Record(a), V::Record(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.iter().any(|(l, w)| k == l && equal(v, w)))
        }
        (V::Table(a, ta), V::Table(b, tb)) => {
            same_table_layout(ta, tb)
                && a.len() == b.len()
                && a.iter().zip(b.iter()).all(|(a, b)| equal(a, b))
        }
        (V::Dict(a), V::Dict(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.iter().any(|(l, w)| equal(k, l) && equal(v, w)))
        }
        (V::Variant(a), V::Variant(b)) => {
            a.constructor == b.constructor
                && a.fields.len() == b.fields.len()
                && a.fields
                    .iter()
                    .zip(b.fields.iter())
                    .all(|(x, y)| equal(x, y))
        }
        (V::Outcome(Ok(a)), V::Outcome(Ok(b))) => equal(a, b),
        (V::Outcome(Err(a)), V::Outcome(Err(b))) => equal(&a.payload, &b.payload),
        _ => false,
    }
}

pub(in crate::hir_eval) const KEY_WORK_LIMIT: usize = 64 * 1024 * 1024;

#[derive(PartialEq, Eq, Hash)]
pub(in crate::hir_eval) enum Key {
    Num(i128, i128),
    WideNum(Box<BigRational>),
    Real(u64),
    Bool(bool),
    Str(Rc<str>),
    Char(char),
    Unit,
    None,
    Some(Box<Key>),
    Tuple(Box<[Key]>),
}

pub(in crate::hir_eval) fn key(v: &V) -> Option<Key> {
    let mut budget = KEY_WORK_LIMIT;
    key_with_budget(v, &mut budget, 0)
}
pub(in crate::hir_eval) fn key_with_budget(v: &V, budget: &mut usize, depth: usize) -> Option<Key> {
    if depth >= 128 {
        return None;
    }
    *budget = budget.checked_sub(std::mem::size_of::<Key>())?;
    Some(match v {
        V::Exact(x) => Key::Num(x.n, x.d),
        V::Wide(_) => {
            let q = wide::ratio(v).ok()?;
            let bytes = (q.numer().bits() + q.denom().bits()).div_ceil(8);
            *budget = budget.checked_sub(usize::try_from(bytes).ok()?)?;
            if let (Some(n), Some(d)) = (q.numer().to_i128(), q.denom().to_i128()) {
                Key::Num(n, d)
            } else {
                Key::WideNum(Box::new(q))
            }
        }
        V::Real(r) => Key::Real(if *r == 0.0 { 0 } else { r.to_bits() }),
        V::Bool(b) => Key::Bool(*b),
        V::String(s) => {
            *budget = budget.checked_sub(s.len())?;
            Key::Str(s.clone())
        }
        V::Char(c) => Key::Char(*c),
        V::Unit => Key::Unit,
        V::Optional(None) => Key::None,
        V::Optional(Some(x)) => Key::Some(Box::new(key_with_budget(x, budget, depth + 1)?)),
        V::Tuple(xs) => Key::Tuple(
            xs.iter()
                .map(|x| key_with_budget(x, budget, depth + 1))
                .collect::<Option<Vec<_>>>()?
                .into_boxed_slice(),
        ),
        _ => return None,
    })
}
pub(in crate::hir_eval) fn order(a: &V, b: &V) -> NumericResult<Ordering> {
    match (a, b) {
        (V::Exact(a), V::Exact(b)) => Ok(a.order(*b)),
        (V::Exact(_) | V::Wide(_), V::Exact(_) | V::Wide(_)) => {
            Ok(wide::ratio(a)?.cmp(&wide::ratio(b)?))
        }
        (V::Real(a), V::Real(b)) => a.partial_cmp(b).ok_or("nonfinite real result"),
        (V::String(a), V::String(b)) => Ok(a.cmp(b)),
        (V::Char(a), V::Char(b)) => Ok(a.cmp(b)),
        _ => Err("values are not comparable in this backend"),
    }
}
