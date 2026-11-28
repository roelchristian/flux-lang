//! Runtime values and operations on their structural representation.
use super::{array, engine::Failure, Exact, Wide};
use crate::{
    hir::{CoreOp, ExprId, TableSchema},
    resolve::SymbolId,
    types::Type,
};
use std::rc::Rc;
mod cells;
mod collections;
mod compare;
pub(super) mod display;
pub(super) use cells::{read_cell, write_cell};
pub(super) use collections::{
    as_bool, as_seq, collection, column_rows, layout, named, projection, wrap,
};
pub(super) use compare::{equal, key, key_with_budget, order, Key, KEY_WORK_LIMIT};

#[derive(Clone)]
pub(in crate::hir_eval) enum V {
    Exact(Exact),
    Wide(Rc<Wide>),
    Real(f64),
    Bool(bool),
    String(Rc<str>),
    Char(char),
    Unit,
    Tuple(Rc<[V]>),
    Sequence(Rc<[V]>),
    Dense(Rc<array::Dense>),
    Record(Rc<[(Box<str>, V)]>),
    Table(Rc<[V]>, Rc<TableSchema>),
    Dict(Rc<[(V, V)]>),
    Optional(Option<Rc<V>>),
    Variant(Rc<VariantValue>),
    // Keep exceptional provenance off all ordinary numeric/collection values.
    Outcome(Result<Rc<V>, Rc<Failure>>),
    Fun(Rc<Fun>),
    Action(Rc<Action>),
}
/// Keep sums off the hot path for ordinary sequence element layouts.
pub(in crate::hir_eval) struct VariantValue {
    pub(in crate::hir_eval) constructor: SymbolId,
    pub(in crate::hir_eval) name: Rc<str>,
    pub(in crate::hir_eval) fields: Rc<[V]>,
}

#[derive(Clone)]
pub(in crate::hir_eval) enum Function {
    User {
        formals: Rc<[SymbolId]>,
        body: ExprId,
        env: usize,
        declared: Type,
        open_rows: bool,
    },
    Core(CoreOp, Type, Option<Rc<TableSchema>>),
    Constructor(SymbolId, usize),
    Projection(Box<str>),
}
#[derive(Clone)]
pub(in crate::hir_eval) struct Fun {
    pub(in crate::hir_eval) code: Function,
    pub(in crate::hir_eval) bound: Vec<V>,
    // Types come from checked expressions/signatures, never from Value tags.
    pub(in crate::hir_eval) bound_types: Vec<Type>,
    pub(in crate::hir_eval) instance: Option<Type>,
}
pub(in crate::hir_eval) fn function(code: Function) -> V {
    V::Fun(Rc::new(Fun {
        code,
        bound: Vec::new(),
        bound_types: Vec::new(),
        instance: None,
    }))
}
#[derive(Clone)]
pub(in crate::hir_eval) enum Action {
    Block { expr: ExprId, env: usize },
    IoRead(Rc<str>),
    IoWrite(Rc<str>, Rc<str>),
    IoExists(Rc<str>),
    IoPrint(V),
    CsvRead(Rc<str>, Option<Rc<TableSchema>>),
    CsvWrite(Rc<str>, V),
    Library(CoreOp, Vec<V>),
}
