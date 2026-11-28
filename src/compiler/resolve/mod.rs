//! Lexical identities and dependency ordering for a parsed Flux program.
use crate::{
    parser::{Expr, MatchPattern, Program, RecoveryCase, Stmt, SyntaxId, Variant},
    standard_errors, Error,
};
use std::{borrow::Cow, collections::HashMap, ops::Range, sync::Arc};

mod dependencies;
mod eager;
mod identity;
mod scopes;
#[cfg(test)]
mod tests;

pub(crate) use identity::{ProgramShape, SyntaxLocator};
use scopes::resolve_impl;

/// Deterministic within the same parsed syntax tree; unrelated edits may renumber IDs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SymbolId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    Value,
    Callable(usize),
    Constructor(usize),
    Schema,
    Parameter,
    Stage,
    External,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub start: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Resolution {
    pub(crate) syntax: Arc<SyntaxLocator>,
    pub(crate) program_shape: Option<Arc<ProgramShape>>,
    /// Program.defs indices in dependency order. A recursive group consists only
    /// of named function declarations. Its members are in source order.
    pub groups: Vec<Vec<usize>>,
    pub top: Vec<Option<SymbolId>>,
    pub symbols: Vec<Symbol>,
    refs: HashMap<SyntaxId, SymbolId>,
    defs: HashMap<usize, SymbolId>,
    params: HashMap<usize, Vec<SymbolId>>,
    lambdas: HashMap<SyntaxId, Vec<SymbolId>>,
    placeholders: HashMap<SyntaxId, SymbolId>,
    offsets: HashMap<SyntaxId, usize>,
    spans: HashMap<SyntaxId, Range<usize>>,
    owners: HashMap<SymbolId, SymbolId>,
    variants: HashMap<usize, SymbolId>,
    recovery_symbols: HashMap<usize, Vec<SymbolId>>,
    recovery_ctors: HashMap<usize, SymbolId>,
    match_bindings: HashMap<usize, SymbolId>,
    match_constructors: HashMap<usize, SymbolId>,
    standard: Vec<(&'static str, &'static str, SymbolId)>,
}

impl Resolution {
    /// Stable metadata can be reused with an exact clone or a compatible
    /// reparse. Declaration and pattern positions are still part of the
    /// equality guard; offset-shifting edits require fresh resolution.
    /// Only the borrowed-expression adapter is reindexed here.
    pub(crate) fn for_program<'a>(&'a self, program: &Program) -> Option<Cow<'a, Self>> {
        if !self.program_shape.as_ref()?.matches(program) {
            return None;
        }
        if self.syntax.matches(program) {
            return Some(Cow::Borrowed(self));
        }
        let mut resolution = self.clone();
        resolution.syntax = Arc::new(SyntaxLocator::new(program));
        resolution.spans = program
            .identified_origins()
            .into_iter()
            .map(|(id, _, span)| (id, span))
            .collect();
        Some(Cow::Owned(resolution))
    }

    /// Locate one occurrence in the same live AST passed to `resolve`.
    /// Across a clone/reparse, use the deterministic Program syntax index.
    pub fn syntax_id(&self, expr: &Expr) -> Option<SyntaxId> {
        self.syntax.id(expr)
    }
    pub fn reference(&self, expr: &Expr) -> Option<SymbolId> {
        self.reference_at(self.syntax_id(expr)?)
    }
    pub fn reference_at(&self, id: SyntaxId) -> Option<SymbolId> {
        self.refs.get(&id).copied()
    }
    pub fn span_at(&self, id: SyntaxId) -> Option<Range<usize>> {
        self.spans.get(&id).cloned()
    }
    /// Look up a statement in this resolution's parsed source, or an exact
    /// clone. Declaration identity uses its source position; callers must
    /// not supply a statement from an unrelated or edited program.
    pub fn declaration(&self, stmt: &Stmt) -> Option<SymbolId> {
        self.defs.get(&stmt.start()).copied()
    }
    pub fn parameters(&self, stmt: &Stmt) -> &[SymbolId] {
        self.params.get(&stmt.start()).map_or(&[], Vec::as_slice)
    }
    pub fn lambda_parameters(&self, lambda: &Expr) -> &[SymbolId] {
        self.syntax_id(lambda)
            .and_then(|id| self.lambdas.get(&id))
            .map_or(&[], Vec::as_slice)
    }
    /// Symbol for the preceding stage's value, visible within this stage.
    pub fn placeholder(&self, stage: &Expr) -> Option<SymbolId> {
        self.placeholders.get(&self.syntax_id(stage)?).copied()
    }
    pub fn constructor_owner(&self, id: SymbolId) -> Option<SymbolId> {
        self.owners.get(&id).copied()
    }
    pub fn variant(&self, variant: &Variant) -> Option<SymbolId> {
        self.variants.get(&variant.start).copied()
    }
    pub fn recovery_bindings(&self, arm: &RecoveryCase) -> &[SymbolId] {
        self.recovery_symbols
            .get(&arm.start)
            .map_or(&[], Vec::as_slice)
    }
    pub fn recovery_constructor(&self, arm: &RecoveryCase) -> Option<SymbolId> {
        self.recovery_ctors.get(&arm.start).copied()
    }
    pub fn match_binding(&self, pat: &MatchPattern) -> Option<SymbolId> {
        self.match_bindings.get(&pat.span.start).copied()
    }
    pub fn match_constructor(&self, pat: &MatchPattern) -> Option<SymbolId> {
        self.match_constructors.get(&pat.span.start).copied()
    }
    pub fn standard_owner(&self, owner: &str) -> Option<SymbolId> {
        standard_errors::owner_id(owner).map(SymbolId)
    }
    pub fn standard_constructor(&self, owner: &str, name: &str) -> Option<SymbolId> {
        self.standard
            .iter()
            .find(|(o, n, _)| *o == owner && *n == name)
            .map(|x| x.2)
    }
    pub fn standard_constructors(
        &self,
    ) -> impl Iterator<Item = (&'static str, &'static str, SymbolId)> + '_ {
        self.standard.iter().copied()
    }
    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0]
    }
    /// Exact source expression envelope, if present. Synthetic expressions
    /// and manually changed AST trees have no exact source envelope.
    pub fn span(&self, expr: &Expr) -> Option<Range<usize>> {
        self.span_at(self.syntax_id(expr)?)
    }
    /// Exact expression start or nearest declaration/entry fallback.
    pub fn offset(&self, expr: &Expr) -> Option<usize> {
        let id = self.syntax_id(expr)?;
        self.spans
            .get(&id)
            .map(|s| s.start)
            .or_else(|| self.offsets.get(&id).copied())
    }
}

pub fn resolve(p: &Program) -> Result<Resolution, Error> {
    resolve_impl(p, true)
}

pub(crate) fn resolve_live(p: &Program) -> Result<Resolution, Error> {
    resolve_impl(p, false)
}

fn decl_name(s: &Stmt) -> Option<(&str, SymbolKind, usize)> {
    match s {
        Stmt::Def {
            name,
            params,
            start,
            ..
        } => Some((
            name,
            if params.is_empty() {
                SymbolKind::Value
            } else {
                SymbolKind::Callable(params.len())
            },
            *start,
        )),
        Stmt::Schema { name, start, .. } | Stmt::Sum { name, start, .. } => {
            Some((name, SymbolKind::Schema, *start))
        }
        Stmt::Effect {
            name: Some(name),
            start,
            ..
        } => Some((name, SymbolKind::Value, *start)),
        Stmt::Effect { name: None, .. } => None,
    }
}
