//! Checked AST identity, rebinding and edit detection.

use crate::parser::{Expr, Program, Stmt, SyntaxId};
use std::collections::HashMap;

// Exact (not hashed) guard against using checked metadata on an edited tree.
#[derive(Clone, Debug)]
pub(crate) struct ProgramShape {
    defs: Vec<Stmt>,
    entry: Option<Expr>,
    entry_start: Option<usize>,
    imports: Vec<String>,
    constraints: std::collections::BTreeMap<usize, Vec<(String, String)>>,
}
impl ProgramShape {
    pub(super) fn new(p: &Program) -> Self {
        Self {
            defs: p.defs.clone(),
            entry: p.entry.clone(),
            entry_start: p.entry_start,
            imports: p.imports.clone(),
            constraints: p.constraints.clone(),
        }
    }
    pub(crate) fn matches(&self, p: &Program) -> bool {
        self.entry_start == p.entry_start
            && self.imports == p.imports
            && self.constraints == p.constraints
            && self.defs == p.defs
            && self.entry == p.entry
    }
}

// Compatibility locator for borrowed AST APIs; only this map uses memory
// identity. No symbol/type/origin/evidence table is keyed by an address.
#[derive(Clone, Debug, Default)]
pub(crate) struct SyntaxLocator(HashMap<usize, SyntaxId>);
impl SyntaxLocator {
    pub(crate) fn id(&self, node: &Expr) -> Option<SyntaxId> {
        self.0.get(&(std::ptr::from_ref(node) as usize)).copied()
    }
    pub(super) fn matches(&self, program: &Program) -> bool {
        program
            .identified_expressions()
            .all(|(id, expr)| self.id(expr) == Some(id))
    }

    pub(super) fn new(program: &Program) -> Self {
        Self(
            program
                .identified_expressions()
                .map(|(id, e)| (std::ptr::from_ref(e) as usize, id))
                .collect(),
        )
    }
}
