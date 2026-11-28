//! Stable grouping with an optional checked, column-backed key path.
use crate::hir_eval::{
    error, key_with_budget, layout, resources, wrap, Collection, Eval, Function, Key, Rc, Span,
    Table, TableSchema, KEY_WORK_LIMIT, R, V,
};
use std::collections::HashMap;

const ROW_LIMIT: usize = 8192;

struct Ordered<T> {
    entries: Vec<(V, T)>,
    index: HashMap<Key, usize>,
    budget: usize,
}

impl<T> Ordered<T> {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: HashMap::new(),
            budget: KEY_WORK_LIMIT,
        }
    }

    // Retain the first representative as well as first-occurrence order.
    // In particular, repr (signed zero, decimal scale) is not the hash key.
    fn bucket(&mut self, representative: V, new: impl FnOnce() -> T) -> Option<usize> {
        let key = key_with_budget(&representative, &mut self.budget, 0)?;
        Some(*self.index.entry(key).or_insert_with(|| {
            let index = self.entries.len();
            self.entries.push((representative, new()));
            index
        }))
    }
}

type Groups = Vec<(V, GroupRows)>;

enum GroupRows {
    Values(Vec<V>),
    View(V),
}

impl GroupRows {
    fn wrap(self, schema: Option<Rc<TableSchema>>) -> V {
        match self {
            Self::Values(values) => wrap(values, schema),
            Self::View(view) => view,
        }
    }
}

fn row_groups(eval: &mut Eval<'_>, f: V, rows: &Collection, span: Span) -> R<Groups> {
    let mut groups = Ordered::new();
    for row in rows.iter() {
        let key = eval.apply(f.clone(), vec![row.clone()], span)?;
        let index = groups
            .bucket(key, || GroupRows::Values(Vec::new()))
            .ok_or_else(|| {
                error(
                    span,
                    "group key exceeds supported hash complexity or resource limit",
                )
            })?;
        let GroupRows::Values(items) = &mut groups.entries[index].1 else {
            unreachable!("ordinary groups are materialized rows")
        };
        items.push(row);
    }
    Ok(groups.entries)
}

struct Bounds {
    first: usize,
    last: usize,
    len: usize,
}

enum Selection {
    Slice { start: usize, end: usize },
    Gather(Vec<usize>, resources::MemoryReservation),
}

pub(in crate::hir_eval) fn key_slot(f: &V, table: &Table) -> Option<usize> {
    let V::Fun(function) = f else { return None };
    if !function.bound.is_empty() {
        return None;
    }
    let Function::Projection(field) = &function.code else {
        return None;
    };
    match table.schema().as_ref() {
        TableSchema::Named(fields) => fields.iter().position(|(name, _)| name == field.as_ref()),
        TableSchema::Positional(fields) => field
            .parse::<usize>()
            .ok()?
            .checked_sub(1)
            .filter(|&index| index < fields.len()),
    }
}

fn column_groups(key: &V, table: &Table, span: Span) -> Option<Groups> {
    let slot = key_slot(key, table)?;
    // A bare projection on a checked column is total, strict and costs
    // exactly one apply tick. Plan all leases/hash work without changing
    // Eval, so any decline (including key budget exhaustion) can replay
    // the original applications at the precise first failing row.
    let membership_memory = resources::reserve_for::<usize>(table.len()).ok()?;
    let mut membership = Vec::new();
    membership.try_reserve_exact(table.len()).ok()?;
    let mut groups = Ordered::new();
    for row in 0..table.len() {
        let index = groups.bucket(table.cell(row, slot)?, || Bounds {
            first: row,
            last: row,
            len: 0,
        })?;
        let bounds = &mut groups.entries[index].1;
        bounds.last = row;
        bounds.len += 1;
        membership.push(index);
    }
    let mut selections = Vec::with_capacity(groups.entries.len());
    for (_, bounds) in &groups.entries {
        let end = bounds.last + 1;
        selections.push(if end - bounds.first == bounds.len {
            Selection::Slice {
                start: bounds.first,
                end,
            }
        } else {
            let memory = resources::reserve_for::<usize>(bounds.len).ok()?;
            let mut indices = Vec::new();
            indices.try_reserve_exact(bounds.len).ok()?;
            Selection::Gather(indices, memory)
        });
    }
    for (row, index) in membership.into_iter().enumerate() {
        if let Selection::Gather(indices, _) = &mut selections[index] {
            indices.push(row);
        }
    }
    drop(membership_memory);

    let views: Vec<_> = groups
        .entries
        .into_iter()
        .zip(selections)
        .map(|((key, _), selection)| {
            let view = match selection {
                Selection::Slice { start, end } => table.sliced(start, end),
                Selection::Gather(indices, memory) => table.gather(indices, memory, span).ok()?,
            };
            Some((key, GroupRows::View(view)))
        })
        .collect::<Option<_>>()?;
    Some(views)
}

pub(in crate::hir_eval) fn apply(
    eval: &mut Eval<'_>,
    key: V,
    summarize: Option<V>,
    rows: Collection,
    schema: Option<&Rc<TableSchema>>,
    span: Span,
) -> R<V> {
    if rows.len() > ROW_LIMIT {
        return Err(error(span, "prototype grouping limit (8192 rows) exceeded"));
    }
    let source_layout = rows.schema().cloned();
    let planned = match &rows {
        Collection::Table(table)
            if resources::optimized() && table.columnar() && table.len() <= eval.fuel =>
        {
            column_groups(&key, table, span)
        }
        _ => None,
    };
    let groups = if let Some(groups) = planned {
        eval.charge_work(rows.len(), span)?;
        groups
    } else {
        row_groups(eval, key, &rows, span)?
    };
    if let Some(f) = summarize {
        // Do not move or specialize arbitrary summaries. They are two
        // strict applications after *all* keys, in first-group order.
        let schema = layout(schema, span)?;
        let mut out = Vec::with_capacity(groups.len());
        for (key, items) in groups {
            let partial = eval.apply(f.clone(), vec![key], span)?;
            out.push(eval.apply(partial, vec![items.wrap(source_layout.clone())], span)?);
        }
        Ok(Table::from_rows(out.into(), schema))
    } else {
        Ok(V::Dict(
            groups
                .into_iter()
                .map(|(key, items)| (key, items.wrap(source_layout.clone())))
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hir_eval::{
            exact_i128, function, repr, run_with_options, Domain, ResourcePolicy, RuntimeOptions,
        },
        semantic::{self, Mode, Policy},
        types::{Prim, Type},
    };

    fn checked(source: &str) -> crate::hir::HirProgram {
        let parsed = crate::parser::parse(source).unwrap();
        semantic::prepare(&parsed, Mode::Script, Policy::TypedOnly)
            .unwrap()
            .hir()
            .unwrap()
            .clone()
    }

    #[test]
    fn checked_group_views_retain_representative_and_gather_in_logical_order() {
        let _scope = resources::enter(RuntimeOptions::default()).unwrap();
        let s = Span::default();
        let some = |real| V::Optional(Some(Rc::new(V::Real(real))));
        let record = V::Record(Rc::from([
            (
                "key".into(),
                V::Sequence(Rc::from([
                    some(-0.0),
                    V::Optional(None),
                    some(0.0),
                    some(3.0),
                    V::Optional(None),
                ])),
            ),
            (
                "id".into(),
                V::Sequence((1..=5).map(|n| exact_i128(n, Domain::Integer)).collect()),
            ),
        ]));
        let schema = Rc::new(TableSchema::Named(vec![
            ("key".into(), Type::Optional(Box::new(Type::Prim(Prim::R)))),
            ("id".into(), Type::Prim(Prim::Z)),
        ]));
        let Some(Ok(V::Table(table))) = Table::from_column_record(&record, schema.clone(), s)
        else {
            panic!("column table")
        };
        let Ok(V::Table(gathered)) = table.gather(
            vec![0, 4, 2, 1, 3],
            resources::reserve_for::<usize>(5).unwrap(),
            s,
        ) else {
            panic!("selected column table")
        };
        let key = function(Function::Projection("key".into()));
        let before = resources::available_memory();
        let groups = column_groups(&key, &gathered, s).unwrap();
        let representatives: Vec<_> = groups.iter().map(|(rep, _)| repr(rep)).collect();
        assert_eq!(representatives, ["?1 -0", "?0", "?1 3"]);
        for ((_, items), ids) in groups.iter().zip([&[1, 3][..], &[5, 2], &[4]]) {
            let GroupRows::View(V::Table(view)) = items else {
                panic!("expected a checked view")
            };
            assert!(view.columnar());
            assert_eq!(view.schema(), &schema);
            assert_eq!(view.len(), ids.len());
            for (i, id) in ids.iter().enumerate() {
                assert_eq!(repr(&view.cell(i, 1).unwrap()), id.to_string());
            }
        }
        // Only the two noncontiguous result groups retain index buffers.
        assert_eq!(
            resources::available_memory(),
            before - 4 * size_of::<usize>()
        );
        drop(groups);
        assert_eq!(resources::available_memory(), before);
        let tight = resources::reserve(before - 1).unwrap();
        assert!(column_groups(&key, &gathered, s).is_none());
        drop(tight);
        assert_eq!(resources::available_memory(), before);
    }

    #[test]
    fn grouping_and_curried_aggregation_preserve_provenance_under_work_cutoffs() {
        let p = checked(
            r#"
            << "df";
            t := #: .{
                key := [1.20d 2d 1.200d 1.2d 2.00d 0.00d];
                id := [10z 20z 30z 40z 50z 60z];
            };
            selected := rev (?: [1b 1b 1b 0b 1b 1b] t);
            summ(k:d,rs:#.{key:d; id:z;}) :=
                .{ key := k; count := # rs; items := .id rs; total := +/ .id rs; };
            !^{
                ! print :# (df.agg &.key &summ selected);
                ! print :# (df.agg &.key &summ ([: 0n selected));
                ! print =: &.1 #[ (1z,"a") (2z,"b") (1z,"c") ];
            };
        "#,
        );
        let attempt = |optimize, resources| {
            run_with_options(
                &p,
                RuntimeOptions {
                    resources,
                    optimize,
                },
            )
            .map(|out| out.stdout)
            .map_err(|err| (err.structured().unwrap().primary, err.message))
        };
        let default = ResourcePolicy::default();
        let oracle = attempt(false, default).unwrap();
        assert_eq!(
            oracle,
            concat!(
                "[{key := 0.00; count := 1; items := [60]; total := 60} ",
                "{key := 2.00; count := 2; items := [50 20]; total := 70} ",
                "{key := 1.200; count := 2; items := [30 10]; total := 40}]\n",
                "[]\n#{1 : #[(1, a) (1, c)]; 2 : #[(2, b)]}\n",
            )
        );
        for max_work in 1..200 {
            let limits = ResourcePolicy {
                max_work,
                ..default
            };
            assert_eq!(attempt(true, limits), attempt(false, limits), "{limits:?}");
        }
    }
}
