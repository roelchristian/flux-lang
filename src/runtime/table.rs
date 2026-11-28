//! Immutable checked tables. The language schema is independent of layout.
use super::{as_seq, error, invariant, resources, Rc, Span, TableSchema, R, V};
use crate::types::Type;

mod column;
use column::Column;

pub(super) struct Table {
    schema: Rc<TableSchema>,
    storage: Storage,
    rows: Selection,
}

#[derive(Clone)]
enum Storage {
    Rows(Rc<[V]>),
    Columns(Rc<Columns>),
}

struct Columns {
    values: Vec<Rc<Column>>,
    height: usize,
    _memory: resources::MemoryReservation,
}

impl Columns {
    fn with_capacity(height: usize, width: usize) -> Option<Self> {
        let memory = resources::reserve_for::<Rc<Column>>(width).ok()?;
        let mut values = Vec::new();
        values.try_reserve_exact(width).ok()?;
        Some(Self {
            values,
            height,
            _memory: memory,
        })
    }
}

struct Indices {
    values: Vec<usize>,
    _memory: resources::MemoryReservation,
}

#[derive(Clone)]
struct Selection {
    offset: usize,
    len: usize,
    reverse: bool,
    indices: Option<Rc<Indices>>,
}

impl Selection {
    fn all(len: usize) -> Self {
        Self {
            offset: 0,
            len,
            reverse: false,
            indices: None,
        }
    }

    fn identity(&self, backing: usize) -> bool {
        self.indices.is_none() && self.offset == 0 && self.len == backing && !self.reverse
    }

    fn index(&self, row: usize) -> Option<usize> {
        if row >= self.len {
            return None;
        }
        let relative = if self.reverse {
            self.len - 1 - row
        } else {
            row
        };
        let absolute = self.offset + relative;
        match &self.indices {
            Some(indices) => indices.values.get(absolute).copied(),
            None => Some(absolute),
        }
    }

    fn slice(&self, start: usize, end: usize) -> Self {
        Self {
            offset: self.offset + if self.reverse { self.len - end } else { start },
            len: end - start,
            reverse: self.reverse,
            indices: self.indices.clone(),
        }
    }
}

fn width(schema: &TableSchema) -> usize {
    match schema {
        TableSchema::Named(fields) => fields.len(),
        TableSchema::Positional(fields) => fields.len(),
    }
}

fn slot_type(schema: &TableSchema, slot: usize) -> &Type {
    match schema {
        TableSchema::Named(fields) => &fields[slot].1,
        TableSchema::Positional(fields) => &fields[slot],
    }
}

// A structural record may have the same checked field names/types in a
// different order. That remains observable from :# and row callbacks.
fn fits_storage_order(row: &V, schema: &TableSchema) -> bool {
    match (row, schema) {
        (V::Tuple(row), TableSchema::Positional(slots)) => row.len() == slots.len(),
        (V::Record(row), TableSchema::Named(fields)) => row
            .iter()
            .map(|(name, _)| name.as_ref())
            .eq(fields.iter().map(|(name, _)| name.as_str())),
        _ => false,
    }
}

impl Table {
    fn value(schema: Rc<TableSchema>, storage: Storage, height: usize) -> V {
        V::Table(Rc::new(Self {
            schema,
            rows: Selection::all(height),
            storage,
        }))
    }

    pub(super) fn from_rows(rows: Rc<[V]>, schema: Rc<TableSchema>) -> V {
        let height = rows.len();
        Self::value(schema, Storage::Rows(rows), height)
    }

    pub(super) fn from_checked_rows(rows: Rc<[V]>, schema: Rc<TableSchema>) -> V {
        if resources::optimized() && rows.iter().all(|row| fits_storage_order(row, &schema)) {
            let columns = Self::from_checked_cells(schema.clone(), rows.len(), |row, slot| {
                Ok::<_, std::convert::Infallible>(match &rows[row] {
                    V::Record(fields) => fields[slot].1.clone(),
                    V::Tuple(fields) => fields[slot].clone(),
                    _ => unreachable!("checked table storage order"),
                })
            });
            if let Some(Ok(columns)) = columns {
                return columns;
            }
        }
        Self::from_rows(rows, schema)
    }

    /// Build checked cells in logical row order and schema column order,
    /// without allocating typed records. Callers must own a pure decoder
    /// and retain the input for their ordinary path if storage is declined.
    pub(super) fn from_checked_cells<E>(
        schema: Rc<TableSchema>,
        height: usize,
        mut decode: impl FnMut(usize, usize) -> Result<V, E>,
    ) -> Option<Result<V, E>> {
        if !resources::optimized() || height > resources::limits().max_elements {
            return None;
        }
        let width = width(&schema);
        let mut columns = Columns::with_capacity(height, width)?;
        if width == 0 {
            return Some(Ok(Self::value(
                schema,
                Storage::Columns(Rc::new(columns)),
                height,
            )));
        }
        for slot in 0..width {
            columns
                .values
                .push(Rc::new(Column::new(slot_type(&schema, slot), height)?));
        }
        for row in 0..height {
            for (slot, column) in columns.values.iter_mut().enumerate() {
                match decode(row, slot) {
                    Ok(value) => Rc::get_mut(column)
                        .expect("column is exclusive during construction")
                        .push(row, value, height)?,
                    Err(error) => return Some(Err(error)),
                }
            }
        }
        Some(Ok(Self::value(
            schema,
            Storage::Columns(Rc::new(columns)),
            height,
        )))
    }

    /// Direct sharing for explicitly column-shaped named input. None means
    /// the caller must use its ordinary row path, not that a row was lost.
    pub(super) fn from_column_record(
        record: &V,
        schema: Rc<TableSchema>,
        span: Span,
    ) -> Option<R<V>> {
        if !resources::optimized() || !fits_storage_order(record, &schema) {
            return None;
        }
        let V::Record(fields) = record else {
            return None;
        };
        let mut height = None;
        for (_, column) in fields.iter() {
            let xs = match as_seq(column) {
                Ok(xs) => xs,
                Err(why) => return Some(Err(error(span, why))),
            };
            if height.is_some_and(|height| height != xs.len()) {
                return Some(Err(error(span, "record columns have unequal lengths")));
            }
            height = Some(xs.len());
        }
        let height = height.unwrap_or(0);
        if height > resources::limits().max_elements {
            return None;
        }
        let mut columns = Columns::with_capacity(height, fields.len())?;
        for (_, value) in fields.iter() {
            let V::Sequence(xs) = value else {
                unreachable!("validated source columns");
            };
            columns
                .values
                .push(Rc::new(Column::from_shared(xs.clone())?));
        }
        Some(Ok(Self::value(
            schema,
            Storage::Columns(Rc::new(columns)),
            height,
        )))
    }

    /// Reorder, rename, or repeat checked columns without copying their
    /// payloads. Selection and zero-field table height travel with the view.
    pub(super) fn try_project_columns(
        &self,
        target: Rc<TableSchema>,
        fields: &[&str],
    ) -> Option<V> {
        let (Storage::Columns(source), TableSchema::Named(target_fields)) =
            (&self.storage, target.as_ref())
        else {
            return None;
        };
        if target_fields.len() != fields.len() {
            return None;
        }
        let mut columns = Columns::with_capacity(source.height, fields.len())?;
        for ((_, ty), field) in target_fields.iter().zip(fields) {
            let slot = self.slot(field)?;
            let column = source.values.get(slot)?;
            if slot_type(&self.schema, slot) != ty {
                return None;
            }
            columns.values.push(column.clone());
        }
        Some(V::Table(Rc::new(Self {
            schema: target,
            storage: Storage::Columns(Rc::new(columns)),
            rows: self.rows.clone(),
        })))
    }

    pub(super) fn schema(&self) -> &Rc<TableSchema> {
        &self.schema
    }

    pub(super) fn len(&self) -> usize {
        self.rows.len
    }

    pub(super) fn columnar(&self) -> bool {
        matches!(self.storage, Storage::Columns(_))
    }

    fn slot(&self, field: &str) -> Option<usize> {
        match self.schema.as_ref() {
            TableSchema::Named(names) => names.iter().position(|(name, _)| name == field),
            TableSchema::Positional(_) => field.parse::<usize>().ok()?.checked_sub(1),
        }
    }

    pub(super) fn cell(&self, index: usize, slot: usize) -> Option<V> {
        let row = self.rows.index(index)?;
        match &self.storage {
            Storage::Columns(columns) => Some(columns.values.get(slot)?.value(row)),
            Storage::Rows(rows) => match (rows.get(row)?, self.schema.as_ref()) {
                (V::Record(fields), TableSchema::Named(names)) => {
                    let key = &names.get(slot)?.0;
                    fields
                        .iter()
                        .find(|(name, _)| name.as_ref() == key)
                        .map(|(_, v)| v.clone())
                }
                (V::Tuple(values), TableSchema::Positional(_)) => values.get(slot).cloned(),
                _ => None,
            },
        }
    }

    pub(super) fn get(&self, index: usize) -> Option<V> {
        let backing = self.rows.index(index)?;
        Some(match &self.storage {
            Storage::Rows(rows) => rows.get(backing)?.clone(),
            Storage::Columns(columns) => match self.schema.as_ref() {
                TableSchema::Named(names) => V::Record(
                    names
                        .iter()
                        .zip(&columns.values)
                        .map(|((name, _), col)| (name.as_str().into(), col.value(backing)))
                        .collect(),
                ),
                TableSchema::Positional(_) => V::Tuple(
                    columns
                        .values
                        .iter()
                        .map(|col| col.value(backing))
                        .collect(),
                ),
            },
        })
    }

    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = V> + ExactSizeIterator + '_ {
        (0..self.len()).map(|i| self.get(i).expect("checked logical row index"))
    }

    fn view(&self, rows: Selection) -> V {
        V::Table(Rc::new(Self {
            schema: self.schema.clone(),
            storage: self.storage.clone(),
            rows,
        }))
    }

    pub(super) fn sliced(&self, start: usize, end: usize) -> V {
        match &self.storage {
            Storage::Rows(rows) => {
                Self::from_rows(Rc::from(&rows[start..end]), self.schema.clone())
            }
            Storage::Columns(_) => self.view(self.rows.slice(start, end)),
        }
    }

    pub(super) fn reversed(&self) -> V {
        if !self.columnar() {
            return Self::from_rows(self.iter().rev().collect(), self.schema.clone());
        }
        self.view(Selection {
            reverse: !self.rows.reverse,
            ..self.rows.clone()
        })
    }

    // Indices are checked in logical order, then rebased once into the
    // backing storage. Repeated filtering cannot create chains of maps.
    pub(super) fn gather(
        &self,
        mut indices: Vec<usize>,
        memory: resources::MemoryReservation,
        span: Span,
    ) -> R<V> {
        for index in &mut indices {
            *index = self
                .rows
                .index(*index)
                .ok_or_else(|| invariant(span, "invalid checked table selection index"))?;
        }
        Ok(self.view(Selection {
            len: indices.len(),
            offset: 0,
            reverse: false,
            indices: Some(Rc::new(Indices {
                values: indices,
                _memory: memory,
            })),
        }))
    }

    pub(super) fn project(&self, field: &str, span: Span) -> Option<R<V>> {
        let Storage::Columns(columns) = &self.storage else {
            return None;
        };
        let index = self.slot(field)?;
        let column = columns.values.get(index)?;
        if self.rows.identity(columns.height) {
            if let Some(shared) = column.shared() {
                return Some(Ok(V::Sequence(shared.clone())));
            }
        }
        Some(self.project_view(column, span))
    }

    fn project_view(&self, column: &Column, span: Span) -> R<V> {
        let scratch = self
            .len()
            .checked_mul(2)
            .ok_or_else(|| error(span, "table column projection exceeds addressable size"))?;
        let _memory = resources::reserve_for::<V>(scratch).map_err(|why| error(span, why))?;
        let output = (0..self.len())
            .map(|i| column.value(self.rows.index(i).expect("checked column row index")))
            .collect();
        Ok(V::Sequence(output))
    }

    pub(super) fn materialize_rows(&self, span: Span) -> R<Rc<[V]>> {
        if let Storage::Rows(rows) = &self.storage {
            if self.rows.identity(rows.len()) {
                return Ok(rows.clone());
            }
        }
        let too_large = || error(span, "table row materialization exceeds addressable size");
        let row_bytes = match self.schema.as_ref() {
            TableSchema::Named(names) => names.iter().try_fold(size_of::<V>(), |sum, (name, _)| {
                sum.checked_add(size_of::<(Box<str>, V)>())?
                    .checked_add(name.len())
            }),
            TableSchema::Positional(slots) => slots
                .len()
                .checked_add(1)
                .and_then(|slots| slots.checked_mul(size_of::<V>())),
        };
        let bytes = row_bytes
            .and_then(|bytes| bytes.checked_mul(self.len())?.checked_mul(2))
            .ok_or_else(too_large)?;
        let _memory = resources::reserve(bytes).map_err(|why| error(span, why))?;
        Ok(self.iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hir_eval::{exact_i128, repr, Domain, RuntimeOptions},
        types::{Prim, Type},
    };

    fn table(value: &V) -> &Table {
        let V::Table(value) = value else {
            panic!("expected a table");
        };
        value
    }

    fn column(name: &str, cells: &[i128]) -> (Box<str>, V) {
        (
            name.into(),
            V::Sequence(
                cells
                    .iter()
                    .map(|n| exact_i128(*n, Domain::Integer))
                    .collect(),
            ),
        )
    }

    #[test]
    fn ordered_column_input_and_steps_share_the_same_backing() {
        let _scope = resources::enter(RuntimeOptions::default()).unwrap();
        let originals = V::Record(Rc::from([
            column("name", &[11, 12, 13, 14]),
            column("cost", &[21, 22, 23, 24]),
        ]));
        let schema = Rc::new(TableSchema::Named(vec![
            ("name".into(), Type::Prim(Prim::Z)),
            ("cost".into(), Type::Prim(Prim::Z)),
        ]));
        let value = Table::from_column_record(&originals, schema, Span::default())
            .expect("recognized checked columns")
            .ok()
            .expect("column storage");
        let t = table(&value);
        let Storage::Columns(backing) = &t.storage else {
            panic!("expected column storage");
        };
        let column = t.project("cost", Span::default()).unwrap().ok().unwrap();
        let V::Sequence(column) = column else {
            panic!("expected projected sequence");
        };
        assert!(Rc::ptr_eq(&column, backing.values[1].shared().unwrap()));

        let reservation = resources::reserve_for::<usize>(3).unwrap();
        let gathered = t
            .gather(vec![3, 0, 2], reservation, Span::default())
            .ok()
            .unwrap();
        let reversed = table(&gathered).reversed();
        let cut = table(&reversed).sliced(1, 3);
        let t = table(&cut);
        let Storage::Columns(selected) = &t.storage else {
            panic!("expected selected column storage");
        };
        assert!(Rc::ptr_eq(backing, selected));
        assert_eq!(
            repr(&t.project("cost", Span::default()).unwrap().ok().unwrap()),
            "[21 24]",
        );
        assert_eq!(t.len(), 2);

        let renamed_schema = Rc::new(TableSchema::Named(vec![
            ("first".into(), Type::Prim(Prim::Z)),
            ("second".into(), Type::Prim(Prim::Z)),
        ]));
        let renamed = t
            .try_project_columns(renamed_schema, &["cost", "cost"])
            .expect("checked repeated projection");
        let Storage::Columns(renamed_columns) = &table(&renamed).storage else {
            panic!("projection must remain columnar");
        };
        assert!(Rc::ptr_eq(&renamed_columns.values[0], &backing.values[1]));
        assert!(Rc::ptr_eq(&renamed_columns.values[1], &backing.values[1]));
        assert_eq!(repr(&table(&renamed).cell(1, 1).unwrap()), "24");

        let zero = t
            .try_project_columns(Rc::new(TableSchema::Named(vec![])), &[])
            .expect("checked zero-field projection");
        assert_eq!(table(&zero).len(), 2);
        assert_eq!(repr(&zero), "#[{} {}]");
    }

    #[test]
    fn from_rows_preserves_height_without_columns_and_row_order_fallback() {
        let _scope = resources::enter(RuntimeOptions::default()).unwrap();
        let empty = V::Record(Rc::from([]));
        let zero_width = Table::from_checked_rows(
            Rc::from([empty.clone(), empty]),
            Rc::new(TableSchema::Named(vec![])),
        );
        assert!(table(&zero_width).columnar());
        assert_eq!(table(&zero_width).len(), 2);
        assert_eq!(repr(&zero_width), "#[{} {}]");
        assert_eq!(
            repr(&V::Sequence(
                table(&zero_width)
                    .materialize_rows(Span::default())
                    .ok()
                    .unwrap()
            )),
            "[{} {}]"
        );

        let row = V::Record(Rc::from([
            ("b".into(), exact_i128(2, Domain::Integer)),
            ("a".into(), exact_i128(1, Domain::Integer)),
        ]));
        let schema = Rc::new(TableSchema::Named(vec![
            ("a".into(), Type::Prim(Prim::Z)),
            ("b".into(), Type::Prim(Prim::Z)),
        ]));
        let fallback = Table::from_checked_rows(Rc::from([row]), schema);
        assert!(!table(&fallback).columnar());
        assert_eq!(repr(&table(&fallback).get(0).unwrap()), "{b := 2; a := 1}");
    }

    #[test]
    fn reference_mode_keeps_row_storage() {
        let _scope = resources::enter(RuntimeOptions {
            optimize: false,
            ..RuntimeOptions::default()
        })
        .unwrap();
        let row = V::Tuple(Rc::from([exact_i128(5, Domain::Integer)]));
        let value = Table::from_checked_rows(
            Rc::from([row]),
            Rc::new(TableSchema::Positional(vec![Type::Prim(Prim::Z)])),
        );
        assert!(!table(&value).columnar());
    }
}
