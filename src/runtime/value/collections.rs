//! Checked value accessors, table layout and row/column conversion.
use super::V;
use crate::hir::{Span, TableSchema};
use crate::hir_eval::{error, NumericResult, R};
use std::rc::Rc;

pub(in crate::hir_eval) fn projection(field: &str, v: V, span: Span) -> R<V> {
    match v {
        V::Record(fs) => fs
            .iter()
            .find(|(k, _)| k.as_ref() == field)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| error(span, format!("record has no field .{field}"))),
        V::Tuple(xs) => field
            .parse::<usize>()
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| xs.get(i))
            .cloned()
            .ok_or_else(|| error(span, format!("tuple has no field .{field}"))),
        V::Sequence(xs) | V::Table(xs, _) => Ok(V::Sequence(
            xs.iter()
                .cloned()
                .map(|x| projection(field, x, span))
                .collect::<R<Vec<_>>>()?
                .into(),
        )),
        _ => Err(error(
            span,
            "projection expects a row or structured collection",
        )),
    }
}

type Collection = (Rc<[V]>, Option<Rc<TableSchema>>);
pub(in crate::hir_eval) fn collection(v: V, span: Span) -> R<Collection> {
    match v {
        V::Sequence(xs) => Ok((xs, None)),
        V::Table(xs, s) => Ok((xs, Some(s))),
        _ => Err(error(span, "expected sequence or table")),
    }
}
pub(in crate::hir_eval) fn wrap(xs: Vec<V>, s: Option<Rc<TableSchema>>) -> V {
    if let Some(s) = s {
        V::Table(xs.into(), s)
    } else {
        V::Sequence(xs.into())
    }
}
pub(in crate::hir_eval) fn layout(
    schema: Option<&Rc<TableSchema>>,
    span: Span,
) -> R<Rc<TableSchema>> {
    schema
        .cloned()
        .ok_or_else(|| error(span, "closed table schema missing from typed HIR"))
}
pub(in crate::hir_eval) fn named(v: &V, span: Span) -> R<&[(Box<str>, V)]> {
    if let V::Record(fields) = v {
        Ok(fields)
    } else {
        Err(error(span, "expected a record"))
    }
}
type RowCells = Vec<(Box<str>, V)>;
pub(in crate::hir_eval) fn column_rows(v: &V, span: Span) -> R<Vec<RowCells>> {
    let fields = named(v, span)?;
    let mut n = None;
    let mut columns = Vec::new();
    for (k, c) in fields {
        let items = as_seq(c).map_err(|m| error(span, m))?;
        if n.is_some_and(|n| n != items.len()) {
            return Err(error(span, "record columns have unequal lengths"));
        }
        n = Some(items.len());
        columns.push((k, items));
    }
    let mut rows = (0..n.unwrap_or(0))
        .map(|_| Vec::with_capacity(fields.len()))
        .collect::<Vec<_>>();
    for (k, items) in columns {
        for (r, v) in rows.iter_mut().zip(items) {
            r.push((k.clone(), v.clone()));
        }
    }
    Ok(rows)
}

pub(in crate::hir_eval) fn as_bool(v: &V) -> NumericResult<bool> {
    if let V::Bool(b) = v {
        Ok(*b)
    } else {
        Err("expected scalar Boolean")
    }
}
pub(in crate::hir_eval) fn as_seq(v: &V) -> NumericResult<&[V]> {
    if let V::Sequence(xs) = v {
        Ok(xs)
    } else {
        Err("expected a sequence")
    }
}
