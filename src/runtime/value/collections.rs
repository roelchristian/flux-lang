//! Checked value accessors, table layout and row/column conversion.
use super::{Table, V};
use crate::hir::{Span, TableSchema};
use crate::hir_eval::{error, resources, NumericResult, R};
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
        V::Sequence(xs) => Ok(V::Sequence(
            xs.iter()
                .cloned()
                .map(|x| projection(field, x, span))
                .collect::<R<Vec<_>>>()?
                .into(),
        )),
        V::Table(table) => {
            if let Some(column) = table.project(field, span) {
                return column;
            }
            Ok(V::Sequence(
                table
                    .iter()
                    .map(|row| projection(field, row, span))
                    .collect::<R<Vec<_>>>()?
                    .into(),
            ))
        }
        _ => Err(error(
            span,
            "projection expects a row or structured collection",
        )),
    }
}

pub(in crate::hir_eval) enum Collection {
    Sequence(Rc<[V]>),
    Table(Rc<Table>),
}

impl Collection {
    pub(in crate::hir_eval) fn schema(&self) -> Option<&Rc<TableSchema>> {
        match self {
            Self::Sequence(_) => None,
            Self::Table(table) => Some(table.schema()),
        }
    }

    pub(in crate::hir_eval) fn len(&self) -> usize {
        match self {
            Self::Sequence(items) => items.len(),
            Self::Table(table) => table.len(),
        }
    }

    pub(in crate::hir_eval) fn get(&self, index: usize) -> Option<V> {
        match self {
            Self::Sequence(items) => items.get(index).cloned(),
            Self::Table(table) => table.get(index),
        }
    }

    pub(in crate::hir_eval) fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = V> + ExactSizeIterator + '_ {
        (0..self.len()).map(|i| self.get(i).expect("checked collection index"))
    }
}

pub(in crate::hir_eval) fn collection(v: V, span: Span) -> R<Collection> {
    match v {
        V::Sequence(xs) => Ok(Collection::Sequence(xs)),
        V::Table(table) => Ok(Collection::Table(table)),
        _ => Err(error(span, "expected sequence or table")),
    }
}
pub(in crate::hir_eval) fn wrap(xs: Vec<V>, s: Option<Rc<TableSchema>>) -> V {
    if let Some(s) = s {
        Table::from_rows(xs.into(), s)
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
    let height = n.unwrap_or(0);
    if height > resources::limits().max_elements {
        return Err(error(span, "table row construction exceeds the row limit"));
    }
    let bytes_per_row = fields
        .iter()
        .try_fold(size_of::<RowCells>(), |bytes, (name, _)| {
            bytes
                .checked_add(size_of::<(Box<str>, V)>())?
                .checked_add(name.len())
        });
    let bytes = bytes_per_row
        .and_then(|bytes| bytes.checked_mul(height))
        .ok_or_else(|| error(span, "table row construction exceeds addressable size"))?;
    let _scratch = resources::reserve(bytes).map_err(|message| error(span, message))?;
    let mut rows = (0..height)
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
