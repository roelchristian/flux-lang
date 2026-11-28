//! Conversion between external CSV cell text and checked scalar values.
use super::V;
use crate::hir_eval::{
    decimal, domain, finite, parse_exact_value, rational, repr, wide, Domain, Exact, NumericResult,
    OVERFLOW,
};
use crate::types::{Prim, Type};
use num_bigint::BigInt;
use std::rc::Rc;

pub(in crate::hir_eval) fn read_cell(s: &str, ty: &Type) -> NumericResult<V> {
    if let Type::Optional(inner) = ty {
        return if s.is_empty() {
            Ok(V::Optional(None))
        } else {
            Ok(V::Optional(Some(Rc::new(read_cell(s, inner)?))))
        };
    }
    Ok(match ty {
        Type::Prim(Prim::S) => V::String(Rc::from(s)),
        Type::Prim(Prim::C) => {
            let mut i = s.chars();
            match (i.next(), i.next()) {
                (Some(c), None) => V::Char(c),
                _ => return Err("expected a single character"),
            }
        }
        Type::Prim(Prim::B) => match s {
            "true" | "1b" => V::Bool(true),
            "false" | "0b" => V::Bool(false),
            _ => return Err("expected true, false, 1b, or 0b"),
        },
        Type::Prim(Prim::N | Prim::Z) => parse_exact_value(s, domain(ty).expect("known domain"))?,
        Type::Prim(Prim::Q) => {
            if let Some((a, b)) = s.split_once('/') {
                // Use the fast bounded parser when possible.
                match (a.parse::<i128>(), b.parse::<i128>()) {
                    (Ok(a), Ok(b)) => match Exact::new(a, b, Domain::Rational) {
                        Ok(v) => V::Exact(v),
                        Err(e) if e == OVERFLOW => rational(BigInt::from(a), BigInt::from(b))?,
                        Err(e) => return Err(e),
                    },
                    _ => rational(wide::parse_int(a)?, wide::parse_int(b)?)?,
                }
            } else {
                if s.is_empty()
                    || s.bytes()
                        .any(|b| !matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.'))
                {
                    return Err("expected a decimal or rational");
                }
                parse_exact_value(s, Domain::Rational)?
            }
        }
        Type::Prim(Prim::D) => decimal::decimal_literal(s)?,
        Type::Prim(Prim::R) => finite(s.parse().map_err(|_| "expected a finite real")?)?,
        _ => return Err("CSV schema contains an unsupported column type"),
    })
}
pub(in crate::hir_eval) fn write_cell(v: &V) -> NumericResult<String> {
    Ok(match v {
        V::String(s) => s.to_string(),
        V::Char(c) => c.to_string(),
        V::Bool(b) => b.to_string(),
        V::Exact(_) | V::Wide(_) | V::Real(_) => repr(v),
        V::Optional(None) => String::new(),
        V::Optional(Some(v)) => {
            let s = write_cell(v)?;
            if s.is_empty() {
                return Err("present optional cell encodes as blank and would be read as ?0");
            }
            s
        }
        _ => return Err("unsupported CSV output cell type"),
    })
}
