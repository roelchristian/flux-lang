use super::{ConstructorDef, Definition, ExprId, ExprNode, TableSchema};
use crate::{
    resolve::SymbolId,
    types::{NominalId, Type},
};
use std::{collections::HashMap, rc::Rc};

#[derive(Clone, Debug)]
pub struct HirProgram {
    pub arena: Vec<ExprNode>,
    pub constructors: HashMap<SymbolId, ConstructorDef>,
    pub(super) nominal_schemas: HashMap<NominalId, Rc<TableSchema>>,
    pub(super) standard_constructors: HashMap<(String, String), SymbolId>,
    /// Dependency order. All function symbols in a group are introduced before any initializer.
    pub groups: Vec<Vec<Definition>>,
    pub entry: Option<ExprId>,
}
/// Derive a closed table layout without examining runtime values.
pub(super) fn table_schema(
    ty: &Type,
    nominal: impl FnOnce(&NominalId) -> Option<Rc<TableSchema>>,
) -> Option<Rc<TableSchema>> {
    match ty {
        Type::Action(out) | Type::Result(out, _) | Type::Fun(_, out) => table_schema(out, nominal),
        Type::Table(row) => match row.as_ref() {
            Type::Record(row) if row.tail.is_none() => {
                Some(Rc::new(TableSchema::Named(row.fields.clone())))
            }
            Type::Tuple(fields) => Some(Rc::new(TableSchema::Positional(fields.clone()))),
            Type::Nominal(id) => nominal(id),
            _ => None,
        },
        _ => None,
    }
}

impl HirProgram {
    pub fn standard_constructor(&self, owner: &str, name: &str) -> Option<SymbolId> {
        self.standard_constructors
            .get(&(owner.to_owned(), name.to_owned()))
            .copied()
    }
    /// Layout witness for a checked callable instance, independent of row values.
    pub(crate) fn instance_schema(&self, ty: &Type) -> Option<Rc<TableSchema>> {
        table_schema(ty, |id| self.nominal_schemas.get(id).cloned())
    }
    pub fn node(&self, id: ExprId) -> &ExprNode {
        &self.arena[id.0]
    }
}
