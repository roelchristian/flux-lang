//! Lexical binding, expression resolution and dependency discovery.

use super::{
    decl_name, dependencies::groups, eager::verify_eager_locals, ProgramShape, Resolution, Symbol,
    SymbolId, SymbolKind, SyntaxLocator,
};
use crate::{
    builtins,
    parser::{
        standard_owners, Expr, MatchPattern, MatchPatternKind, Program, RecoveryPattern, Stmt,
        Variant,
    },
    standard_errors, Error,
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

pub(super) fn resolve_impl(p: &Program, retain_shape: bool) -> Result<Resolution, Error> {
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
    for &owner in standard_errors::OWNERS {
        if owner != "ioerr"
            && !p
                .imports
                .iter()
                .any(|m| standard_owners(m).contains(&owner))
        {
            continue;
        }
        let owner_id =
            SymbolId(standard_errors::owner_id(owner).expect("known standard error type"));
        for v in standard_errors::variants(owner) {
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
        self.out.defs.insert(stmt.start(), id);
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
        self.offset = s.start();
        match s {
            Stmt::Schema { .. } | Stmt::Sum { .. } => {}
            Stmt::Def { params, body, .. } if !params.is_empty() => {
                let ids = self.parameter_scope(params)?;
                self.out.params.insert(s.start(), ids);
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
                e.try_for_each_child(|a| self.expression(a))?;
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
            Expr::NamedRecord(n, _) => {
                self.use_name(e, n, Usage::Record)?;
                e.try_for_each_child(|a| self.expression(a))?;
            }
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
            Expr::Block(stmts, tail)
            | Expr::Action(stmts, tail)
            | Expr::Fallible(stmts, tail)
            | Expr::FallibleActionBody(stmts, tail) => self.block(stmts, tail.as_deref())?,
            Expr::Int(_)
            | Expr::Num(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Char(_)
            | Expr::Unit
            | Expr::Tuple(_)
            | Expr::Array(_)
            | Expr::Cases(_, _)
            | Expr::Record(_)
            | Expr::Raise(_)
            | Expr::Propagate(_)
            | Expr::AutoPropagate(_) => e.try_for_each_child(|child| self.expression(child))?,
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
