//! Lexical identities and dependency ordering for a parsed Flux program.
use crate::parser::{
    standard_owners, Expr, MatchPattern, MatchPatternKind, Program, RecoveryCase, RecoveryPattern,
    Stmt, SyntaxId, Variant,
};
use crate::{builtins, errors041, Error};
use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap},
    ops::Range,
    sync::Arc,
};

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
        self.defs.get(&stmt_start(stmt)).copied()
    }
    pub fn parameters(&self, stmt: &Stmt) -> &[SymbolId] {
        self.params
            .get(&stmt_start(stmt))
            .map_or(&[], Vec::as_slice)
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
        errors041::owner_id(owner).map(SymbolId)
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
    fn new(p: &Program) -> Self {
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
    fn matches(&self, program: &Program) -> bool {
        program
            .identified_expressions()
            .all(|(id, expr)| self.id(expr) == Some(id))
    }

    fn new(program: &Program) -> Self {
        Self(
            program
                .identified_expressions()
                .map(|(id, e)| (std::ptr::from_ref(e) as usize, id))
                .collect(),
        )
    }
}
fn stmt_start(stmt: &Stmt) -> usize {
    match stmt {
        Stmt::Def { start, .. }
        | Stmt::Effect { start, .. }
        | Stmt::Schema { start, .. }
        | Stmt::Sum { start, .. } => *start,
    }
}

pub fn resolve(p: &Program) -> Result<Resolution, Error> {
    resolve_impl(p, true)
}

pub(crate) fn resolve_live(p: &Program) -> Result<Resolution, Error> {
    resolve_impl(p, false)
}

fn resolve_impl(p: &Program, retain_shape: bool) -> Result<Resolution, Error> {
    let mut r = Resolver {
        out: Resolution {
            syntax: Arc::new(SyntaxLocator::new(p)),
            program_shape: retain_shape.then(|| Arc::new(ProgramShape::new(p))),
            ..Resolution::default()
        },
        scopes: Vec::new(),
        external: HashMap::new(),
        standard_visible: HashMap::new(),
        roots: HashMap::new(),
        deps: vec![BTreeSet::new(); p.defs.len()],
        root: None,
        offset: 0,
    };
    r.out.spans = p
        .identified_origins()
        .into_iter()
        .map(|(id, _, span)| (id, span))
        .collect();
    for &owner in errors041::OWNERS {
        if owner != "ioerr"
            && !p
                .imports
                .iter()
                .any(|m| standard_owners(m).contains(&owner))
        {
            continue;
        }
        let owner_id = SymbolId(errors041::owner_id(owner).expect("known standard error type"));
        for v in errors041::variants(owner) {
            let id = r.fresh(v.name, SymbolKind::Constructor(v.payload.len()), 0);
            r.out.owners.insert(id, owner_id);
            r.out.standard.push((owner, v.name, id));
            r.standard_visible.insert(v.name.to_owned(), id);
        }
    }
    r.scopes.push(HashMap::new());
    for (i, decl) in p.defs.iter().enumerate() {
        let id = r.allocate_decl(decl);
        if let Some(id) = id {
            if let Stmt::Sum { variants, .. } = decl {
                r.register_variants(id, variants)?;
            } else {
                r.bind(id)?;
            }
            r.roots.insert(id, i);
        }
        r.out.top.push(id);
    }
    for (i, decl) in p.defs.iter().enumerate() {
        r.root = Some(i);
        r.statement(decl)?;
    }
    r.root = None;
    if let Some(expr) = &p.entry {
        r.offset = p.entry_start.unwrap_or(0);
        r.expression(expr)?;
    }
    r.out.groups = groups(&r.deps, p)?;
    verify_eager_locals(p, &r.out)?;
    Ok(r.out)
}

struct Resolver {
    out: Resolution,
    scopes: Vec<HashMap<String, SymbolId>>,
    external: HashMap<String, SymbolId>,
    standard_visible: HashMap<String, SymbolId>,
    roots: HashMap<SymbolId, usize>,
    deps: Vec<BTreeSet<usize>>,
    root: Option<usize>,
    offset: usize,
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
fn stmt_offset(s: &Stmt) -> usize {
    match s {
        Stmt::Def { start, .. }
        | Stmt::Schema { start, .. }
        | Stmt::Sum { start, .. }
        | Stmt::Effect { start, .. } => *start,
    }
}

impl Resolver {
    fn fresh(&mut self, name: &str, kind: SymbolKind, start: usize) -> SymbolId {
        let id = SymbolId(self.out.symbols.len());
        self.out.symbols.push(Symbol {
            id,
            name: name.to_owned(),
            kind,
            start,
        });
        id
    }
    fn allocate_decl(&mut self, stmt: &Stmt) -> Option<SymbolId> {
        let (name, kind, start) = decl_name(stmt)?;
        let id = self.fresh(name, kind, start);
        self.out.defs.insert(stmt_start(stmt), id);
        Some(id)
    }
    fn register_variants(&mut self, owner: SymbolId, variants: &[Variant]) -> Result<(), Error> {
        for variant in variants {
            let id = self.fresh(
                &variant.name,
                SymbolKind::Constructor(variant.payload.len()),
                variant.start,
            );
            self.bind(id)?;
            self.out.owners.insert(id, owner);
            self.out.variants.insert(variant.start, id);
        }
        Ok(())
    }
    fn bind(&mut self, id: SymbolId) -> Result<(), Error> {
        let sym = self.out.symbol(id);
        let top = self.scopes.last_mut().expect("resolver scope");
        if top.contains_key(&sym.name) {
            return Err(Error::new(
                sym.start,
                format!("duplicate binding `{}`", sym.name),
            ));
        }
        top.insert(sym.name.clone(), id);
        Ok(())
    }
    fn lookup(&self, n: &str) -> Option<SymbolId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(n).copied())
            .or_else(|| self.standard_visible.get(n).copied())
    }
    fn edge(&mut self, id: SymbolId) {
        let id = self.out.constructor_owner(id).unwrap_or(id);
        if let (Some(owner), Some(&dependency)) = (self.root, self.roots.get(&id)) {
            self.deps[owner].insert(dependency);
        }
    }
    fn use_name(&mut self, e: &Expr, n: &str, usage: Usage) -> Result<(), Error> {
        let start = self.out.offset(e).unwrap_or(self.offset);
        let id = if let Some(id) = self.lookup(n) {
            let k = self.out.symbol(id).kind;
            let valid = match (usage, k) {
                (Usage::Plain, SymbolKind::Value | SymbolKind::Parameter | SymbolKind::Stage) => {
                    true
                }
                (Usage::Plain, SymbolKind::Constructor(0)) => true,
                (Usage::Ref, SymbolKind::Callable(_) | SymbolKind::Constructor(1..)) => true,
                (Usage::Record, SymbolKind::Schema) => true,
                (
                    Usage::Call(actual),
                    SymbolKind::Callable(expected) | SymbolKind::Constructor(expected),
                ) => actual == expected,
                _ => false,
            };
            if !valid {
                return Err(Error::new(
                    start,
                    format!("invalid use of `{n}` as {}", usage.description()),
                ));
            }
            id
        } else {
            match usage {
                Usage::Plain | Usage::Record => {
                    return Err(Error::new(start, format!("unknown binding `{n}`")));
                }
                Usage::Call(_) | Usage::Ref => {
                    // The parser already checked qualification. Keep registry
                    // arity validation here for consumers of a manually built AST.
                    let arity = builtins::lookup(n)
                        .map(|b| b.arity)
                        .or_else(|| builtins::runtime_arity(n))
                        .ok_or_else(|| Error::new(start, format!("unknown callable `{n}`")))?;
                    match usage {
                        Usage::Call(k) if k != arity => {
                            return Err(Error::new(
                                start,
                                format!("`{n}` expects {arity} operands, got {k}"),
                            ))
                        }
                        Usage::Ref if arity == 0 => {
                            return Err(Error::new(
                                start,
                                format!("`{n}` has no referenceable operand"),
                            ))
                        }
                        _ => {}
                    }
                    let canonical = builtins::canonical(n);
                    if let Some(&id) = self.external.get(canonical) {
                        id
                    } else {
                        let id = self.fresh(canonical, SymbolKind::External, start);
                        self.external.insert(canonical.to_owned(), id);
                        id
                    }
                }
            }
        };
        self.edge(id);
        self.out
            .refs
            .insert(self.out.syntax_id(e).expect("indexed expression"), id);
        Ok(())
    }
    fn parameter_scope(&mut self, names: &[String]) -> Result<Vec<SymbolId>, Error> {
        self.scopes.push(HashMap::new());
        let mut ids = Vec::with_capacity(names.len());
        for n in names {
            let id = self.fresh(n, SymbolKind::Parameter, self.offset);
            self.bind(id)?;
            ids.push(id);
        }
        Ok(ids)
    }
    fn statement(&mut self, s: &Stmt) -> Result<(), Error> {
        let outer = self.offset;
        self.offset = stmt_offset(s);
        match s {
            Stmt::Schema { .. } | Stmt::Sum { .. } => {}
            Stmt::Def { params, body, .. } if !params.is_empty() => {
                let ids = self.parameter_scope(params)?;
                self.out.params.insert(stmt_start(s), ids);
                self.expression(body)?;
                self.scopes.pop();
            }
            Stmt::Def { body, .. } | Stmt::Effect { body, .. } => self.expression(body)?,
        }
        self.offset = outer;
        Ok(())
    }
    fn block(&mut self, stmts: &[Stmt], tail: Option<&Expr>) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        // Allocate all declarations in deterministic source order. Only callables
        // and schemas enter scope before the declaration site.
        for s in stmts {
            if let Some(id) = self.allocate_decl(s) {
                if let Stmt::Sum { variants, .. } = s {
                    self.register_variants(id, variants)?;
                } else if matches!(
                    self.out.symbol(id).kind,
                    SymbolKind::Callable(_) | SymbolKind::Schema
                ) {
                    self.bind(id)?;
                }
            }
        }
        for s in stmts {
            self.statement(s)?;
            if let Some(id) = self.out.declaration(s) {
                if self.out.symbol(id).kind == SymbolKind::Value {
                    self.bind(id)?;
                }
            }
        }
        if let Some(e) = tail {
            self.expression(e)?;
        }
        self.scopes.pop();
        Ok(())
    }
    fn collect_pattern<'p>(
        &mut self,
        pat: &'p MatchPattern,
        bindings: &mut Vec<(&'p str, usize)>,
    ) -> Result<(), Error> {
        match &pat.kind {
            MatchPatternKind::Bind(name) => bindings.push((name, pat.span.start)),
            MatchPatternKind::Constructor(name, args) => {
                let id = self
                    .lookup(name)
                    .filter(|&id| matches!(self.out.symbol(id).kind, SymbolKind::Constructor(n) if n == args.len()))
                    .ok_or_else(|| Error::new(pat.span.start, format!("unknown match constructor or arity mismatch for `{name}`")))?;
                self.edge(id);
                self.out.match_constructors.insert(pat.span.start, id);
                for arg in args {
                    self.collect_pattern(arg, bindings)?;
                }
            }
            MatchPatternKind::Tuple(items) => {
                for item in items {
                    self.collect_pattern(item, bindings)?;
                }
            }
            MatchPatternKind::Record(fields) => {
                for (_, field) in fields {
                    self.collect_pattern(field, bindings)?;
                }
            }
            MatchPatternKind::Optional(Some(payload)) => self.collect_pattern(payload, bindings)?,
            MatchPatternKind::Optional(None)
            | MatchPatternKind::Wildcard
            | MatchPatternKind::Unit
            | MatchPatternKind::Bool(_) => {}
        }
        Ok(())
    }
    fn expression(&mut self, e: &Expr) -> Result<(), Error> {
        self.out.offsets.insert(
            self.out.syntax_id(e).expect("indexed expression"),
            self.offset,
        );
        match e {
            Expr::Name(n) => self.use_name(e, n, Usage::Plain)?,
            Expr::Ref(n) => self.use_name(e, n, Usage::Ref)?,
            Expr::Call(n, args) => {
                self.use_name(e, n, Usage::Call(args.len()))?;
                for a in args {
                    self.expression(a)?;
                }
            }
            Expr::Tuple(es) | Expr::Array(es) => {
                for a in es {
                    self.expression(a)?;
                }
            }
            Expr::Cases(arms, fallback) => {
                for (cond, branch) in arms {
                    self.expression(cond)?;
                    self.expression(branch)?;
                }
                self.expression(fallback)?;
            }
            Expr::Lambda(params, body) | Expr::TypedLambda(params, _, body) => {
                let ids = self.parameter_scope(params)?;
                self.out
                    .lambdas
                    .insert(self.out.syntax_id(e).expect("indexed expression"), ids);
                self.expression(body)?;
                self.scopes.pop();
            }
            Expr::Pipeline(stages) => {
                if let Some((first, rest)) = stages.split_first() {
                    self.expression(first)?;
                    for stage in rest {
                        let id = self.fresh("$", SymbolKind::Stage, self.offset);
                        self.out
                            .placeholders
                            .insert(self.out.syntax_id(stage).expect("indexed stage"), id);
                        self.scopes.push(HashMap::new());
                        self.bind(id)?;
                        self.expression(stage)?;
                        self.scopes.pop();
                    }
                }
            }
            Expr::NamedRecord(n, fields) => {
                self.use_name(e, n, Usage::Record)?;
                for (_, v) in fields {
                    self.expression(v)?;
                }
            }
            Expr::Record(fields) => {
                for (_, v) in fields {
                    self.expression(v)?;
                }
            }
            Expr::Raise(body) | Expr::Propagate(body) => self.expression(body)?,
            Expr::Recover(body, arms) => {
                self.expression(body)?;
                for arm in arms {
                    let bindings = match &arm.pattern {
                        RecoveryPattern::Wildcard => Vec::new(),
                        RecoveryPattern::Bind(name) => vec![name.clone()],
                        RecoveryPattern::Constructor(name, slots) => {
                            let id=self.lookup(name).filter(|&id|matches!(self.out.symbol(id).kind,SymbolKind::Constructor(n) if n==slots.len()))
                                .ok_or_else(||Error::new(arm.start,format!("unknown recovery constructor or arity mismatch for `{name}`")))?;
                            self.edge(id);
                            self.out.recovery_ctors.insert(arm.start, id);
                            slots
                                .iter()
                                .filter(|s| s.as_str() != "_")
                                .cloned()
                                .collect()
                        }
                    };
                    let saved = self.offset;
                    self.offset = arm.start;
                    let ids = self.parameter_scope(&bindings)?;
                    self.offset = saved;
                    self.out.recovery_symbols.insert(arm.start, ids);
                    self.expression(&arm.body)?;
                    self.scopes.pop();
                }
            }
            Expr::Match(body, arms) => {
                self.expression(body)?;
                for arm in arms {
                    let mut bindings = Vec::new();
                    self.collect_pattern(&arm.pattern, &mut bindings)?;
                    self.scopes.push(HashMap::new());
                    for (name, start) in bindings {
                        let id = self.fresh(name, SymbolKind::Parameter, start);
                        self.bind(id)?;
                        self.out.match_bindings.insert(start, id);
                    }
                    self.expression(&arm.body)?;
                    self.scopes.pop();
                }
            }
            Expr::Block(stmts, tail) | Expr::Action(stmts, tail) | Expr::Fallible(stmts, tail) => {
                self.block(stmts, tail.as_deref())?
            }
            Expr::Int(_)
            | Expr::Num(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Char(_)
            | Expr::Unit => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Usage {
    Plain,
    Ref,
    Call(usize),
    Record,
}
impl Usage {
    fn description(self) -> &'static str {
        match self {
            Self::Plain => "a value",
            Self::Ref => "a callable reference",
            Self::Call(_) => "a direct callable",
            Self::Record => "a record constructor",
        }
    }
}

// A local callable is in scope before its declaration, but local quantities
// are installed one at a time. Check that eagerly reachable code does not use
// an uninitialized slot. Suspended code is only followed at a known call or
// execute site. Opaque higher-order calls still have the evaluator's guard.
fn verify_eager_locals(p: &Program, r: &Resolution) -> Result<(), Error> {
    let mut index = EagerIndex {
        resolution: r,
        defs: HashMap::new(),
        blocks: Vec::new(),
    };
    index.gather_stmts(&p.defs);
    if let Some(e) = &p.entry {
        index.gather_expr(e);
    }
    for (stmts, tail) in &index.blocks {
        let mut pending = stmts
            .iter()
            .filter_map(|s| r.declaration(s))
            .filter(|id| r.symbol(*id).kind == SymbolKind::Value)
            .collect::<std::collections::HashSet<_>>();
        if pending.is_empty() {
            continue;
        }
        for stmt in *stmts {
            match stmt {
                Stmt::Def { params, body, .. } if params.is_empty() => {
                    index.check(body, &pending)?
                }
                Stmt::Effect { body, .. } => index.check(body, &pending)?,
                _ => {}
            };
            if let Some(id) = r.declaration(stmt) {
                pending.remove(&id);
            }
        }
        if let Some(tail) = tail {
            index.check(tail, &pending)?;
        }
    }
    Ok(())
}

struct EagerIndex<'a> {
    resolution: &'a Resolution,
    defs: HashMap<SymbolId, (&'a Expr, usize)>,
    blocks: Vec<(&'a [Stmt], Option<&'a Expr>)>,
}
impl<'a> EagerIndex<'a> {
    fn gather_stmts(&mut self, ss: &'a [Stmt]) {
        for s in ss {
            match s {
                Stmt::Def { params, body, .. } => {
                    if let Some(id) = self.resolution.declaration(s) {
                        self.defs.insert(id, (body, params.len()));
                    }
                    self.gather_expr(body);
                }
                Stmt::Effect { body, .. } => {
                    if let Some(id) = self.resolution.declaration(s) {
                        self.defs.insert(id, (body, 0));
                    }
                    self.gather_expr(body);
                }
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
            }
        }
    }
    fn gather_expr(&mut self, e: &'a Expr) {
        match e {
            Expr::Block(ss, tail) | Expr::Action(ss, tail) | Expr::Fallible(ss, tail) => {
                self.blocks.push((ss, tail.as_deref()));
                self.gather_stmts(ss);
                if let Some(e) = tail {
                    self.gather_expr(e);
                }
            }
            Expr::Call(_, es) | Expr::Tuple(es) | Expr::Array(es) | Expr::Pipeline(es) => {
                for x in es {
                    self.gather_expr(x);
                }
            }
            Expr::Raise(body) | Expr::Propagate(body) => self.gather_expr(body),
            Expr::Recover(body, arms) => {
                self.gather_expr(body);
                for arm in arms {
                    self.gather_expr(&arm.body);
                }
            }
            Expr::Match(body, arms) => {
                self.gather_expr(body);
                for arm in arms {
                    self.gather_expr(&arm.body);
                }
            }
            Expr::Cases(pairs, tail) => {
                for (c, v) in pairs {
                    self.gather_expr(c);
                    self.gather_expr(v);
                }
                self.gather_expr(tail);
            }
            Expr::Lambda(_, body) | Expr::TypedLambda(_, _, body) => self.gather_expr(body),
            Expr::Record(fs) | Expr::NamedRecord(_, fs) => {
                for (_, x) in fs {
                    self.gather_expr(x);
                }
            }
            _ => {}
        }
    }
    fn check(
        &self,
        e: &'a Expr,
        pending: &std::collections::HashSet<SymbolId>,
    ) -> Result<(), Error> {
        if !pending.is_empty() {
            let mut walk = EagerWalk {
                index: self,
                pending,
                active: std::collections::HashSet::new(),
                depth: 0,
                fuel: 20_000,
                masked: Vec::new(),
                pattern_values: HashMap::new(),
            };
            walk.expression(e)?;
        }
        Ok(())
    }
    fn known(&self, expr: &'a Expr, fuel: usize) -> Known<'a> {
        if fuel == 0 {
            return Known::Unknown;
        }
        match expr {
            Expr::Name(_) => {
                let id = self.resolution.reference(expr);
                if let Some(id) =
                    id.filter(|&id| self.resolution.symbol(id).kind == SymbolKind::Constructor(0))
                {
                    Known::Variant(id, &[])
                } else {
                    id.and_then(|id| self.defs.get(&id))
                        .and_then(|&(body, n)| (n == 0).then_some(body))
                        .map_or(Known::Unknown, |body| self.known(body, fuel - 1))
                }
            }
            Expr::Call(_, args)
                if self.resolution.reference(expr).is_some_and(|id| {
                    matches!(self.resolution.symbol(id).kind, SymbolKind::Constructor(_))
                }) =>
            {
                Known::Variant(self.resolution.reference(expr).unwrap(), args)
            }
            Expr::Ref(_) => self
                .resolution
                .reference(expr)
                .and_then(|id| self.defs.get(&id))
                .map_or(Known::Unknown, |&(body, n)| Known::Function(body, n)),
            Expr::Lambda(ps, body) | Expr::TypedLambda(ps, _, body) => {
                Known::Function(body, ps.len())
            }
            Expr::Action(ss, tail) => Known::Action(expr, ss, tail.as_deref()),
            Expr::Tuple(es) => Known::Tuple(es),
            Expr::Record(fs) | Expr::NamedRecord(_, fs) => Known::Record(fs),
            Expr::Call(n, args)
                if args.len() == 1
                    && n.starts_with('.')
                    && !matches!(n.as_str(), ".." | ".:")
                    && self.resolution.reference(expr).is_some_and(|id| {
                        self.resolution.symbol(id).kind == SymbolKind::External
                    }) =>
            {
                self.known(&args[0], fuel - 1)
                    .project(&n[1..])
                    .map_or(Known::Unknown, |e| self.known(e, fuel - 1))
            }
            Expr::Call(op, args)
                if matches!(op.as_str(), "?0" | "?1")
                    && self.resolution.reference(expr).is_some_and(|id| {
                        self.resolution.symbol(id).kind == SymbolKind::External
                    }) =>
            {
                Known::Optional(args.first())
            }
            Expr::Bool(b) => Known::Bool(*b),
            Expr::Unit => Known::Unit,
            _ => Known::Unknown,
        }
    }
}

#[derive(Clone, Copy)]
enum Known<'a> {
    Unknown,
    Bool(bool),
    Unit,
    Variant(SymbolId, &'a [Expr]),
    Tuple(&'a [Expr]),
    Record(&'a [(String, Expr)]),
    Optional(Option<&'a Expr>),
    Function(&'a Expr, usize),
    Action(&'a Expr, &'a [Stmt], Option<&'a Expr>),
}
impl<'a> Known<'a> {
    fn project(self, selector: &str) -> Option<&'a Expr> {
        match self {
            Self::Tuple(es) => es.get(selector.parse::<usize>().ok()?.checked_sub(1)?),
            Self::Record(fs) => fs.iter().find(|(n, _)| n == selector).map(|(_, e)| e),
            _ => None,
        }
    }
}
struct EagerWalk<'a, 's> {
    index: &'s EagerIndex<'a>,
    pending: &'s std::collections::HashSet<SymbolId>,
    masked: Vec<SymbolId>,
    pattern_values: HashMap<SymbolId, Known<'a>>,
    active: std::collections::HashSet<SyntaxId>,
    depth: usize,
    fuel: usize,
}
impl<'a> EagerWalk<'a, '_> {
    fn guarded(
        &mut self,
        owner: &'a Expr,
        f: impl FnOnce(&mut Self) -> Result<Known<'a>, Error>,
    ) -> Result<Known<'a>, Error> {
        let id = self
            .index
            .resolution
            .syntax_id(owner)
            .expect("indexed owner");
        if self.depth >= 128 || !self.active.insert(id) {
            return Ok(Known::Unknown);
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        self.active.remove(&id);
        result
    }
    fn apply(&mut self, f: Known<'a>, args: usize) -> Result<Known<'a>, Error> {
        if let Known::Function(body, n) = f {
            if args < n {
                return Ok(Known::Function(body, n - args));
            }
            if n != 0 {
                let result = self.guarded(body, |w| w.expression(body))?;
                if args > n {
                    return self.apply(result, args - n);
                }
                return Ok(result);
            }
        }
        Ok(Known::Unknown)
    }
    fn block(&mut self, ss: &'a [Stmt], tail: Option<&'a Expr>) -> Result<Known<'a>, Error> {
        // Recursion can instantiate the same lexical block again. Those
        // slots belong to a fresh frame; its own steps are checked separately.
        let mark = self.masked.len();
        self.masked.extend(
            ss.iter()
                .filter_map(|s| self.index.resolution.declaration(s)),
        );
        let result = (|| {
            for s in ss {
                match s {
                    Stmt::Def { params, body, .. } if params.is_empty() => {
                        self.expression(body)?;
                    }
                    Stmt::Effect { body, .. } => {
                        self.expression(body)?;
                    }
                    _ => {}
                }
            }
            tail.map_or(Ok(Known::Unknown), |e| self.expression(e))
        })();
        self.masked.truncate(mark);
        result
    }
    fn known_value(&self, e: &'a Expr, fuel: usize) -> Known<'a> {
        self.index
            .resolution
            .reference(e)
            .and_then(|id| self.pattern_values.get(&id).copied())
            .unwrap_or_else(|| self.index.known(e, fuel))
    }
    fn known_match(
        &self,
        pattern: &MatchPattern,
        value: Known<'a>,
        bindings: &mut Vec<(SymbolId, Known<'a>)>,
    ) -> Option<bool> {
        use MatchPatternKind::*;
        fn merge(values: impl Iterator<Item = Option<bool>>) -> Option<bool> {
            let mut unknown = false;
            for v in values {
                match v {
                    Some(false) => return Some(false),
                    None => unknown = true,
                    Some(true) => {}
                }
            }
            (!unknown).then_some(true)
        }
        match (&pattern.kind, value) {
            (Bind(_), _) => {
                if let Some(id) = self.index.resolution.match_binding(pattern) {
                    bindings.push((id, value));
                }
                Some(true)
            }
            (Wildcard, _) | (Unit, Known::Unit) => Some(true),
            (Bool(a), Known::Bool(b)) => Some(*a == b),
            (Optional(None), Known::Optional(payload)) => Some(payload.is_none()),
            (Optional(Some(_)), Known::Optional(None)) => Some(false),
            (Optional(Some(inner)), Known::Optional(Some(item))) => {
                self.known_match(inner, self.known_value(item, 32), bindings)
            }
            (Constructor(_, patterns), Known::Variant(id, items)) => {
                if self.index.resolution.match_constructor(pattern)? != id {
                    return Some(false);
                }
                if patterns.len() != items.len() {
                    return None;
                }
                merge(
                    patterns
                        .iter()
                        .zip(items)
                        .map(|(p, e)| self.known_match(p, self.known_value(e, 32), bindings)),
                )
            }
            (Tuple(patterns), Known::Tuple(items)) if patterns.len() == items.len() => merge(
                patterns
                    .iter()
                    .zip(items)
                    .map(|(p, e)| self.known_match(p, self.known_value(e, 32), bindings)),
            ),
            (Record(patterns), Known::Record(fields)) => {
                merge(patterns.iter().map(|(name, pattern)| {
                    let (_, item) = fields.iter().find(|(field, _)| field == name)?;
                    self.known_match(pattern, self.known_value(item, 32), bindings)
                }))
            }
            _ => None,
        }
    }
    fn match_body(
        &mut self,
        body: &'a Expr,
        bindings: Vec<(SymbolId, Known<'a>)>,
    ) -> Result<Known<'a>, Error> {
        // Nested re-entry can encounter the same syntactic arm. Restore any
        // outer abstract values on exit, rather than clearing them blindly.
        let previous: Vec<_> = bindings
            .into_iter()
            .map(|(id, value)| (id, self.pattern_values.insert(id, value)))
            .collect();
        let result = self.expression(body);
        for (id, prior) in previous.into_iter().rev() {
            if let Some(prior) = prior {
                self.pattern_values.insert(id, prior);
            } else {
                self.pattern_values.remove(&id);
            }
        }
        result
    }
    fn expression(&mut self, e: &'a Expr) -> Result<Known<'a>, Error> {
        // Recursion/highly shared call graphs cannot turn the checker into an
        // unbounded evaluator; unresolved paths retain the runtime guard.
        if self.fuel == 0 {
            return Ok(Known::Unknown);
        }
        self.fuel -= 1;
        let r = self.index.resolution;
        Ok(match e {
            Expr::Name(_) => {
                if let Some(id) = r
                    .reference(e)
                    .filter(|s| self.pending.contains(s) && !self.masked.contains(s))
                {
                    return Err(Error::new(
                        r.offset(e).unwrap_or(0),
                        format!(
                            "local `{}` is read before its initializer has finished",
                            r.symbol(id).name
                        ),
                    ));
                }
                self.known_value(e, 128)
            }
            Expr::Call(n, args) => {
                let id = r.reference(e);
                let external = id.is_some_and(|id| r.symbol(id).kind == SymbolKind::External);
                if external && n == "??" {
                    if let [fallback, optional] = args.as_slice() {
                        return match self.expression(optional)? {
                            Known::Optional(Some(_)) => Ok(Known::Unknown),
                            Known::Optional(None) => self.expression(fallback),
                            _ => {
                                self.expression(fallback)?;
                                Ok(Known::Unknown)
                            }
                        };
                    }
                }
                let values = args
                    .iter()
                    .map(|x| self.expression(x))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(id) =
                    id.filter(|&id| matches!(r.symbol(id).kind, SymbolKind::Constructor(_)))
                {
                    return Ok(Known::Variant(id, args));
                }
                match (external, n.as_str(), values.as_slice()) {
                    (true, selector, [value])
                        if selector.starts_with('.') && !matches!(selector, ".." | ".:") =>
                    {
                        value
                            .project(&selector[1..])
                            .map_or(Known::Unknown, |e| self.index.known(e, 128))
                    }
                    (true, "?0", []) => Known::Optional(None),
                    (true, "?1", [_]) => Known::Optional(args.first()),
                    (true, "@", [f, _]) => self.apply(*f, 1)?,
                    (true, "@@", [f, Known::Tuple(es)]) => self.apply(*f, es.len())?,
                    (true, "!", [Known::Action(owner, ss, tail)]) => {
                        self.guarded(owner, |w| w.block(ss, *tail))?
                    }
                    _ => {
                        if let Some(&(body, count)) = id.and_then(|id| self.index.defs.get(&id)) {
                            self.apply(Known::Function(body, count), args.len())?
                        } else {
                            Known::Unknown
                        }
                    }
                }
            }
            Expr::Cases(pairs, tail) => {
                let mut uncertain = false;
                for (cond, value) in pairs {
                    match self.expression(cond)? {
                        Known::Bool(false) => continue,
                        Known::Bool(true) => {
                            let selected = self.expression(value)?;
                            return Ok(if uncertain { Known::Unknown } else { selected });
                        }
                        _ => {
                            uncertain = true;
                            self.expression(value)?;
                        }
                    }
                }
                let fallback = self.expression(tail)?;
                if uncertain {
                    Known::Unknown
                } else {
                    fallback
                }
            }
            Expr::Raise(body) | Expr::Propagate(body) => {
                self.expression(body)?;
                Known::Unknown
            }
            Expr::Recover(body, arms) => {
                self.expression(body)?;
                for arm in arms {
                    self.expression(&arm.body)?;
                }
                Known::Unknown
            }
            Expr::Match(body, arms) => {
                let value = self.expression(body)?;
                let mut uncertain = false;
                for arm in arms {
                    let mut bindings = Vec::new();
                    match self.known_match(&arm.pattern, value, &mut bindings) {
                        Some(false) => continue,
                        Some(true) => {
                            let selected = self.match_body(&arm.body, bindings)?;
                            return Ok(if uncertain { Known::Unknown } else { selected });
                        }
                        None => {
                            uncertain = true;
                            self.match_body(&arm.body, bindings)?;
                        }
                    }
                }
                Known::Unknown
            }
            Expr::Block(ss, tail) | Expr::Fallible(ss, tail) => self.block(ss, tail.as_deref())?,
            Expr::Array(es) | Expr::Tuple(es) | Expr::Pipeline(es) => {
                for x in es {
                    self.expression(x)?;
                }
                self.index.known(e, 128)
            }
            Expr::Record(fs) | Expr::NamedRecord(_, fs) => {
                for (_, x) in fs {
                    self.expression(x)?;
                }
                self.index.known(e, 128)
            }
            _ => self.index.known(e, 128),
        })
    }
}

/// Iterative Kosaraju so a long dependency chain does not grow the Rust stack.
fn groups(edges: &[BTreeSet<usize>], p: &Program) -> Result<Vec<Vec<usize>>, Error> {
    let n = edges.len();
    let mut reverse = vec![Vec::new(); n];
    for (i, ds) in edges.iter().enumerate() {
        for &j in ds {
            reverse[j].push(i);
        }
    }
    let mut seen = vec![false; n];
    let mut postorder = Vec::with_capacity(n);
    for i in 0..n {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let mut stack = vec![(i, edges[i].iter())];
        while let Some((j, children)) = stack.last_mut() {
            if let Some(&k) = children.next() {
                if !seen[k] {
                    seen[k] = true;
                    stack.push((k, edges[k].iter()));
                }
            } else {
                postorder.push(*j);
                stack.pop();
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    for i in postorder.into_iter().rev() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let mut stack = vec![i];
        let mut component = Vec::new();
        while let Some(j) = stack.pop() {
            component.push(j);
            for &k in &reverse[j] {
                if !seen[k] {
                    seen[k] = true;
                    stack.push(k);
                }
            }
        }
        component.sort_unstable();
        if component.len() > 1 || edges[i].contains(&i) {
            if let Some(&v) = component
                .iter()
                .find(|&&j| !matches!(decl_name(&p.defs[j]), Some((_, SymbolKind::Callable(_), _))))
            {
                let names = component
                    .iter()
                    .filter_map(|&j| decl_name(&p.defs[j]).map(|(n, _, _)| n))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(Error::new(
                    stmt_offset(&p.defs[v]),
                    format!("cyclic value dependency involving {names}"),
                ));
            }
        }
        components.push(component);
    }
    components.reverse();
    Ok(components)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn orders_deps_and_allows_function_sccs() {
        let p = parse("a := f 0; f(x:n):n := ? (= x 0) b (g x); g(x:n):n := f x; b := 3;").unwrap();
        assert_eq!(
            resolve(&p).unwrap().groups,
            vec![vec![3], vec![1, 2], vec![0]]
        );
    }

    #[test]
    fn rejects_value_cycles_at_the_value_declaration() {
        let src = "f(x:n):n := a;\na := f 0;";
        let p = parse(src).unwrap();
        let e = resolve(&p).unwrap_err();
        assert_eq!(e.start, src.find("a :=").unwrap());
        assert!(e.message.contains("cyclic value dependency"));
        assert!(resolve(&parse("a := a;").unwrap()).is_err());
    }

    #[test]
    fn resolves_declaration_site_shadowing_and_stage_captures() {
        let p = parse("avg := 1; p := { f(x:n):z := + avg x; avg := 2; f 0 }; q := |{ 1; |{ $; + $ 1 }; + $ 1 };").unwrap();
        let r = resolve(&p).unwrap();
        let Stmt::Def {
            body: Expr::Block(stmts, _),
            ..
        } = &p.defs[1]
        else {
            panic!()
        };
        let Stmt::Def {
            body: Expr::Call(_, args),
            ..
        } = &stmts[0]
        else {
            panic!()
        };
        assert_eq!(r.reference(&args[0]), r.top[0]);
        assert_ne!(r.reference(&args[0]), r.declaration(&stmts[1]));
        let Stmt::Def {
            body: Expr::Pipeline(outer),
            ..
        } = &p.defs[2]
        else {
            panic!()
        };
        let Expr::Pipeline(inner) = &outer[1] else {
            panic!()
        };
        assert_eq!(r.reference(&inner[0]), r.placeholder(&outer[1]));
        let Expr::Call(_, ops) = &inner[1] else {
            panic!()
        };
        assert_eq!(r.reference(&ops[0]), r.placeholder(&inner[1]));
        assert_ne!(r.placeholder(&inner[1]), r.placeholder(&outer[1]));
    }

    #[test]
    fn stable_ids_and_alias_identity_on_fresh_parse() {
        let source = r#"<< "statistics" : st;
            f(x:[q]):?q := st.mean x;
            g(y:[q]):?q := avg y;
            fit := \(x:q) -> |{ x; + $ x };
        "#;
        let p = parse(source).unwrap();
        let q = parse(source).unwrap();
        let r = resolve(&p).unwrap();
        let s = resolve(&q).unwrap();
        assert_eq!(r.symbols, s.symbols);
        assert_eq!(r.groups, s.groups);
        let (Stmt::Def { body: a, .. }, Stmt::Def { body: b, .. }) = (&p.defs[0], &p.defs[1])
        else {
            panic!()
        };
        assert_eq!(r.reference(a), r.reference(b));
        assert_eq!(r.symbol(r.reference(a).unwrap()).name, "avg");
    }

    #[test]
    fn rejects_invalid_external_arity_on_synthetic_ast() {
        let mut p = parse("x := + 1 2;").unwrap();
        let Stmt::Def {
            body: Expr::Call(_, operands),
            ..
        } = &mut p.defs[0]
        else {
            panic!()
        };
        operands.pop();
        assert!(resolve(&p)
            .unwrap_err()
            .message
            .contains("expects 2 operands"));
    }

    #[test]
    fn expression_origins_follow_owned_ast_and_preserve_subranges() {
        let source = r#"
           t := #[ (1,2) (3,4) ];
           !{
               n := ?{ 1b : (+ 5 (- 8 7)); _ : 2; };
               |{ n; + $ 1 }
           };
        "#;
        let moved = Box::new(parse(source).unwrap().clone());
        let r = resolve(&moved).unwrap();
        let show = |e: &Expr| r.span(e).map(|range| &source[range]);
        let Stmt::Def {
            body: table @ Expr::Call(_, args),
            ..
        } = &moved.defs[0]
        else {
            panic!()
        };
        assert_eq!(show(table), Some("#[ (1,2) (3,4) ]"));
        assert_eq!(show(&args[0]), None);
        let Expr::Array(rows) = &args[0] else {
            panic!()
        };
        assert_eq!(show(&rows[0]), Some("(1,2)"));
        let entry @ Expr::Action(stmts, Some(tail)) = moved.entry.as_ref().unwrap() else {
            panic!()
        };
        assert!(show(entry).unwrap().starts_with("!{"));
        let Stmt::Def {
            body: Expr::Cases(pairs, _),
            ..
        } = &stmts[0]
        else {
            panic!()
        };
        let (guard, value) = &pairs[0];
        assert_eq!(show(guard), Some("1b"));
        assert_eq!(show(value), Some("(+ 5 (- 8 7))"));
        let Expr::Call(_, ops) = value else { panic!() };
        assert_eq!(show(&ops[0]), Some("5"));
        assert_eq!(show(&ops[1]), Some("(- 8 7)"));
        assert_eq!(show(tail), Some("|{ n; + $ 1 }"));
    }

    #[test]
    fn changed_ast_does_not_inherit_unverified_expression_ranges() {
        let mut p = parse("q := + 1 2;").unwrap();
        let Stmt::Def {
            body: Expr::Call(_, operands),
            ..
        } = &mut p.defs[0]
        else {
            panic!()
        };
        operands[1] = Expr::Int(9);
        let r = resolve(&p).unwrap();
        let Stmt::Def {
            body: call @ Expr::Call(_, operands),
            ..
        } = &p.defs[0]
        else {
            panic!()
        };
        assert_eq!(r.span(call), None);
        assert_eq!(r.span(&operands[1]), None);
        assert_eq!(r.offset(&operands[1]), Some(0));
    }
    #[test]
    fn rejects_known_eager_local_reads_before_ready() {
        for (source, binding) in [
            ("!{ a := f (); f(x:()):z := a; (); };", "a"),
            (
                "!{ a := f (); x := 8; f(u:()):z := g u; g(u:()):z := x; (); };",
                "x",
            ),
            ("!{ rf := &f; a := @ rf (); f(x:()):z := a; (); };", "a"),
            (
                "!{ boxed := (&f,()); a := @ (.1 boxed) (); f(x:()):z := a; () };",
                "a",
            ),
            (
                "!{ r := .{ fn := &f; }; a := @ (.fn r) (); f(x:()):z := a; () };",
                "a",
            ),
            ("!{ a := ! !{ f () }; f(x:()):z := a; (); };", "a"),
        ] {
            let p = parse(source).unwrap();
            let error = resolve(&p).unwrap_err();
            assert!(
                error.message.contains(&format!("local `{binding}`")),
                "{source}: {error:?}"
            );
        }
    }

    #[test]
    fn accepts_latent_reads_ready_calls_and_fresh_recursive_frames() {
        for source in [
            "!{ hold := &f; x := 8; f(u:()):z := x; a := @ hold (); (); };",
            r"!{ hold := \u -> f u; x := 8; f(u:()):z := x; a := @ hold (); (); };",
            "!{ a := ? 0b (f ()) 3; f(u:()):z := a; (); };",
            "!{ a := ?? (f ()) (?1 3); f(u:()):z := a; (); };",
            "!{ f(n:z):z := ? (= n 0) 0 (g (- n 1)); g(n:z):z := f n; a := f 2; (); };",
            "f(n:z):z := { y := ? (= n 0) 1 (f (- n 1)); y }; a := f 2;",
        ] {
            let p = parse(source).unwrap();
            assert!(resolve(&p).is_ok(), "{source}: {:?}", resolve(&p));
        }
    }

    #[test]
    fn sums_hoist_constructor_arity_and_recover_lexical_binders() {
        let src = "use(u:()):q^err := ^{ ^: bad 7 \"oops\" }; err :: { | empty; | bad n s; }; result := ^= (use ()) { bad x _ : x; empty : 0; };";
        let p = parse(src).unwrap();
        let r = resolve(&p).unwrap();
        assert_eq!(r.groups[0], vec![1]);
        let Stmt::Sum { variants, .. } = &p.defs[1] else {
            panic!()
        };
        let ctor = r.variant(&variants[1]).unwrap();
        assert_eq!(r.symbol(ctor).kind, SymbolKind::Constructor(2));
        assert_eq!(r.constructor_owner(ctor), r.declaration(&p.defs[1]));
        let Stmt::Def {
            body: Expr::Recover(_, arms),
            ..
        } = &p.defs[2]
        else {
            panic!()
        };
        assert_eq!(r.recovery_constructor(&arms[0]), Some(ctor));
        assert_eq!(
            r.recovery_bindings(&arms[0]),
            &[r.reference(&arms[0].body).unwrap()]
        );
        assert_eq!(r.span(&arms[0].body).map(|range| &src[range]), Some("x"));
        assert_eq!(r.recovery_constructor(&arms[1]), r.variant(&variants[0]));
        assert!(parse("e :: { | b n; }; x := b;").is_err());
        assert!(parse("e :: { | b n n; }; x := ^= ^{0} { b y y : 1; };").is_err());
    }

    #[test]
    fn standard_variants_require_import_and_are_shadowable() {
        let alias = parse(r#"<< "io" : fs; e := fileerr (missing "path");"#).unwrap();
        let alias_res = resolve(&alias).unwrap();
        let Stmt::Def { body, .. } = &alias.defs[0] else {
            panic!()
        };
        assert_eq!(
            alias_res.reference(body),
            alias_res.standard_constructor("csverr", "fileerr")
        );
        assert!(parse("x := missing \"x\";").is_ok());
        assert!(parse("x := invalidjson \"x\";").is_err());
        let p = parse("<< csv; x := missing \"x\"; y := { missing := 2; missing };").unwrap();
        let r = resolve(&p).unwrap();
        let Stmt::Def { body, .. } = &p.defs[0] else {
            panic!()
        };
        let id = r.reference(body);
        assert_eq!(id, r.standard_constructor("ioerr", "missing"));
        assert_eq!(r.constructor_owner(id.unwrap()), r.standard_owner("ioerr"));
        let Stmt::Def {
            body: Expr::Block(ss, Some(tail)),
            ..
        } = &p.defs[1]
        else {
            panic!()
        };
        assert_eq!(r.reference(tail), r.declaration(&ss[0]));
    }

    #[test]
    fn nested_match_binders_have_arm_local_identity() {
        let p = parse("tree[~a] :: { | leaf ~a; | node tree[~a] tree[~a]; }; f(t:tree[q]):q := ?= t { leaf x : x; node (leaf x) _ : x; _ : 0q; };").unwrap();
        let res = resolve(&p).unwrap();
        let Stmt::Def {
            body: Expr::Match(_, arms),
            ..
        } = &p.defs[1]
        else {
            panic!("expected match");
        };
        let MatchPatternKind::Constructor(_, first) = &arms[0].pattern.kind else {
            panic!("expected constructor");
        };
        let MatchPatternKind::Constructor(_, second) = &arms[1].pattern.kind else {
            panic!("expected constructor");
        };
        let MatchPatternKind::Constructor(_, nested) = &second[0].kind else {
            panic!("expected nested constructor");
        };
        assert!(res.match_constructor(&arms[0].pattern).is_some());
        assert_eq!(
            res.match_constructor(&arms[0].pattern),
            res.match_constructor(&second[0])
        );
        let l = res.match_binding(&first[0]).unwrap();
        let r = res.match_binding(&nested[0]).unwrap();
        assert_ne!(l, r);
        assert_eq!(res.reference(&arms[0].body), Some(l));
        assert_eq!(res.reference(&arms[1].body), Some(r));
    }

    #[test]
    fn eager_resolution_follows_only_the_statically_selected_match_arm() {
        let source = "color :: { | red; | blue; }; !{ safe := ?= COLOR { red : 1z; blue : read (); }; delayed := 7z; read(x:()):z := delayed; () };";
        let good = parse(&source.replace("COLOR", "red")).unwrap();
        resolve(&good).unwrap();
        let bad = parse(&source.replace("COLOR", "blue")).unwrap();
        assert!(resolve(&bad)
            .unwrap_err()
            .message
            .contains("before its initializer"));
    }

    #[test]
    fn checks_eager_reads_through_known_match_payload_bindings() {
        for (header, value, pattern) in [
            ("", "(&f,())", "(g,())"),
            ("box :: { | wrap (()->z); };", "wrap &f", "wrap g"),
            ("", "?1 &f", "?1 g"),
            ("", ".{ get := &f; }", ".{ get := g; }"),
        ] {
            let fallback = if pattern == "?1 g" { "_ : 0z;" } else { "" };
            let eager = format!("{header} !{{ a := ?= {value} {{ {pattern} : @ g (); {fallback} }}; f(x:()):z := a; () }};");
            let p = parse(&eager).unwrap();
            assert!(
                resolve(&p)
                    .unwrap_err()
                    .message
                    .contains("before its initializer"),
                "{eager}"
            );

            let deferred = format!(
                r"{header} !{{ a := ?= {value} {{ {pattern} : \x -> @ g x; }}; f(x:()):z := @ a x; () }};"
            );
            resolve(&parse(&deferred).unwrap()).unwrap();
        }
    }
}
